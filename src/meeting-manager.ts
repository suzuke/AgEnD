import { existsSync, mkdirSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { homedir } from "node:os";
import type { FleetContext } from "./fleet-context.js";
import type { InstanceConfig } from "./types.js";
import type { EphemeralInstanceConfig } from "./meeting/types.js";
import type { InboundMessage } from "./channel/types.js";

export class MeetingManager {
  private ephemeralTopicMap: Map<string, number> = new Map();

  constructor(private ctx: FleetContext) {}

  /** Get the ephemeral topic ID for an instance (used by fleet-manager for outbound routing) */
  getEphemeralTopicId(instanceName: string): number | undefined {
    return this.ephemeralTopicMap.get(instanceName);
  }

  /** Check if a command is a meeting command, and handle it if so */
  async handleCommand(msg: InboundMessage): Promise<boolean> {
    const text = msg.text?.trim();
    if (!text) return false;

    if (text === "/meets" || text === "/meets@" || text.startsWith("/meets ") || text.startsWith("/meets@")) {
      await this.handleMeetsCommand(msg, "discussion");
      return true;
    }
    if (text === "/debate" || text === "/debate@" || text.startsWith("/debate ") || text.startsWith("/debate@")) {
      await this.handleMeetsCommand(msg, "debate");
      return true;
    }
    if (text === "/collab" || text === "/collab@" || text.startsWith("/collab ") || text.startsWith("/collab@")) {
      await this.handleMeetsCommand(msg, "collab");
      return true;
    }

    return false;
  }

  private parseMeetsArgs(text: string): { topic: string; mode: "debate" | "collab" | "discussion"; count: number; rounds?: number; names?: string[]; repo?: string; angles?: string[] } | null {
    const args = text.replace(/^\/(meets|collab|debate)(@\S+)?\s*/, "").trim();
    if (!args) return null;

    let mode: "debate" | "collab" | "discussion" = "discussion";
    let count = 2;
    let rounds: number | undefined;
    let names: string[] | undefined;
    let repo: string | undefined;
    let angles: string[] | undefined;
    let topic = args;

    const repoMatch = topic.match(/--repo\s+(\S+)/);
    if (repoMatch) {
      repo = repoMatch[1].startsWith("~")
        ? join(homedir(), repoMatch[1].slice(1))
        : resolve(repoMatch[1]);
      topic = topic.replace(repoMatch[0], "").trim();
    }

    const rMatch = topic.match(/-r\s+(\d+)/);
    if (rMatch) {
      rounds = parseInt(rMatch[1], 10);
      topic = topic.replace(rMatch[0], "").trim();
    }

    const nMatch = topic.match(/-n\s+(\d+)/);
    if (nMatch) {
      count = parseInt(nMatch[1], 10);
      topic = topic.replace(nMatch[0], "").trim();
    }

    const namesMatch = topic.match(/--names\s+"([^"]+)"/);
    if (namesMatch) {
      names = namesMatch[1].split(",").map(n => n.trim());
      topic = topic.replace(namesMatch[0], "").trim();
    }

    const anglesMatch = topic.match(/--angles\s+"([^"]+)"/);
    if (anglesMatch) {
      angles = anglesMatch[1].split(",").map(a => a.trim());
      topic = topic.replace(anglesMatch[0], "").trim();
    }

    if (topic.includes("--debate")) {
      mode = "debate";
      topic = topic.replace("--debate", "").trim();
    }

    topic = topic.replace(/^["']|["']$/g, "").trim();
    if (!topic) return null;

    return { topic, mode, count, rounds, names, repo, angles };
  }

  private async handleMeetsCommand(msg: InboundMessage, forceMode?: "debate" | "collab" | "discussion"): Promise<void> {
    if (!this.ctx.adapter) return;

    const parsed = this.parseMeetsArgs(msg.text);
    if (!parsed) {
      const usage = forceMode === "collab"
        ? '用法：/collab --repo ~/app "任務"\n例如：/collab -n 3 --repo ~/app "實作 OAuth"'
        : '用法：/meets "議題"\n例如：/meets -n 3 "要不要拆 monorepo？"';
      await this.ctx.adapter.sendText(msg.chatId, usage);
      return;
    }

    if (forceMode) parsed.mode = forceMode;

    if (parsed.mode === "collab" && !parsed.repo) {
      await this.ctx.adapter.sendText(msg.chatId, '⚠️ 協作模式需要指定 repo：/collab --repo ~/app "任務"');
      return;
    }

    if (parsed.repo && !existsSync(join(parsed.repo, ".git"))) {
      const { execFileSync } = await import("child_process");
      mkdirSync(parsed.repo, { recursive: true });
      execFileSync("git", ["init"], { cwd: parsed.repo, stdio: "pipe" });
      execFileSync("git", ["commit", "--allow-empty", "-m", "init"], { cwd: parsed.repo, stdio: "pipe" });
      this.ctx.logger.info({ repo: parsed.repo }, "Auto-initialized git repo for collab");
    }

    const maxParticipants = this.ctx.fleetConfig?.defaults?.meetings?.maxParticipants ?? 6;
    if (parsed.count > maxParticipants) {
      await this.ctx.adapter.sendText(msg.chatId, `⚠️ 超過參與者上限 (${maxParticipants})`);
      return;
    }

    if (parsed.count < 2) {
      await this.ctx.adapter.sendText(msg.chatId, "⚠️ 至少需要 2 位參與者");
      return;
    }

    await this.startMeeting(msg.chatId, parsed.topic, parsed.mode, parsed.count, parsed.names, parsed.repo, parsed.rounds, parsed.angles);
  }

  private async startMeeting(
    chatId: string,
    topic: string,
    mode: "debate" | "collab" | "discussion",
    count: number,
    customNames?: string[],
    repo?: string,
    rounds?: number,
    angles?: string[],
  ): Promise<void> {
    let channelId: number;
    try {
      const topicLabel = topic.length > 30 ? topic.slice(0, 30) + "…" : topic;
      channelId = (await this.createMeetingChannel(`📋 ${topicLabel}`)).channelId;
    } catch (err) {
      await this.ctx.adapter!.sendText(chatId, `⚠️ 無法建立會議 topic: ${(err as Error).message}`);
      return;
    }

    const systemPrompt = `你是一個 AI 會議主持人。你的工作是協調多角度討論並用 reply 工具將每個角色的回應分批發送到 channel。每個角色的回應要分開發送，不要合併成一則訊息。`;

    let kickoffMessage: string;
    if (mode === "debate") {
      kickoffMessage = `使用 Agent Team 來辯論這個議題：「${topic}」\n\n要求：\n- ${count} 位參與者（正方、反方${count > 2 ? "、仲裁" : ""}）\n- 進行 ${rounds ?? 3} 輪辯論\n- 每個角色的發言要分開用 reply 發送，標明角色`;
    } else if (mode === "collab") {
      kickoffMessage = `使用 Agent Team 來協作完成這個任務：「${topic}」\n${repo ? `\n專案目錄：${repo}\n` : ""}\n要求：\n- ${count} 位參與者\n- 每個人用 isolation: "worktree" 在獨立 git worktree 工作\n- 先討論分工，再各自開發\n- 每個階段進展都分開用 reply 發送`;
    } else {
      if (!angles) {
        const defaultAngles = ["技術面", "成本效益", "使用者體驗", "風險與挑戰", "組織影響", "長期策略"];
        angles = defaultAngles.slice(0, count);
      }
      const angleList = angles.join("、");
      kickoffMessage = `使用 Agent Team 來從多個角度討論這個議題：「${topic}」\n\n分析角度：${angleList}\n\n要求：\n- 每個角度派一個 Agent 獨立分析\n- 分析完成後進行 ${rounds ?? 2} 輪交叉討論\n- 最後收斂出共識結論\n- 每個角色的回應要分開用 reply 發送，標明角度`;
    }

    const instanceName = await this.spawnEphemeralInstance({
      systemPrompt,
      workingDirectory: repo ?? "/tmp",
      lightweight: true,
      skipPermissions: true,
    });

    this.ctx.routingTable.set(channelId, { kind: "instance", name: instanceName });
    this.ephemeralTopicMap.set(instanceName, channelId);

    await this.ctx.adapter!.sendText(chatId, `✅ 會議已建立，請到新的 topic 查看`);

    const ipc = this.ctx.instanceIpcClients.get(instanceName);
    if (ipc) {
      ipc.send({
        type: "fleet_inbound",
        content: kickoffMessage,
        meta: {
          chat_id: String(this.ctx.fleetConfig?.channel?.group_id ?? ""),
          message_id: `meet-${Date.now()}`,
          user: "system",
          user_id: "system",
          ts: new Date().toISOString(),
          thread_id: String(channelId),
        },
      });
    }
  }

  async spawnEphemeralInstance(config: EphemeralInstanceConfig, signal?: AbortSignal): Promise<string> {
    const name = `meet-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;

    if (signal?.aborted) throw Object.assign(new Error("Aborted"), { name: "AbortError" });

    let workDir = config.workingDirectory;
    if (workDir !== "/tmp") {
      if (!existsSync(join(workDir, ".git"))) {
        throw new Error(`Not a git repository: ${workDir}`);
      }
      const { execFileSync } = await import("child_process");
      try {
        execFileSync("git", ["rev-parse", "HEAD"], { cwd: workDir, stdio: "pipe" });
      } catch {
        execFileSync("git", ["commit", "--allow-empty", "-m", "init"], { cwd: workDir, stdio: "pipe" });
      }
      const worktreePath = join("/tmp", `ccd-collab-${name}`);
      const branchName = `meet/${name}`;
      execFileSync("git", ["worktree", "add", worktreePath, "-b", branchName], { cwd: workDir, stdio: "pipe" });
      workDir = worktreePath;
      this.ctx.logger.info({ name, worktreePath, branchName }, "Created git worktree for collab instance");
    }

    const instanceConfig: InstanceConfig = {
      working_directory: workDir,
      lightweight: true,
      systemPrompt: config.systemPrompt,
      skipPermissions: config.skipPermissions,
      restart_policy: { max_retries: 0, backoff: "linear", reset_after: 0 },
      context_guardian: { threshold_percentage: 100, max_idle_wait_ms: 0, completion_timeout_ms: 0, grace_period_ms: 0, max_age_hours: 24 },
      memory: { auto_summarize: false, watch_memory_dir: false, backup_to_sqlite: false },
      log_level: "info",
      backend: config.backend,
    };

    await this.ctx.startInstance(name, instanceConfig, true);

    const deadline = Date.now() + 60_000;
    const sockPath = join(this.ctx.getInstanceDir(name), "channel.sock");
    while (!existsSync(sockPath)) {
      if (Date.now() > deadline) throw new Error(`IPC timeout for ${name}`);
      if (signal?.aborted) throw Object.assign(new Error("Aborted"), { name: "AbortError" });
      await new Promise(r => setTimeout(r, 500));
    }
    await this.ctx.connectIpcToInstance(name);

    const ipc = this.ctx.instanceIpcClients.get(name);
    if (ipc) {
      const mcpDeadline = Date.now() + 60_000;
      await new Promise<void>((resolve, reject) => {
        const onMessage = (msg: Record<string, unknown>) => {
          if (msg.type === "mcp_ready") { resolve(); }
        };
        ipc.on("message", onMessage);
        const check = () => {
          if (Date.now() > mcpDeadline) {
            ipc.removeListener("message", onMessage);
            reject(new Error(`MCP ready timeout for ${name}`));
          } else {
            setTimeout(check, 500);
          }
        };
        check();
      });
    }

    return name;
  }

  async destroyEphemeralInstance(name: string): Promise<void> {
    await this.ctx.stopInstance(name);

    const worktreePath = join("/tmp", `ccd-collab-${name}`);
    if (existsSync(worktreePath)) {
      try {
        const { execFileSync } = await import("child_process");
        const mainRepo = execFileSync("git", ["rev-parse", "--git-common-dir"], { cwd: worktreePath, stdio: "pipe" }).toString().trim();
        const mainRepoDir = dirname(mainRepo);
        execFileSync("git", ["worktree", "remove", "--force", worktreePath], { cwd: mainRepoDir, stdio: "pipe" });
        try {
          execFileSync("git", ["branch", "-D", `meet/${name}`], { cwd: mainRepoDir, stdio: "pipe" });
        } catch { /* branch may not exist */ }
        this.ctx.logger.info({ name }, "Cleaned up git worktree");
      } catch (err) {
        this.ctx.logger.warn({ name, err }, "Failed to clean up worktree");
      }
    }
  }

  async createMeetingChannel(title: string): Promise<{ channelId: number }> {
    const threadId = await this.ctx.createForumTopic(title);
    return { channelId: threadId };
  }

  async closeMeetingChannel(channelId: number): Promise<void> {
    const groupId = this.ctx.fleetConfig?.channel?.group_id;
    const botTokenEnv = this.ctx.fleetConfig?.channel?.bot_token_env;
    if (!groupId || !botTokenEnv) return;
    const botToken = process.env[botTokenEnv];
    if (!botToken) return;

    await fetch(
      `https://api.telegram.org/bot${botToken}/closeForumTopic`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ chat_id: groupId, message_thread_id: channelId }),
      },
    );
  }
}
