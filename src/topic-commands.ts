import { readFileSync, existsSync } from "node:fs";
import { exec } from "node:child_process";
import { promisify } from "node:util";
import { join, basename, dirname } from "node:path";
import { homedir } from "node:os";
import { fileURLToPath } from "node:url";
import { randomBytes } from "node:crypto";

const execAsync = promisify(exec);
import type { FleetContext } from "./fleet-context.js";
import type { InboundMessage } from "./channel/types.js";
import { DEFAULT_INSTANCE_CONFIG } from "./config.js";
import { formatCents } from "./cost-guard.js";
import { detectPlatform } from "./service-installer.js";

/** Sanitize a directory name into a valid instance name. Keeps Unicode letters (incl. CJK). */
export function sanitizeInstanceName(name: string): string {
  const sanitized = name.toLowerCase().replace(/[^\p{L}\d-]/gu, "-").replace(/-+/g, "-").replace(/^-|-$/g, "");
  return sanitized || "project";
}

const SEMVER_RE = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;
const UPDATE_CONFIRM_TTL_MS = 60_000;

interface PendingUpdate {
  /** Resolved version to install (e.g. "1.22.3"); null means "@latest". */
  version: string | null;
  /** Pre-install version captured from the running install's package.json. */
  previousVersion: string;
  /** One-time confirmation token. */
  token: string;
  /** User who initiated; only this user can confirm. */
  userId: string | null;
  /** Wall-clock ms after which the pending request is rejected. */
  expiresAt: number;
}

export class TopicCommands {
  /** Test seam: replaceable child_process runner. */
  protected exec: (cmd: string, opts: { timeout: number }) => Promise<unknown> = execAsync;

  /** Pending /update awaiting `/update confirm <token>`. */
  private pendingUpdate: PendingUpdate | null = null;

  constructor(private ctx: FleetContext) {}

  /** Parse and dispatch commands from the General topic */
  async handleGeneralCommand(msg: InboundMessage): Promise<boolean> {
    const text = msg.text?.trim();
    if (!text) return false;

    if (text === "/status" || text === "/status@" || text.startsWith("/status@")) {
      await this.handleStatusCommand(msg);
      return true;
    }

    if (text === "/restart" || text === "/restart@" || text.startsWith("/restart@")) {
      await this.handleRestartCommand(msg);
      return true;
    }

    if (text === "/sysinfo" || text === "/sysinfo@" || text.startsWith("/sysinfo@")
        || text === "/sys-info" || text === "/sys_info") {
      await this.handleSysInfoCommand(msg);
      return true;
    }

    if (text === "/update" || text === "/update@" || text.startsWith("/update@")
        || text.startsWith("/update ")) {
      await this.handleUpdateCommand(msg);
      return true;
    }

    return false;
  }

  private async handleRestartCommand(msg: InboundMessage): Promise<void> {
    if (!this.ctx.adapter) return;
    const chatId = msg.chatId;
    const threadId = msg.threadId;
    await this.ctx.adapter.sendText(chatId, "🔄 Graceful restart — waiting for instances to idle...", { threadId });
    // SIGUSR2 triggers in-process restart (safe without service manager)
    process.kill(process.pid, "SIGUSR2");
  }

  private async handleStatusCommand(msg: InboundMessage): Promise<void> {
    if (!this.ctx.adapter || !this.ctx.fleetConfig) return;

    const lines: string[] = [];
    for (const [name] of Object.entries(this.ctx.fleetConfig.instances)) {
      const status = this.ctx.getInstanceStatus(name);
      const paused = this.ctx.costGuard?.isLimited(name);

      let contextStr = "-";
      try {
        const data = JSON.parse(readFileSync(join(this.ctx.dataDir, "instances", name, "statusline.json"), "utf-8"));
        if (data.context_window?.used_percentage != null) {
          contextStr = `${Math.round(data.context_window.used_percentage)}%`;
        }
      } catch { /* file may not exist yet */ }

      const costCents = this.ctx.costGuard?.getDailyCostCents(name) ?? 0;

      let icon: string;
      if (paused) icon = "⏸";
      else if (status === "running") icon = "🟢";
      else if (status === "crashed") icon = "🔴";
      else icon = "⚪";

      lines.push(`${icon} ${name} — ctx ${contextStr}, ${formatCents(costCents)} today`);
    }

    if (lines.length === 0) {
      lines.push("No instances configured.");
    }

    const limitCents = this.ctx.costGuard?.getLimitCents() ?? 0;
    const totalCents = this.ctx.costGuard?.getFleetTotalCents() ?? 0;
    if (limitCents > 0) {
      lines.push("");
      lines.push(`Fleet: ${formatCents(totalCents)} / ${formatCents(limitCents)} daily`);
    }

    await this.ctx.adapter.sendText(msg.chatId, lines.join("\n"));
  }

