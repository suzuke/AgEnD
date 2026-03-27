# Phase 2: Observability — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add rate-limit-aware scheduling, hang detection, daily summary, and context rotation quality tracking to CCD's fleet management.

**Architecture:** All four features write to the existing EventLog (Phase 1). Rate limit data flows from statusline.json through the fleet manager's existing watcher. Hang detection lives in a new `HangDetector` class per daemon instance. Daily summary reads from EventLog + statusline on a cron timer. Rotation quality is tracked by hooking into ContextGuardian's state transitions.

**Tech Stack:** TypeScript, better-sqlite3 (EventLog), croner (daily summary timer), Grammy (Telegram inline buttons for hang detection)

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `src/hang-detector.ts` | HangDetector class: multi-signal hang detection per instance |
| Create | `tests/hang-detector.test.ts` | Unit tests for HangDetector |
| Create | `src/daily-summary.ts` | DailySummary class: cron-based fleet summary to Telegram General topic |
| Create | `tests/daily-summary.test.ts` | Unit tests for DailySummary |
| Modify | `src/fleet-manager.ts` | Wire rate limit tracking, HangDetector, DailySummary, rotation quality logging |
| Modify | `src/daemon.ts` | Expose `lastActivityTs` from transcript monitor, track rotation quality |
| Modify | `src/types.ts` | Add `HangDetectorConfig`, `DailySummaryConfig` to FleetDefaults |
| Modify | `src/config.ts` | Add defaults for hang detection and daily summary |

---

### Task 1: Rate Limit-Aware Scheduling

The fleet manager's statusline watcher already reads statusline.json every 10s. We need to: (a) cache rate_limits data, (b) check before triggering schedules, (c) defer triggers when rate limit is high.

**Files:**
- Modify: `src/fleet-manager.ts`

- [ ] **Step 1: Add rate limit tracking to the statusline watcher**

In `src/fleet-manager.ts`, add a field to cache rate limits per instance:

```typescript
private instanceRateLimits = new Map<string, { five_hour_pct: number; seven_day_pct: number }>();
```

In the `startStatuslineWatcher` interval callback, after `updateCost`, also cache rate limits:

```typescript
const rl = data.rate_limits;
if (rl) {
  this.instanceRateLimits.set(name, {
    five_hour_pct: rl.five_hour?.used_percentage ?? 0,
    seven_day_pct: rl.seven_day?.used_percentage ?? 0,
  });
}
```

- [ ] **Step 2: Add rate limit check in handleScheduleTrigger**

At the top of `handleScheduleTrigger`, before the `deliver()` call, add a deferral check:

```typescript
const RATE_LIMIT_DEFER_THRESHOLD = 85;
const rl = this.instanceRateLimits.get(target);
if (rl && rl.five_hour_pct > RATE_LIMIT_DEFER_THRESHOLD) {
  this.scheduler!.recordRun(id, "deferred", `5hr rate limit at ${rl.five_hour_pct}%`);
  this.eventLog?.insert(target, "schedule_deferred", {
    schedule_id: id,
    label,
    five_hour_pct: rl.five_hour_pct,
  });
  this.notifyInstanceTopic(target, `⏳ Schedule "${label ?? id}" deferred — rate limit at ${rl.five_hour_pct}%`);
  this.logger.info({ target, scheduleId: id, rateLimitPct: rl.five_hour_pct }, "Schedule deferred due to rate limit");
  return;
}
```

- [ ] **Step 3: Clean up instanceRateLimits in clearStatuslineWatchers**

Add `this.instanceRateLimits.clear();` to `clearStatuslineWatchers()`.

- [ ] **Step 4: Run all tests to verify no regressions**

