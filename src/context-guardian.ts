import { EventEmitter } from "node:events";
import { readFileSync, watchFile, unwatchFile, existsSync } from "node:fs";
import type { ContextStatus, StatusLineData, InstanceConfig } from "./types.js";
import type { Logger } from "./logger.js";

type GuardianConfig = InstanceConfig["context_guardian"];

/**
 * ContextGuardian — pure monitoring, no restart triggers.
 *
 * All CLI backends (Claude Code, Codex, Gemini CLI, OpenCode, Kiro CLI) have
 * built-in auto-compact that handles context limits internally. AgEnD no longer
 * triggers restarts based on context usage or session age.
 *
 * Retained responsibilities:
 * - Poll statusline.json and emit "status_update" for dashboard/logging
 * - Crash recovery (health check + respawn) is handled by Daemon directly
 */
export class ContextGuardian extends EventEmitter {
  private statusFilePath: string;
  private consecutiveReadFailures = 0;

  constructor(
    private _config: GuardianConfig,
    private logger: Logger,
    statusFilePath: string,
  ) {
    super();
    this.statusFilePath = statusFilePath;
  }

  startWatching(): void {
    this.logger.debug({ path: this.statusFilePath }, "Watching status line file");
    watchFile(this.statusFilePath, { interval: 2000 }, () => this.readAndCheck());
  }

  private readAndCheck(): void {
    try {
      if (!existsSync(this.statusFilePath)) return;
      const raw = readFileSync(this.statusFilePath, "utf-8");
      const data: StatusLineData = JSON.parse(raw);
      const cw = data.context_window;

      if (cw.used_percentage != null) {
        this.consecutiveReadFailures = 0;
        const status: ContextStatus = {
          used_percentage: cw.used_percentage,
          remaining_percentage: cw.remaining_percentage ?? (100 - cw.used_percentage),
          context_window_size: cw.context_window_size,
        };
        const rl = data.rate_limits;
        this.logger.debug({
          context: `${cw.used_percentage}%`,
          cost: `$${data.cost.total_cost_usd.toFixed(2)}`,
          rate_5h: rl?.five_hour ? `${rl.five_hour.used_percentage}%` : "n/a",
          rate_7d: rl?.seven_day ? `${rl.seven_day.used_percentage}%` : "n/a",
        }, "Status update received");
        this.emit("status_update", { ...status, rate_limits: rl });
      } else {
        this.consecutiveReadFailures++;
        if (this.consecutiveReadFailures >= 3) {
          this.logger.warn({ consecutiveFailures: this.consecutiveReadFailures }, "Context usage unavailable for 3+ consecutive reads");
        }
      }
    } catch (err) {
      this.consecutiveReadFailures++;
      if (this.consecutiveReadFailures >= 3) {
        this.logger.warn({ err, consecutiveFailures: this.consecutiveReadFailures }, "Context usage read failed 3+ consecutive times");
      } else {
        this.logger.debug({ err }, "Failed to read status line file");
      }
    }
  }

  stop(): void {
    unwatchFile(this.statusFilePath);
  }
}