  private async handleSysInfoCommand(msg: InboundMessage): Promise<void> {
    if (!this.ctx.adapter) return;
    const info = this.ctx.getSysInfo();

    const upHours = Math.floor(info.uptime_seconds / 3600);
    const upMins = Math.floor((info.uptime_seconds % 3600) / 60);
    const lines: string[] = [
      `⚙️ System Info`,
      `Uptime: ${upHours}h ${upMins}m`,
      `Memory: ${info.memory_mb.rss} MB RSS, ${info.memory_mb.heapUsed}/${info.memory_mb.heapTotal} MB heap`,
      "",
      "Instances:",
    ];

    for (const inst of info.instances) {
      const icon = inst.status === "running" ? "🟢" : inst.status === "crashed" ? "🔴" : "⚪";
      const ipc = inst.ipc ? "✓" : "✗";
      let detail = `${icon} ${inst.name} [IPC:${ipc}] ${formatCents(inst.costCents)}`;
      if (inst.rateLimits) {
        detail += ` (5h:${inst.rateLimits.five_hour_pct}% 7d:${inst.rateLimits.seven_day_pct}%)`;
      }
      lines.push(detail);
    }

    if (info.fleet_cost_limit_cents > 0) {
      lines.push("");
      lines.push(`Fleet cost: ${formatCents(info.fleet_cost_cents)} / ${formatCents(info.fleet_cost_limit_cents)} daily`);
    }

    await this.ctx.adapter.sendText(msg.chatId, lines.join("\n"), { threadId: msg.threadId });
  }

  /** Read the version string from the running install's package.json. */
  protected readCurrentVersion(): string {
    try {
      const here = dirname(fileURLToPath(import.meta.url));
      const pkg = JSON.parse(readFileSync(join(here, "..", "package.json"), "utf-8"));
      return pkg.version ?? "unknown";
    } catch {
      return "unknown";
    }
  }

  /**
   * /update                       → preview latest, request confirmation
   * /update <semver>              → preview that version, request confirmation
   * /update confirm <token>       → execute (originator only, within 60s)
   * /update cancel                → clear pending request
   *
   * Authorization: requires a non-empty `channel.access.allowed_users` list AND
   * the requester to be on it. An empty allow-list means no one can /update —
   * we will not accept the dangerous default of "open to anyone".
   */
  private async handleUpdateCommand(msg: InboundMessage): Promise<void> {
    if (!this.ctx.adapter) return;
    const chatId = msg.chatId;
    const threadId = msg.threadId;
    const text = (msg.text ?? "").trim();

    const allowed = this.ctx.fleetConfig?.channel?.access?.allowed_users ?? [];
    if (allowed.length === 0) {
      await this.ctx.adapter.sendText(chatId,
        "⛔ /update is disabled — channel.access.allowed_users is empty. Add at least one user ID to fleet.yaml to enable updates.",
        { threadId });
      return;
    }
    if (!allowed.some(u => String(u) === String(msg.userId))) {
      await this.ctx.adapter.sendText(chatId, "⛔ Not authorized", { threadId });
      return;
    }

    // Parse the args after the command word.
    const parts = text.split(/\s+/);
    // parts[0] is "/update" or "/update@bot"; the rest are args.
    const args = parts.slice(1);

    if (args[0] === "cancel") {
      this.pendingUpdate = null;
      await this.ctx.adapter.sendText(chatId, "🗑️ Pending /update cancelled.", { threadId });
      return;
    }

    if (args[0] === "confirm") {
      await this.confirmAndApplyUpdate(msg, args[1]);
      return;
    }

    // Stage 1: register a pending update + show confirmation prompt.
    let targetVersion: string | null = null;
    if (args[0]) {
      if (!SEMVER_RE.test(args[0])) {
        await this.ctx.adapter.sendText(chatId,
          `⛔ Invalid version "${args[0]}". Expected semver (e.g. 1.22.3).`,
          { threadId });
        return;
      }
      targetVersion = args[0];
    }

    // If a different user already has a live pending request, tell them that
    // their token was just superseded so they don't sit confused waiting for
    // a token that will silently fail.
    const prior = this.pendingUpdate;
    if (prior && Date.now() <= prior.expiresAt
        && prior.userId && String(prior.userId) !== String(msg.userId)) {
      await this.ctx.adapter.sendText(chatId,
        `ℹ️ Note: a previous pending /update from user ${prior.userId} was superseded by this request.`,
        { threadId });
    }

    const previous = this.readCurrentVersion();
    const token = randomBytes(4).toString("hex"); // 8 hex chars (32 bits)
    this.pendingUpdate = {
      version: targetVersion,
      previousVersion: previous,
      token,
      userId: msg.userId ?? null,
      expiresAt: Date.now() + UPDATE_CONFIRM_TTL_MS,
    };

    const targetLabel = targetVersion ? `@${targetVersion}` : "@latest";
    await this.ctx.adapter.sendText(chatId,
      `⚠️ Pending update\n` +
      `Current: ${previous}\n` +
      `Target:  ${targetLabel}\n\n` +
      `Reply within 60s:\n` +
      `  /update confirm ${token}\n` +
      `Or cancel:\n` +
      `  /update cancel`,
      { threadId });
  }