Run: `npx vitest run`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/fleet-manager.ts
git commit -m "feat: rate limit-aware scheduling — defer triggers when 5hr usage > 85%"
```

---

### Task 2: Hang Detection

**Files:**
- Create: `src/hang-detector.ts`
- Create: `tests/hang-detector.test.ts`
- Modify: `src/types.ts`
- Modify: `src/config.ts`
- Modify: `src/daemon.ts`
- Modify: `src/fleet-manager.ts`

- [ ] **Step 1: Add config types**

In `src/types.ts`, add after `CostGuardConfig`:

```typescript
export interface HangDetectorConfig {
  enabled: boolean;
  timeout_minutes: number;
}
```

In `FleetDefaults`, add:

```typescript
hang_detector?: HangDetectorConfig;
```

In `src/config.ts`, add:

```typescript
export const DEFAULT_HANG_DETECTOR: HangDetectorConfig = {
  enabled: true,
  timeout_minutes: 15,
};
```

- [ ] **Step 2: Write failing tests for HangDetector**

```typescript
// tests/hang-detector.test.ts
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { HangDetector } from "../src/hang-detector.js";

describe("HangDetector", () => {
  let detector: HangDetector;

  beforeEach(() => {
    vi.useFakeTimers();
    detector = new HangDetector(15);
  });

  afterEach(() => {
    detector.stop();
    vi.useRealTimers();
  });

  it("does not flag as hung when activity is recent", () => {
    detector.recordActivity();
    expect(detector.isHung()).toBe(false);
  });

  it("flags as hung after timeout with no activity", () => {
    detector.recordActivity();
    vi.advanceTimersByTime(16 * 60 * 1000);
    expect(detector.isHung()).toBe(true);
  });

  it("resets hung state on new activity", () => {
    detector.recordActivity();
    vi.advanceTimersByTime(16 * 60 * 1000);
    expect(detector.isHung()).toBe(true);
    detector.recordActivity();
    expect(detector.isHung()).toBe(false);
  });

  it("emits hang event once when timeout is reached", () => {
    const handler = vi.fn();
    detector.on("hang", handler);
    detector.start();
    detector.recordActivity();
    vi.advanceTimersByTime(16 * 60 * 1000);
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("does not emit hang again until activity resumes and times out again", () => {
    const handler = vi.fn();
    detector.on("hang", handler);
    detector.start();
    detector.recordActivity();
    vi.advanceTimersByTime(16 * 60 * 1000); // first hang
    vi.advanceTimersByTime(16 * 60 * 1000); // still hung, no re-emit
    expect(handler).toHaveBeenCalledTimes(1);
    detector.recordActivity(); // resume
    vi.advanceTimersByTime(16 * 60 * 1000); // second hang
    expect(handler).toHaveBeenCalledTimes(2);
  });

  it("considers statusline freshness", () => {
    detector.recordActivity();
    detector.recordStatuslineUpdate();
    vi.advanceTimersByTime(16 * 60 * 1000);
    // statusline was updated recently — not hung
    // But if statusline is also stale, then hung
    expect(detector.isHung()).toBe(true); // both stale
  });

  it("not hung if statusline is fresh even if transcript is stale", () => {
    detector.recordActivity(); // transcript
    vi.advanceTimersByTime(10 * 60 * 1000);
    detector.recordStatuslineUpdate(); // statusline is fresher
    vi.advanceTimersByTime(6 * 60 * 1000); // 16 min since transcript, 6 since statusline
    expect(detector.isHung()).toBe(false); // statusline still fresh
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `npx vitest run tests/hang-detector.test.ts`
Expected: FAIL — `hang-detector.js` does not exist

- [ ] **Step 4: Implement HangDetector**

```typescript
// src/hang-detector.ts
import { EventEmitter } from "node:events";

export class HangDetector extends EventEmitter {
  private lastActivityTs = 0;
  private lastStatuslineTs = 0;
  private hungEmitted = false;
  private checkTimer: ReturnType<typeof setInterval> | null = null;
  private timeoutMs: number;

  constructor(timeoutMinutes: number) {
    super();
    this.timeoutMs = timeoutMinutes * 60 * 1000;
  }

  recordActivity(): void {
    this.lastActivityTs = Date.now();
    if (this.hungEmitted) {
      this.hungEmitted = false; // reset so next hang emits again
    }
  }

  recordStatuslineUpdate(): void {
    this.lastStatuslineTs = Date.now();
  }

  isHung(): boolean {
    if (this.lastActivityTs === 0) return false;
    const now = Date.now();
    const transcriptStale = now - this.lastActivityTs > this.timeoutMs;
    const statuslineStale = this.lastStatuslineTs === 0 || now - this.lastStatuslineTs > this.timeoutMs;
    return transcriptStale && statuslineStale;
  }

  start(intervalMs = 60_000): void {
    this.checkTimer = setInterval(() => {
      if (this.isHung() && !this.hungEmitted) {
        this.hungEmitted = true;
        this.emit("hang");
      }
    }, intervalMs);
  }

  stop(): void {
    if (this.checkTimer) {
      clearInterval(this.checkTimer);
      this.checkTimer = null;
    }
  }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `npx vitest run tests/hang-detector.test.ts`
Expected: All 7 tests PASS

- [ ] **Step 6: Wire HangDetector into Daemon**

In `src/daemon.ts`, add import and create a HangDetector:

```typescript
import { HangDetector } from "./hang-detector.js";
```

Add field: `private hangDetector: HangDetector | null = null;`

In `start()`, after transcript monitor setup (inside the `!this.config.lightweight` block), create and wire the hang detector:

```typescript
const hangConfig = { enabled: true, timeout_minutes: 15 }; // defaults
this.hangDetector = new HangDetector(hangConfig.timeout_minutes);

// Feed transcript events to hang detector
this.transcriptMonitor.on("tool_use", () => this.hangDetector?.recordActivity());
this.transcriptMonitor.on("tool_result", () => this.hangDetector?.recordActivity());
this.transcriptMonitor.on("assistant_text", () => this.hangDetector?.recordActivity());

// Feed statusline updates to hang detector
this.guardian.on("status_update", () => this.hangDetector?.recordStatuslineUpdate());

this.hangDetector.start();
```

In `stop()`, add: `this.hangDetector?.stop();`

Expose a getter for fleet manager to access:

```typescript
getHangDetector(): HangDetector | null { return this.hangDetector; }
```

- [ ] **Step 7: Wire hang detection notifications in FleetManager**

In `src/fleet-manager.ts`, after `startInstance`, access the daemon's hang detector and listen for hang events:

In `startAll()`, after `await this.startInstance(name, config, ...)`, get the daemon and wire the hang event:

```typescript
const daemon = this.daemons.get(name);
const hangDetector = daemon?.getHangDetector();
if (hangDetector) {
  hangDetector.on("hang", () => {
    this.eventLog?.insert(name, "hang_detected", {});
    this.logger.warn({ name }, "Instance appears hung");
    // Send Telegram notification with inline buttons
    this.sendHangNotification(name);
  });
}
```

Add `sendHangNotification` method:

```typescript
private async sendHangNotification(instanceName: string): Promise<void> {
  if (!this.adapter) return;
  const groupId = this.fleetConfig?.channel?.group_id;
  if (!groupId) return;
  const threadId = this.fleetConfig?.instances[instanceName]?.topic_id;

  const tgAdapter = this.adapter as TelegramAdapter;
  const { InlineKeyboard } = await import("grammy");
  const keyboard = new InlineKeyboard()
    .text("🔄 Force restart", `hang:restart:${instanceName}`)
    .text("⏳ Keep waiting", `hang:wait:${instanceName}`);

  await tgAdapter.sendTextWithKeyboard(
    String(groupId),
    `⚠️ ${instanceName} appears hung (no activity for 15+ minutes)`,
    keyboard,
    threadId != null ? String(threadId) : undefined,
  ).catch(e => this.logger.debug({ err: e }, "Failed to send hang notification"));
}
```

Handle callback query for hang actions in fleet manager's existing callback handler or by adding a new one in the adapter message handler:

In `handleInboundMessage` or the callback_query handler, add:

```typescript
// In the adapter.on("callback_query") handler, add:
if (data.callbackData.startsWith("hang:")) {
  const [, action, instanceName] = data.callbackData.split(":");
  if (action === "restart") {
    await this.stopInstance(instanceName);
    const config = this.fleetConfig?.instances[instanceName];
    if (config) {
      await this.startInstance(instanceName, config, true);
      await new Promise(r => setTimeout(r, 3000));
      await this.connectIpcToInstance(instanceName);
    }
    this.adapter?.editMessage(data.chatId, data.messageId, `🔄 ${instanceName} restarted.`)
      .catch(() => {});
  } else {
    this.adapter?.editMessage(data.chatId, data.messageId, `⏳ Continuing to wait for ${instanceName}.`)
      .catch(() => {});
  }
  return;
}
```

- [ ] **Step 8: Run all tests**

Run: `npx vitest run`
Expected: All tests pass

- [ ] **Step 9: Commit**

```bash
git add src/hang-detector.ts tests/hang-detector.test.ts src/daemon.ts src/fleet-manager.ts src/types.ts src/config.ts
git commit -m "feat: hang detection with multi-signal check and Telegram inline buttons"
```

---

### Task 3: Context Rotation Quality Tracking

**Files:**
- Modify: `src/daemon.ts`

- [ ] **Step 1: Track rotation quality in daemon**

In `src/daemon.ts`, add tracking in the context guardian event handlers.

Add fields:

```typescript
private rotationStartedAt = 0;
private preRotationContextPct = 0;
```

In the `guardian.on("request_handover")` handler (where HANDING_OVER starts), record the start time and context percentage:

```typescript
this.rotationStartedAt = Date.now();
this.preRotationContextPct = this.readContextPercentage();
```

In the `guardian.on("rotate")` handler (after rotation completes and before `markRotationComplete`), check handover quality:

```typescript
// Check handover quality
const durationMs = Date.now() - this.rotationStartedAt;
const memDir = this.config.memory_directory ?? join(
  homedir(),
  ".claude/projects",
  this.config.working_directory.replace(/\//g, "-").replace(/^-/, ""),
  "memory",
);
const handoverPath = join(memDir, "handover.md");
let handoverStatus: "complete" | "timeout" | "empty" = "empty";
try {
  const content = readFileSync(handoverPath, "utf-8").trim();
  handoverStatus = content.length > 0 ? "complete" : "empty";
} catch {
  handoverStatus = "empty";
}
// If completion timer fired (timeout), mark as timeout
if (this.guardian?.state === "ROTATING" && durationMs > this.config.context_guardian.completion_timeout_ms) {
  handoverStatus = "timeout";
}

this.emit("rotation_quality", {
  instance: this.name,
  handover_status: handoverStatus,
  duration_ms: durationMs,
  previous_context_pct: this.preRotationContextPct,
});
```

- [ ] **Step 2: Log rotation event in FleetManager**

In `FleetManager.startAll()`, after wiring hang detection per instance, also listen for `rotation_quality`:

```typescript
daemon?.on("rotation_quality", (data: Record<string, unknown>) => {
  this.eventLog?.insert(name, "context_rotation", data);
  this.logger.info({ name, ...data }, "Context rotation completed");
});
```

- [ ] **Step 3: Run all tests**

Run: `npx vitest run`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add src/daemon.ts src/fleet-manager.ts
git commit -m "feat: context rotation quality tracking — log handover status to events"
```

---

### Task 4: Daily Summary

**Files:**
- Create: `src/daily-summary.ts`
- Create: `tests/daily-summary.test.ts`
- Modify: `src/types.ts`
- Modify: `src/config.ts`
- Modify: `src/fleet-manager.ts`

- [ ] **Step 1: Add config types**

In `src/types.ts`, add after `HangDetectorConfig`:

```typescript
export interface DailySummaryConfig {
  enabled: boolean;
  hour: number; // 0-23
  minute: number; // 0-59
}
```

In `FleetDefaults`, add:

```typescript
daily_summary?: DailySummaryConfig;
```

In `src/config.ts`, add:

```typescript
export const DEFAULT_DAILY_SUMMARY: DailySummaryConfig = {
  enabled: true,
  hour: 21,
  minute: 0,
};
```

- [ ] **Step 2: Write failing tests for DailySummary**

```typescript
// tests/daily-summary.test.ts
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { DailySummary } from "../src/daily-summary.js";
import { EventLog } from "../src/event-log.js";
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("DailySummary", () => {
  let tmpDir: string;
  let eventLog: EventLog;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `ccd-summary-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    eventLog = new EventLog(join(tmpDir, "events.db"));
  });

  afterEach(() => {
    eventLog.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("generates summary text from events", () => {
    eventLog.insert("proj-a", "cost_snapshot", { accumulated_cents: 500 });
    eventLog.insert("proj-a", "context_rotation", { handover_status: "complete" });
    eventLog.insert("proj-b", "cost_snapshot", { accumulated_cents: 200 });
    eventLog.insert("proj-a", "schedule_deferred", {});
    eventLog.insert("proj-a", "hang_detected", {});

    const instances = ["proj-a", "proj-b"];
    const costData = new Map([["proj-a", 820], ["proj-b", 200]]);
    const summary = DailySummary.generateText(eventLog, instances, costData, 5000);
    expect(summary).toContain("proj-a");
    expect(summary).toContain("proj-b");
    expect(summary).toContain("$8.20");
    expect(summary).toContain("$10.20"); // fleet total
  });

  it("highlights anomalies", () => {
    eventLog.insert("proj-a", "hang_detected", {});
    eventLog.insert("proj-a", "instance_paused", { reason: "cost_limit" });
    eventLog.insert("proj-a", "context_rotation", { handover_status: "timeout" });

    const summary = DailySummary.generateText(eventLog, ["proj-a"], new Map([["proj-a", 0]]), 0);
    expect(summary).toContain("hang");
    expect(summary).toContain("timeout");
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `npx vitest run tests/daily-summary.test.ts`
Expected: FAIL — `daily-summary.js` does not exist

- [ ] **Step 4: Implement DailySummary**

```typescript
// src/daily-summary.ts
import { Cron } from "croner";
import type { EventLog } from "./event-log.js";
import type { DailySummaryConfig, CostGuardConfig } from "./types.js";
import { formatCents } from "./cost-guard.js";

export class DailySummary {
  private job: Cron | null = null;

  constructor(
    private config: DailySummaryConfig,
    private timezone: string,
    private onSummary: (text: string) => void,
    private getSummaryText: () => string,
  ) {}

  start(): void {
    if (!this.config.enabled) return;
    const cron = `${this.config.minute} ${this.config.hour} * * *`;
    this.job = new Cron(cron, { timezone: this.timezone }, () => {
      const text = this.getSummaryText();
      this.onSummary(text);
    });
  }

  stop(): void {
    this.job?.stop();
    this.job = null;
  }

  /** Generate summary text from event log data. Pure function for testability. */
  static generateText(
    eventLog: EventLog,
    instances: string[],
    costCentsMap: Map<string, number>,
    fleetTotalCents: number,
  ): string {
    const today = new Date().toISOString().split("T")[0];
    const todayEvents = eventLog.query({ since: today, limit: 1000 });

    const lines: string[] = [`📊 Daily Report — ${today}`, ""];

    for (const name of instances) {
      const instanceEvents = todayEvents.filter(e => e.instance_name === name);
      const rotations = instanceEvents.filter(e => e.event_type === "context_rotation").length;
      const hangs = instanceEvents.filter(e => e.event_type === "hang_detected").length;
      const scheduleRuns = instanceEvents.filter(e =>
        e.event_type === "schedule_deferred" || (e.payload as Record<string, unknown>)?.schedule_id
      ).length;
      const costCents = costCentsMap.get(name) ?? 0;
      const incompleteHandovers = instanceEvents.filter(e =>
        e.event_type === "context_rotation" &&
        (e.payload as Record<string, unknown>)?.handover_status !== "complete"
      ).length;

      let line = `${name}: ${formatCents(costCents)}`;
      if (rotations > 0) line += `, ${rotations} rotation${rotations > 1 ? "s" : ""}`;
      if (scheduleRuns > 0) line += `, ${scheduleRuns} schedule run${scheduleRuns > 1 ? "s" : ""}`;

      const anomalies: string[] = [];
      if (hangs > 0) anomalies.push(`${hangs} hang${hangs > 1 ? "s" : ""}`);
      if (incompleteHandovers > 0) anomalies.push(`${incompleteHandovers} incomplete/timeout handover${incompleteHandovers > 1 ? "s" : ""}`);
      if (anomalies.length > 0) line += ` ⚠️ ${anomalies.join(", ")}`;

      lines.push(line);
    }

    lines.push("");
    lines.push(`Total: ${formatCents(fleetTotalCents)}`);

    return lines.join("\n");
  }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `npx vitest run tests/daily-summary.test.ts`
Expected: All tests PASS

- [ ] **Step 6: Wire DailySummary into FleetManager**

In `src/fleet-manager.ts`, add imports:

```typescript
import { DailySummary } from "./daily-summary.js";
import { DEFAULT_DAILY_SUMMARY, DEFAULT_HANG_DETECTOR } from "./config.js";
import type { DailySummaryConfig } from "./types.js";
```

Add field: `private dailySummary: DailySummary | null = null;`

In `startAll()`, after cost guard setup:

```typescript
const summaryConfig: DailySummaryConfig = {
  ...DEFAULT_DAILY_SUMMARY,
  ...(fleet.defaults as Record<string, unknown>)?.daily_summary as Partial<DailySummaryConfig> ?? {},
};
const summaryTz = costGuardConfig.timezone;

this.dailySummary = new DailySummary(summaryConfig, summaryTz, (text) => {
  if (!this.adapter || !this.fleetConfig?.channel?.group_id) return;
  this.adapter.sendText(String(this.fleetConfig.channel.group_id), text)
    .catch(e => this.logger.debug({ err: e }, "Failed to send daily summary"));
}, () => {
  const instances = Object.keys(this.fleetConfig?.instances ?? {});
  const costMap = new Map<string, number>();
  for (const name of instances) {
    costMap.set(name, this.costGuard?.getDailyCostCents(name) ?? 0);
  }
  return DailySummary.generateText(
    this.eventLog!,
    instances,
    costMap,
    this.costGuard?.getFleetTotalCents() ?? 0,
  );
});
this.dailySummary.start();
```

In `stopAll()`, add: `this.dailySummary?.stop();`

- [ ] **Step 7: Run all tests**

Run: `npx vitest run`
Expected: All tests pass

- [ ] **Step 8: Commit**

```bash
git add src/daily-summary.ts tests/daily-summary.test.ts src/fleet-manager.ts src/types.ts src/config.ts
git commit -m "feat: daily summary — fleet overview posted to General topic at configured time"
```

---

## Build Order

```
Task 1 (Rate Limit) ────────── independent
Task 2 (Hang Detection) ────── independent
Task 3 (Rotation Quality) ──── independent
Task 4 (Daily Summary) ─────── independent (but benefits from Tasks 1-3 being done first so events exist)
```

Recommended: Task 1 → Task 3 → Task 2 → Task 4 (simplest first, daily summary last since it reads all event types)
