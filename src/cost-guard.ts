import { EventEmitter } from "node:events";
import type { CostGuardConfig } from "./types.js";
import type { EventLog } from "./event-log.js";

interface InstanceTracker {
  accumulatedCents: number;
  lastReportedUsd: number;
  warnEmitted: boolean;
  limitEmitted: boolean;
}

export function formatCents(cents: number): string {
  return `$${(cents / 100).toFixed(2)}`;
}

function msUntilMidnight(timezone: string): number {
  const now = new Date();
  const tzNow = new Date(now.toLocaleString("en-US", { timeZone: timezone }));
  const tzMidnight = new Date(tzNow);
  tzMidnight.setHours(24, 0, 0, 0);
  const diff = tzMidnight.getTime() - tzNow.getTime();
  return diff > 0 ? diff : 24 * 60 * 60 * 1000;
}

export class CostGuard extends EventEmitter {
  private config: CostGuardConfig;
  private eventLog: EventLog;
  private trackers = new Map<string, InstanceTracker>();
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(config: CostGuardConfig, eventLog: EventLog) {
    super();
    this.config = config;
    this.eventLog = eventLog;
  }

  private getTracker(instance: string): InstanceTracker {
    let tracker = this.trackers.get(instance);
    if (!tracker) {
      tracker = {
        accumulatedCents: 0,
        lastReportedUsd: 0,
        warnEmitted: false,
        limitEmitted: false,
      };
      this.trackers.set(instance, tracker);
    }
    return tracker;
  }

  updateCost(instance: string, costUsd: number): void {
    const tracker = this.getTracker(instance);

    // Detect rotation: cost dropped = new session started
    if (costUsd < tracker.lastReportedUsd && tracker.lastReportedUsd > 0) {
      this.snapshotAndReset(instance);
    }

    tracker.lastReportedUsd = costUsd;

    if (this.config.daily_limit_usd <= 0) return;

    const totalCents = this.getDailyCostCents(instance);
    const limitCents = this.getLimitCents();

    if (!tracker.limitEmitted && totalCents >= limitCents) {
      tracker.limitEmitted = true;
      this.emit("limit", instance, totalCents, limitCents);
      return;
    }

    if (!tracker.warnEmitted) {
      const warnThresholdCents = Math.round(
        limitCents * (this.config.warn_at_percentage / 100),
      );
      if (totalCents >= warnThresholdCents) {
        tracker.warnEmitted = true;
        this.emit("warn", instance, totalCents, limitCents);
      }
    }
  }

  snapshotAndReset(instance: string): void {
    const tracker = this.getTracker(instance);
    const sessionCents = Math.round(tracker.lastReportedUsd * 100);
    tracker.accumulatedCents += sessionCents;
    const previousUsd = tracker.lastReportedUsd;
    tracker.lastReportedUsd = 0;
    // Reset per-day notification flags so a new session that pushes the
    // accumulated total past the threshold re-fires `warn` / `limit`. This
    // matters for the limit handler in particular: it pauses the instance,
    // and without re-firing a user-restarted instance can blow past the
    // daily cap again silently. We're still bounded — a single session
    // can fire each event at most once.
    tracker.warnEmitted = false;
    tracker.limitEmitted = false;

    this.eventLog.insert(instance, "cost_snapshot", {
      session_cost_usd: previousUsd,
      accumulated_cents: tracker.accumulatedCents,
    });
  }

  getDailyCostCents(instance: string): number {
    const tracker = this.trackers.get(instance);
    if (!tracker) return 0;
    return tracker.accumulatedCents + Math.round(tracker.lastReportedUsd * 100);
  }

  getFleetTotalCents(): number {
    let total = 0;
    for (const [instance] of this.trackers) {
      total += this.getDailyCostCents(instance);
    }
    return total;
  }

  getLimitCents(): number {
    return Math.round(this.config.daily_limit_usd * 100);
  }

  isLimited(instance: string): boolean {
    if (this.config.daily_limit_usd <= 0) return false;
    return this.getDailyCostCents(instance) >= this.getLimitCents();
  }

  resetDaily(): void {
    this.trackers.clear();
    this.emit("daily_reset");
  }

  startMidnightReset(): void {
    const schedule = () => {
      const ms = msUntilMidnight(this.config.timezone);
      this.timer = setTimeout(() => {
        this.resetDaily();
        schedule();
      }, ms);
    };
    schedule();
  }

  stop(): void {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
  }
}