  private async confirmAndApplyUpdate(msg: InboundMessage, providedToken: string | undefined): Promise<void> {
    const adapter = this.ctx.adapter!;
    const chatId = msg.chatId;
    const threadId = msg.threadId;

    const pending = this.pendingUpdate;
    if (!pending) {
      await adapter.sendText(chatId, "ℹ️ No pending /update. Run /update to start.", { threadId });
      return;
    }
    if (Date.now() > pending.expiresAt) {
      this.pendingUpdate = null;
      await adapter.sendText(chatId, "⏱ Confirmation expired. Run /update again.", { threadId });
      return;
    }
    if (!providedToken || providedToken !== pending.token) {
      await adapter.sendText(chatId, "⛔ Wrong confirmation token.", { threadId });
      return;
    }
    if (pending.userId && String(pending.userId) !== String(msg.userId)) {
      await adapter.sendText(chatId, "⛔ Only the user who ran /update can confirm.", { threadId });
      return;
    }
    // Single-use: clear immediately so a repeat /update confirm <token> is rejected.
    this.pendingUpdate = null;

    const targetSpec = pending.version ? `@suzuke/agend@${pending.version}` : "@suzuke/agend@latest";
    await adapter.sendText(chatId, `📦 Installing ${targetSpec}...`, { threadId });

    try {
      await this.exec(`npm install -g ${targetSpec}`, { timeout: 120_000 });
    } catch {
      await adapter.sendText(chatId,
        `❌ npm install failed. Previous version (${pending.previousVersion}) is still in place.`,
        { threadId });
      return;
    }

    // Health probe: best-effort sanity check that the new install loads.
    let probeOk = true;
    try {
      await this.exec("agend --version", { timeout: 10_000 });
    } catch {
      probeOk = false;
    }

    if (!probeOk) {
      await adapter.sendText(chatId,
        `⚠️ New install failed --version probe. Rolling back to ${pending.previousVersion}...`,
        { threadId });
      try {
        await this.exec(`npm install -g @suzuke/agend@${pending.previousVersion}`, { timeout: 120_000 });
        await adapter.sendText(chatId, `↩️ Rolled back to ${pending.previousVersion}.`, { threadId });
      } catch {
        await adapter.sendText(chatId,
          `🚨 Rollback failed. Manual recovery needed: npm install -g @suzuke/agend@${pending.previousVersion}`,
          { threadId });
      }
      return;
    }

    await adapter.sendText(chatId, "✅ Updated. Restarting service...", { threadId });
    // Brief delay to let sendText complete before process dies
    await new Promise(r => setTimeout(r, 1000));

    const label = "com.agend.fleet";
    const plat = detectPlatform();

    if (plat === "macos") {
      const plistPath = join(homedir(), "Library/LaunchAgents", `${label}.plist`);
      if (existsSync(plistPath)) {
        const uid = process.getuid?.() ?? 501;
        try {
          await this.exec(`launchctl kickstart -k gui/${uid}/${label}`, { timeout: 15_000 });
          return;
        } catch {
          await adapter.sendText(chatId, "⚠️ Failed to restart launchd service", { threadId });
          return;
        }
      }
    } else {
      try {
        await this.exec(`systemctl --user restart ${label}`, { timeout: 15_000 });
        return;
      } catch { /* no systemd service */ }
    }

    // Fallback: signal running daemon
    const pidPath = join(this.ctx.dataDir, "fleet.pid");
    if (existsSync(pidPath)) {
      const pid = parseInt(readFileSync(pidPath, "utf-8").trim(), 10);
      try {
        process.kill(pid, "SIGUSR1");
      } catch {
        await adapter.sendText(chatId, "⚠️ Fleet not running", { threadId });
      }
    } else {
      await adapter.sendText(chatId, "⚠️ No service or running fleet found", { threadId });
    }
  }

