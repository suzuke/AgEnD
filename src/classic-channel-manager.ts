import { existsSync, readFileSync, writeFileSync, mkdirSync, appendFileSync, readdirSync, unlinkSync, statSync } from "node:fs";
import { join } from "node:path";
import yaml from "js-yaml";
import { getAgendHome } from "./paths.js";
import type { Logger } from "./logger.js";

export interface ClassicChannel {
  channelId: string;
  instanceName: string;
  backend?: string;
  createdAt: string;
  createdBy: string;
}

interface ClassicBotYaml {
  defaults?: { backend?: string };
  channels?: Record<string, {
    channelId: string;
    backend?: string;
    createdBy?: string;
    createdAt?: string;
  }>;
}

const YAML_HEADER = `# ClassicBot Configuration
# Available backends: claude-code, gemini-cli, codex, opencode, kiro-cli
`;

/**
 * Manages classic bot channel lifecycle — register/unregister/persist.
 * Persists to ~/.agend/classicBot.yaml with per-channel backend override.
 */
export class ClassicChannelManager {
  private channels = new Map<string, ClassicChannel>();
  private defaults: { backend?: string } = {};
  private readonly configPath: string;
  private lastMtime = 0;

  constructor(private dataDir: string, private logger: Logger) {
    this.configPath = join(dataDir, "classicBot.yaml");
    this.load();
  }

  private load(): void {
    if (!existsSync(this.configPath)) return;
    try {
      const raw = yaml.load(readFileSync(this.configPath, "utf-8")) as ClassicBotYaml | null;
      if (!raw) return;
      this.defaults = raw.defaults ?? {};
      this.channels.clear();
      if (raw.channels) {
        for (const [key, val] of Object.entries(raw.channels)) {
          this.channels.set(val.channelId, {
            channelId: val.channelId,
            instanceName: `classic-${key}`,
            backend: val.backend,
            createdAt: val.createdAt ?? "",
            createdBy: val.createdBy ?? "",
          });
        }
      }
      this.lastMtime = statSync(this.configPath).mtimeMs;
      this.logger.info({ count: this.channels.size }, "Loaded classic channels");
    } catch (err) {
      this.logger.warn({ err }, "Failed to load classicBot.yaml");
    }
  }

  private save(): void {
    mkdirSync(this.dataDir, { recursive: true });
    const obj: ClassicBotYaml = { defaults: this.defaults, channels: {} };
    for (const ch of this.channels.values()) {
      const key = ch.instanceName.replace(/^classic-/, "");
      const entry: Record<string, unknown> = { channelId: ch.channelId, createdBy: ch.createdBy, createdAt: ch.createdAt };
      if (ch.backend) entry.backend = ch.backend;
      obj.channels![key] = entry as any;
    }
    writeFileSync(this.configPath, YAML_HEADER + yaml.dump(obj, { lineWidth: -1 }));
    this.lastMtime = existsSync(this.configPath) ? statSync(this.configPath).mtimeMs : 0;
  }

  /** Poll for external file changes (call periodically, e.g. every 30s) */
  checkReload(): boolean {
    if (!existsSync(this.configPath)) return false;
    const mtime = statSync(this.configPath).mtimeMs;
    if (mtime <= this.lastMtime) return false;
    this.logger.info("classicBot.yaml changed — reloading");
    this.load();
    return true;
  }

  getDefaults(): { backend?: string } { return this.defaults; }

  /** Backend fallback: per-channel → classic defaults → fleetDefault → "claude-code" */
  getBackend(channelId: string, fleetDefault?: string): string {
    const ch = this.channels.get(channelId);
    return ch?.backend || this.defaults.backend || fleetDefault || "claude-code";
  }

  /** Get backend for an instance by name */
  getBackendByInstance(instanceName: string, fleetDefault?: string): string {
    for (const ch of this.channels.values()) {
      if (ch.instanceName === instanceName) return ch.backend || this.defaults.backend || fleetDefault || "claude-code";
    }
    return this.defaults.backend || fleetDefault || "claude-code";
  }

  isClassicChannel(channelId: string): boolean { return this.channels.has(channelId); }
  get(channelId: string): ClassicChannel | undefined { return this.channels.get(channelId); }
  getAll(): ClassicChannel[] { return [...this.channels.values()]; }

  register(channelId: string, instanceName: string, userId: string): ClassicChannel {
    const ch: ClassicChannel = { channelId, instanceName, createdAt: new Date().toISOString(), createdBy: userId };
    this.channels.set(channelId, ch);
    this.save();
    this.logger.info({ channelId, instanceName }, "Registered classic channel");
    return ch;
  }

  unregister(channelId: string): ClassicChannel | undefined {
    const ch = this.channels.get(channelId);
    if (!ch) return undefined;
    this.channels.delete(channelId);
    this.save();
    this.logger.info({ channelId, instanceName: ch.instanceName }, "Unregistered classic channel");
    return ch;
  }

  static chatLogDir(instanceName: string): string {
    return join(getAgendHome(), "workspaces", instanceName, "chat-logs");
  }

  static logMessage(instanceName: string, username: string, text: string, timestamp: Date): void {
    const logDir = ClassicChannelManager.chatLogDir(instanceName);
    mkdirSync(logDir, { recursive: true });
    const dateStr = timestamp.toISOString().slice(0, 10);
    const logFile = join(logDir, `${dateStr}.log`);
    appendFileSync(logFile, `[${timestamp.toISOString()}] <${username}> ${text}\n`);
  }

  /** Delete chat log files older than retentionDays. Dates parsed as local to avoid UTC off-by-one. */
  rotateLogs(retentionDays = 7): number {
    let deleted = 0;
    const cutoff = Date.now() - retentionDays * 86400_000;
    for (const ch of this.channels.values()) {
      const logDir = ClassicChannelManager.chatLogDir(ch.instanceName);
      if (!existsSync(logDir)) continue;
      for (const file of readdirSync(logDir)) {
        const match = file.match(/^(\d{4})-(\d{2})-(\d{2})\.log$/);
        if (!match) continue;
        const fileDate = new Date(+match[1], +match[2] - 1, +match[3]).getTime();
        if (fileDate < cutoff) { unlinkSync(join(logDir, file)); deleted++; }
      }
    }
    if (deleted > 0) this.logger.info({ deleted }, "Rotated classic channel chat logs");
    return deleted;
  }
}