  /** Reply with redirect when message arrives in an unbound topic */
  async handleUnboundTopic(msg: InboundMessage): Promise<void> {
    if (!this.ctx.adapter) return;
    await this.ctx.adapter.sendText(
      msg.chatId,
      "This topic is not bound to an instance. Ask the General assistant to create one with create_instance.",
      { threadId: msg.threadId },
    );
  }

  /** Handle topic deletion — stop daemon and remove from config */
  async handleTopicDeleted(threadId: string): Promise<void> {
    const target = this.ctx.routingTable.get(threadId);
    if (!target) return;
    if (target.kind === "general") {
      this.ctx.logger.debug({ instanceName: target.name, threadId }, "Ignoring delete event for General topic");
      return;
    }

    this.ctx.logger.info({ instanceName: target.name, threadId }, "Topic deleted — auto-unbinding");
    await this.ctx.removeInstance(target.name);
  }

  /** Create instance config, save fleet.yaml, start daemon, connect IPC. */
  async bindAndStart(dirPath: string, topicId: number | string): Promise<string> {
    if (!this.ctx.fleetConfig) throw new Error("Fleet config not loaded");

    const instanceName = `${sanitizeInstanceName(basename(dirPath))}-t${topicId}`;

    this.ctx.fleetConfig.instances[instanceName] = {
      working_directory: dirPath,
      topic_id: topicId,
      restart_policy: this.ctx.fleetConfig.defaults.restart_policy ?? DEFAULT_INSTANCE_CONFIG.restart_policy,
      context_guardian: this.ctx.fleetConfig.defaults.context_guardian ?? DEFAULT_INSTANCE_CONFIG.context_guardian,
      log_level: this.ctx.fleetConfig.defaults.log_level ?? DEFAULT_INSTANCE_CONFIG.log_level,
    };

    this.ctx.saveFleetConfig();
    this.ctx.routingTable.set(String(topicId), { kind: "instance", name: instanceName });

    // startInstance awaits lifecycle.start → daemon.start (IPC listening) →
    // connectIpcToInstance. By the time it resolves, IPC is already wired —
    // the previous code's 5s sleep + second connect was leftover paranoia.
    await this.ctx.startInstance(instanceName, this.ctx.fleetConfig.instances[instanceName], true);

    this.ctx.logger.info({ instanceName, topicId }, "Topic bound and started");
    return instanceName;
  }

  /** Create Telegram topics for instances that don't have topic_id */
  async autoCreateTopics(): Promise<void> {
    if (!this.ctx.fleetConfig?.channel?.group_id) return;
    const botToken = process.env[this.ctx.fleetConfig.channel.bot_token_env];
    if (!botToken) return;

    let configChanged = false;
    for (const [name, config] of Object.entries(this.ctx.fleetConfig.instances)) {
      if (config.topic_id != null) continue;

      // Telegram's native General topic always has thread_id = 1
      if (config.general_topic) {
        config.topic_id = 1;
        configChanged = true;
        this.ctx.logger.info({ name, topicId: 1 }, "Bound to native General topic");
        continue;
      }

      try {
        const topicName = basename(config.working_directory);
        const threadId = await this.ctx.createForumTopic(topicName);
        config.topic_id = threadId;
        configChanged = true;
        this.ctx.logger.info({ name, topicId: config.topic_id, topicName }, "Auto-created Telegram topic");
      } catch (err) {
        this.ctx.logger.warn({ name, err }, "Failed to auto-create topic");
      }
    }

    if (configChanged) {
      this.ctx.saveFleetConfig();
    }
  }

  /** Register bot commands in Telegram command menu */
  async registerBotCommands(): Promise<void> {
    const groupId = this.ctx.fleetConfig?.channel?.group_id;
    const botTokenEnv = this.ctx.fleetConfig?.channel?.bot_token_env;
    if (!groupId || !botTokenEnv) return;
    const botToken = process.env[botTokenEnv];
    if (!botToken) return;

    try {
      await fetch(
        `https://api.telegram.org/bot${botToken}/setMyCommands`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            commands: [
              { command: "status", description: "Show fleet status and costs" },
              { command: "restart", description: "Graceful restart all instances" },
              { command: "sysinfo", description: "System diagnostics" },
              { command: "update", description: "Update AgEnD (two-step confirm; allow-listed users only)" },
            ],
            scope: { type: "chat", chat_id: groupId },
          }),
        },
      );
      this.ctx.logger.info("Registered bot commands: /status");
    } catch (err) {
      this.ctx.logger.warn({ err }, "Failed to register bot commands (non-fatal)");
    }
  }
}
