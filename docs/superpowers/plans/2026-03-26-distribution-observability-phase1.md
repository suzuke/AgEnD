# Phase 1: Distribution + Foundation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make CCD installable via npm, add CI/CD, event logging, cost guard, Telegram /status, and graceful shutdown — so strangers can use CCD without risking bill shock.

**Architecture:** Event log (SQLite `events` table) is the foundation — built first, then cost guard and /status read from it. npm publish and CI are independent. Graceful shutdown adds a save-state prompt before killing tmux windows. All changes are additive — no existing behavior is modified.

**Tech Stack:** TypeScript, better-sqlite3, commander (CLI), Grammy (Telegram), GitHub Actions, vitest

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `src/event-log.ts` | EventLog class: SQLite events table, insert/query methods |
| Create | `tests/event-log.test.ts` | Unit tests for EventLog |
| Modify | `src/types.ts` | Add `CostGuardConfig` type, add `cost_guard` to `FleetDefaults` |
| Create | `src/cost-guard.ts` | CostGuard class: reads statusline, tracks daily cost in cents, emits warn/limit events |
| Create | `tests/cost-guard.test.ts` | Unit tests for CostGuard |
| Modify | `src/fleet-manager.ts` | Integrate EventLog, CostGuard, /status command, graceful shutdown prompt |
| Modify | `src/topic-commands.ts` | Add /status command handler |
| Modify | `src/fleet-context.ts` | Expose `getInstanceStatus` + cost data on FleetContext |
| Modify | `src/daemon.ts` | Emit cost snapshot before context rotation |
| Modify | `src/context-guardian.ts` | Emit `pre_rotate` event (for cost snapshot timing) |
| Modify | `src/cli.ts` | Add `ccd fleet history` subcommand |
| Modify | `src/config.ts` | Add `DEFAULT_COST_GUARD` defaults |
| Modify | `package.json` | Add `files`, `prepublishOnly`, `repository`, `keywords` |
| Create | `.github/workflows/ci.yml` | PR: test + typecheck. Tag: npm publish |

---

### Task 1: Event Log

**Files:**
- Create: `src/event-log.ts`
- Create: `tests/event-log.test.ts`

- [ ] **Step 1: Write the failing test for EventLog**

```typescript
// tests/event-log.test.ts
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { EventLog } from "../src/event-log.js";
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("EventLog", () => {
  let tmpDir: string;
  let eventLog: EventLog;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `ccd-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    eventLog = new EventLog(join(tmpDir, "events.db"));
  });

  afterEach(() => {
    eventLog.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("inserts and queries events", () => {
    eventLog.insert("proj-a", "cost_snapshot", { cost_cents: 320 });
    eventLog.insert("proj-a", "context_rotation", { reason: "context_full" });
    eventLog.insert("proj-b", "cost_snapshot", { cost_cents: 150 });

    const all = eventLog.query();
    expect(all).toHaveLength(3);
  });

  it("filters by instance", () => {
    eventLog.insert("proj-a", "cost_snapshot", { cost_cents: 320 });
    eventLog.insert("proj-b", "cost_snapshot", { cost_cents: 150 });

    const result = eventLog.query({ instance: "proj-a" });
    expect(result).toHaveLength(1);
    expect(result[0].instance_name).toBe("proj-a");
  });

  it("filters by event type", () => {
    eventLog.insert("proj-a", "cost_snapshot", { cost_cents: 320 });
    eventLog.insert("proj-a", "context_rotation", { reason: "context_full" });

    const result = eventLog.query({ type: "cost_snapshot" });
    expect(result).toHaveLength(1);
  });

  it("filters by since date", () => {
    eventLog.insert("proj-a", "cost_snapshot", { cost_cents: 100 });
    const result = eventLog.query({ since: "2020-01-01" });
    expect(result.length).toBeGreaterThanOrEqual(1);
  });

  it("returns parsed payload JSON", () => {
    eventLog.insert("proj-a", "cost_snapshot", { cost_cents: 320 });
    const [event] = eventLog.query();
    expect(event.payload).toEqual({ cost_cents: 320 });
  });

  it("prunes old events", () => {
    eventLog.insert("proj-a", "cost_snapshot", { cost_cents: 100 });
    // Prune events older than 0 days (everything)
    eventLog.prune(0);
    const result = eventLog.query();
    expect(result).toHaveLength(0);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/event-log.test.ts`
Expected: FAIL — `event-log.js` does not exist

- [ ] **Step 3: Implement EventLog**

```typescript
// src/event-log.ts
import Database from "better-sqlite3";

export interface EventRow {
  id: number;
  instance_name: string;
  event_type: string;
  payload: Record<string, unknown> | null;
  created_at: string;
}

interface RawRow {
  id: number;
  instance_name: string;
  event_type: string;
  payload: string | null;
  created_at: string;
}

export interface EventQuery {
  instance?: string;
  type?: string;
  since?: string;
  limit?: number;
}

export class EventLog {
  private db: Database.Database;

  constructor(dbPath: string) {
    this.db = new Database(dbPath);
    this.db.pragma("journal_mode = WAL");
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS events (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        instance_name TEXT NOT NULL,
        event_type TEXT NOT NULL,
        payload TEXT,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
      );
      CREATE INDEX IF NOT EXISTS idx_events_instance ON events(instance_name, created_at);
      CREATE INDEX IF NOT EXISTS idx_events_type ON events(event_type, created_at);
    `);
  }

  insert(instance: string, type: string, payload?: Record<string, unknown>): void {
    this.db.prepare(
      "INSERT INTO events (instance_name, event_type, payload) VALUES (?, ?, ?)"
    ).run(instance, type, payload ? JSON.stringify(payload) : null);
  }

  query(opts: EventQuery = {}): EventRow[] {
    const conditions: string[] = [];
    const params: unknown[] = [];

    if (opts.instance) { conditions.push("instance_name = ?"); params.push(opts.instance); }
    if (opts.type) { conditions.push("event_type = ?"); params.push(opts.type); }
    if (opts.since) { conditions.push("created_at >= ?"); params.push(opts.since); }

    const where = conditions.length > 0 ? `WHERE ${conditions.join(" AND ")}` : "";
    const limit = opts.limit ?? 500;

    const rows = this.db.prepare(
      `SELECT * FROM events ${where} ORDER BY created_at DESC, id DESC LIMIT ?`
    ).all(...params, limit) as RawRow[];

    return rows.map(r => ({
      ...r,
      payload: r.payload ? JSON.parse(r.payload) : null,
    }));
  }

  prune(days: number): void {
    this.db.prepare(
      "DELETE FROM events WHERE created_at < datetime('now', '-' || ? || ' days')"
    ).run(days);
  }

  close(): void {
    this.db.close();
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run tests/event-log.test.ts`
Expected: All 6 tests PASS

- [ ] **Step 5: Add `ccd fleet history` CLI command**

In `src/cli.ts`, add after the `fleet logs` command:

```typescript
fleet
  .command("history")
  .description("Show event history")
  .option("--instance <name>", "Filter by instance")
  .option("--type <type>", "Filter by event type")
  .option("--since <date>", "Show events since date (ISO format)")
  .option("--limit <n>", "Max events to show", "50")
  .option("--json", "Output as JSON")
  .action(async (opts) => {
    const { EventLog } = await import("./event-log.js");
    const eventLog = new EventLog(join(DATA_DIR, "events.db"));
    try {
      const events = eventLog.query({
        instance: opts.instance,
        type: opts.type,
        since: opts.since,
        limit: parseInt(opts.limit, 10),
      });
      if (opts.json) {
        console.log(JSON.stringify(events, null, 2));
        return;
      }
      if (events.length === 0) {
        console.log("No events found.");
        return;
      }
      console.log("Time\t\t\tInstance\t\tType\t\t\tPayload");
      for (const e of events) {
        const payload = e.payload ? JSON.stringify(e.payload) : "";
        console.log(`${e.created_at}\t${e.instance_name}\t${e.event_type}\t${payload.slice(0, 60)}`);
      }
    } finally {
      eventLog.close();
    }
  });
```

- [ ] **Step 6: Commit**

```bash
git add src/event-log.ts tests/event-log.test.ts src/cli.ts
git commit -m "feat: add event log (SQLite events table + ccd fleet history)"
```

---

### Task 2: Cost Guard

**Files:**
- Modify: `src/types.ts` — add CostGuardConfig
- Modify: `src/config.ts` — add DEFAULT_COST_GUARD
- Create: `src/cost-guard.ts`
- Create: `tests/cost-guard.test.ts`

- [ ] **Step 1: Add types**

In `src/types.ts`, add after `AccessConfig`:

```typescript
export interface CostGuardConfig {
  daily_limit_usd: number;
  warn_at_percentage: number;
  timezone: string;
}
```

In `FleetDefaults`, add:

```typescript
cost_guard?: CostGuardConfig;
```

In `src/config.ts`, add:

```typescript
export const DEFAULT_COST_GUARD: CostGuardConfig = {
  daily_limit_usd: 0,  // 0 = disabled
  warn_at_percentage: 80,
  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
};
```

- [ ] **Step 2: Write failing tests for CostGuard**

```typescript
// tests/cost-guard.test.ts
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { CostGuard } from "../src/cost-guard.js";
import { EventLog } from "../src/event-log.js";
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("CostGuard", () => {
  let tmpDir: string;
  let eventLog: EventLog;
  let guard: CostGuard;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `ccd-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    eventLog = new EventLog(join(tmpDir, "events.db"));
    guard = new CostGuard(
      { daily_limit_usd: 10, warn_at_percentage: 80, timezone: "UTC" },
      eventLog,
    );
  });

  afterEach(() => {
    eventLog.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("tracks cost in cents", () => {
    guard.updateCost("proj-a", 3.50);
    expect(guard.getDailyCostCents("proj-a")).toBe(350);
  });

  it("accumulates across sessions (rotation)", () => {
    guard.updateCost("proj-a", 3.50);
    guard.snapshotAndReset("proj-a");  // simulate rotation
    guard.updateCost("proj-a", 1.20);  // new session reports only 1.20
    expect(guard.getDailyCostCents("proj-a")).toBe(470);
  });

  it("emits warn when threshold exceeded", () => {
    const handler = vi.fn();
    guard.on("warn", handler);
    guard.updateCost("proj-a", 8.50); // 85% of $10 limit
    expect(handler).toHaveBeenCalledWith("proj-a", 850, 1000);
  });

  it("emits limit when daily limit exceeded", () => {
    const handler = vi.fn();
    guard.on("limit", handler);
    guard.updateCost("proj-a", 10.50);
    expect(handler).toHaveBeenCalledWith("proj-a", 1050, 1000);
  });

  it("does not emit when limit is 0 (disabled)", () => {
    const disabledGuard = new CostGuard(
      { daily_limit_usd: 0, warn_at_percentage: 80, timezone: "UTC" },
      eventLog,
    );
    const handler = vi.fn();
    disabledGuard.on("warn", handler);
    disabledGuard.on("limit", handler);
    disabledGuard.updateCost("proj-a", 999);
    expect(handler).not.toHaveBeenCalled();
  });

  it("resets at midnight", () => {
    guard.updateCost("proj-a", 5.00);
    guard.resetDaily();
    expect(guard.getDailyCostCents("proj-a")).toBe(0);
  });

  it("logs cost_snapshot event on snapshot", () => {
    guard.updateCost("proj-a", 3.50);
    guard.snapshotAndReset("proj-a");
    const events = eventLog.query({ type: "cost_snapshot", instance: "proj-a" });
    expect(events).toHaveLength(1);
    expect(events[0].payload).toEqual({ cost_cents: 350, reason: "rotation" });
  });

  it("returns fleet total", () => {
    guard.updateCost("proj-a", 3.00);
    guard.updateCost("proj-b", 2.00);
    expect(guard.getFleetTotalCents()).toBe(500);
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `npx vitest run tests/cost-guard.test.ts`
Expected: FAIL — `cost-guard.js` does not exist

- [ ] **Step 4: Implement CostGuard**

```typescript
// src/cost-guard.ts
import { EventEmitter } from "node:events";
import type { CostGuardConfig } from "./types.js";
import type { EventLog } from "./event-log.js";

export class CostGuard extends EventEmitter {
  // instance → { previousSessionsCents, currentSessionCostUsd }
  private trackers = new Map<string, { accumulatedCents: number; lastReportedUsd: number }>();
  private warnedInstances = new Set<string>();
  private limitedInstances = new Set<string>();
  private midnightTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private config: CostGuardConfig,
    private eventLog: EventLog,
  ) {
    super();
  }

  /** Called when statusline.json is read — costUsd is the current session's total_cost_usd */
  updateCost(instance: string, costUsd: number): void {
    let tracker = this.trackers.get(instance);
    if (!tracker) {
      tracker = { accumulatedCents: 0, lastReportedUsd: 0 };
      this.trackers.set(instance, tracker);
    }
    tracker.lastReportedUsd = costUsd;

    if (this.config.daily_limit_usd <= 0) return;

    const totalCents = this.getDailyCostCents(instance);
    const limitCents = Math.round(this.config.daily_limit_usd * 100);
    const warnCents = Math.round(limitCents * this.config.warn_at_percentage / 100);

    if (totalCents >= limitCents && !this.limitedInstances.has(instance)) {
      this.limitedInstances.add(instance);
      this.emit("limit", instance, totalCents, limitCents);
    } else if (totalCents >= warnCents && !this.warnedInstances.has(instance)) {
      this.warnedInstances.add(instance);
      this.emit("warn", instance, totalCents, limitCents);
    }
  }

  /** Snapshot cost before context rotation, then reset current session tracker */
  snapshotAndReset(instance: string): void {
    const tracker = this.trackers.get(instance);
    if (!tracker) return;
    const currentCents = Math.round(tracker.lastReportedUsd * 100);
    tracker.accumulatedCents += currentCents;
    tracker.lastReportedUsd = 0;
    this.eventLog.insert(instance, "cost_snapshot", {
      cost_cents: tracker.accumulatedCents,
      reason: "rotation",
    });
  }

  getDailyCostCents(instance: string): number {
    const tracker = this.trackers.get(instance);
    if (!tracker) return 0;
    return tracker.accumulatedCents + Math.round(tracker.lastReportedUsd * 100);
  }

  getFleetTotalCents(): number {
    let total = 0;
    for (const [, tracker] of this.trackers) {
      total += tracker.accumulatedCents + Math.round(tracker.lastReportedUsd * 100);
    }
    return total;
  }

  getLimitCents(): number {
    return Math.round(this.config.daily_limit_usd * 100);
  }

  isLimited(instance: string): boolean {
    return this.limitedInstances.has(instance);
  }

  resetDaily(): void {
    for (const [, tracker] of this.trackers) {
      tracker.accumulatedCents = 0;
      tracker.lastReportedUsd = 0;
    }
    this.warnedInstances.clear();
    this.limitedInstances.clear();
  }

  /** Start midnight reset timer */
  startMidnightReset(): void {
    const scheduleNext = () => {
      const now = new Date();
      // Calculate next midnight in configured timezone
      const formatter = new Intl.DateTimeFormat("en-US", {
        timeZone: this.config.timezone,
        hour: "numeric", minute: "numeric", second: "numeric", hour12: false,
      });
      const parts = formatter.formatToParts(now);
      const h = parseInt(parts.find(p => p.type === "hour")!.value, 10);
      const m = parseInt(parts.find(p => p.type === "minute")!.value, 10);
      const s = parseInt(parts.find(p => p.type === "second")!.value, 10);
      const secondsUntilMidnight = (24 * 3600) - (h * 3600 + m * 60 + s);
      this.midnightTimer = setTimeout(() => {
        this.resetDaily();
        this.emit("daily_reset");
        scheduleNext();
      }, secondsUntilMidnight * 1000);
    };
    scheduleNext();
  }

  stop(): void {
    if (this.midnightTimer) {
      clearTimeout(this.midnightTimer);
      this.midnightTimer = null;
    }
  }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `npx vitest run tests/cost-guard.test.ts`
Expected: All 8 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/types.ts src/config.ts src/cost-guard.ts tests/cost-guard.test.ts
git commit -m "feat: add cost guard with daily limit, warn threshold, and rotation snapshots"
```

---

### Task 3: Integrate Cost Guard + Event Log into Fleet Manager

**Files:**
- Modify: `src/fleet-manager.ts`
- Modify: `src/daemon.ts`
- Modify: `src/context-guardian.ts`

- [ ] **Step 1: Add `pre_rotate` event to ContextGuardian**

In `src/context-guardian.ts`, modify `enterRotating()`:

```typescript
private enterRotating(): void {
  this.state = "ROTATING";
  this.emit("pre_rotate");  // emitted before rotate — listeners should snapshot state
  this.emit("rotate");
}
```

- [ ] **Step 2: Wire cost snapshot on pre_rotate in Daemon**

In `src/daemon.ts`, in the `guardian.on("rotate")` section (around line 269), add a `pre_rotate` listener BEFORE the existing `rotate` listener:

```typescript
this.guardian.on("pre_rotate", () => {
  // Fleet manager listens for this to snapshot cost before kill
  // Daemon reads current statusline cost and emits it
  const statusFile = join(this.instanceDir, "statusline.json");
  try {
    if (existsSync(statusFile)) {
      const data = JSON.parse(readFileSync(statusFile, "utf-8"));
      const costUsd = data.cost?.total_cost_usd ?? 0;
      this.emit("pre_rotate_cost", this.name, costUsd);
    }
  } catch { /* ignore */ }
});
```

Note: Daemon must extend EventEmitter or expose the event. Since Daemon doesn't currently extend EventEmitter, the cleanest approach is to have FleetManager read the statusline directly when it detects a rotation. Instead, have fleet manager hook into the IPC notification. **Simpler approach**: Fleet manager reads statusline.json for each instance when context guardian status_update fires (it already does this). On rotation, fleet manager snapshots from the last known cost.

**Revised approach — keep it in FleetManager only:**

Skip the `pre_rotate` event. Instead, in `FleetManager.startAll()`, after connecting IPC, register a statusline watcher per instance that feeds the CostGuard. The CostGuard already tracks `lastReportedUsd`, so on rotation the fleet manager calls `costGuard.snapshotAndReset(name)`.

- [ ] **Step 3: Initialize EventLog and CostGuard in FleetManager**

In `src/fleet-manager.ts`, add imports and fields:

```typescript
import { EventLog } from "./event-log.js";
import { CostGuard } from "./cost-guard.js";
import { DEFAULT_COST_GUARD } from "./config.js";
import type { CostGuardConfig } from "./types.js";
```

Add fields to the class:

```typescript
eventLog: EventLog | null = null;
costGuard: CostGuard | null = null;
private statuslineWatchers: Map<string, ReturnType<typeof setInterval>> = new Map();
```

In `startAll()`, before starting instances:

```typescript
// Initialize event log
this.eventLog = new EventLog(join(this.dataDir, "events.db"));

// Initialize cost guard
const costGuardConfig: CostGuardConfig = {
  ...DEFAULT_COST_GUARD,
  ...(this.fleetConfig?.defaults as Record<string, unknown>)?.cost_guard as Partial<CostGuardConfig> ?? {},
};
this.costGuard = new CostGuard(costGuardConfig, this.eventLog);
this.costGuard.startMidnightReset();

this.costGuard.on("warn", (instance: string, totalCents: number, limitCents: number) => {
  const msg = `⚠️ ${instance} 花費已達 $${(totalCents / 100).toFixed(2)} / $${(limitCents / 100).toFixed(2)} (${Math.round(totalCents / limitCents * 100)}%)`;
  this.notifyInstanceTopic(instance, msg);
});

this.costGuard.on("limit", (instance: string, totalCents: number, limitCents: number) => {
  const msg = `🛑 ${instance} 已達每日花費上限 $${(limitCents / 100).toFixed(2)}，自動暫停。`;
  this.notifyInstanceTopic(instance, msg);
  this.eventLog?.insert(instance, "instance_paused", { reason: "cost_limit", cost_cents: totalCents });
  this.stopInstance(instance).catch(err => this.logger.error({ err, instance }, "Failed to pause instance"));
});
```

- [ ] **Step 4: Add statusline watcher for cost tracking**

In `FleetManager`, after connecting IPC to each instance, start a periodic statusline reader:

```typescript
private startStatuslineWatcher(name: string): void {
  const statusFile = join(this.getInstanceDir(name), "statusline.json");
  const timer = setInterval(() => {
    try {
      if (!existsSync(statusFile)) return;
      const data = JSON.parse(readFileSync(statusFile, "utf-8"));
      const costUsd = data.cost?.total_cost_usd ?? 0;
      this.costGuard?.updateCost(name, costUsd);
    } catch { /* ignore */ }
  }, 10_000); // Every 10 seconds
  this.statuslineWatchers.set(name, timer);
}
```

Call `this.startStatuslineWatcher(name)` after `connectIpcToInstance(name)`.

- [ ] **Step 5: Snapshot cost on context rotation**

The fleet manager doesn't directly know when rotation happens. But the daemon kills its tmux window and respawns — the statusline.json cost resets. The simplest approach: in `startStatuslineWatcher`, detect when `total_cost_usd` drops (new session started) and call `snapshotAndReset`:

```typescript
// Inside the watcher interval, track last known cost
private lastKnownCost = new Map<string, number>();

// In the watcher:
const prevCost = this.lastKnownCost.get(name) ?? 0;
if (costUsd < prevCost && prevCost > 0) {
  // Cost dropped = new session (rotation happened)
  this.costGuard?.snapshotAndReset(name);
}
this.lastKnownCost.set(name, costUsd);
```

- [ ] **Step 6: Add `notifyInstanceTopic` helper**

```typescript
private notifyInstanceTopic(instanceName: string, text: string): void {
  if (!this.adapter) return;
  const groupId = this.fleetConfig?.channel?.group_id;
  if (!groupId) return;
  const instanceConfig = this.fleetConfig?.instances[instanceName];
  const threadId = instanceConfig?.topic_id ? String(instanceConfig.topic_id) : undefined;
  this.adapter.sendText(String(groupId), text, { threadId })
    .catch(e => this.logger.debug({ err: e }, "Failed to send cost notification"));
}
```

- [ ] **Step 7: Cleanup watchers in stopAll()**

In `stopAll()`, add before stopping instances:

```typescript
for (const [, timer] of this.statuslineWatchers) clearInterval(timer);
this.statuslineWatchers.clear();
this.costGuard?.stop();
this.eventLog?.close();
```

- [ ] **Step 8: Commit**

```bash
git add src/fleet-manager.ts src/daemon.ts src/context-guardian.ts
git commit -m "feat: integrate cost guard + event log into fleet manager"
```

---

### Task 4: Telegram /status Command

**Files:**
- Modify: `src/topic-commands.ts`
- Modify: `src/fleet-context.ts`

- [ ] **Step 1: Extend FleetContext interface**

In `src/fleet-context.ts`, add:

```typescript
import type { CostGuard } from "./cost-guard.js";

// In FleetContext interface, add:
readonly costGuard: CostGuard | null;
getInstanceStatus(name: string): "running" | "stopped" | "crashed";
```

- [ ] **Step 2: Add /status handler to TopicCommands**

In `src/topic-commands.ts`, in `handleGeneralCommand()`, add before the `return false`:

```typescript
if (text === "/status" || text === "/status@" || text.startsWith("/status@")) {
  await this.handleStatusCommand(msg);
  return true;
}
```

Add the handler method:

```typescript
private async handleStatusCommand(msg: InboundMessage): Promise<void> {
  if (!this.ctx.adapter || !this.ctx.fleetConfig) return;

  const lines: string[] = [];
  for (const [name, inst] of Object.entries(this.ctx.fleetConfig.instances)) {
    const status = this.ctx.getInstanceStatus(name);

    // Read statusline for context usage
    let contextStr = "-";
    const statusFile = join(this.ctx.dataDir, "instances", name, "statusline.json");
    try {
      if (existsSync(statusFile)) {
        const data = JSON.parse(readFileSync(statusFile, "utf-8"));
        if (data.context_window?.used_percentage != null) {
          contextStr = `${Math.round(data.context_window.used_percentage)}%`;
        }
      }
    } catch { /* ignore */ }

    const costCents = this.ctx.costGuard?.getDailyCostCents(name) ?? 0;
    const costStr = `$${(costCents / 100).toFixed(2)}`;
    const paused = this.ctx.costGuard?.isLimited(name);

    let icon: string;
    if (paused) icon = "⏸";
    else if (status === "running") icon = "🟢";
    else if (status === "crashed") icon = "🔴";
    else icon = "⚪";

    lines.push(`${icon} ${name} — ctx ${contextStr}, ${costStr} today`);
  }

  if (lines.length === 0) {
    lines.push("No instances configured.");
  }

  const limitCents = this.ctx.costGuard?.getLimitCents() ?? 0;
  const totalCents = this.ctx.costGuard?.getFleetTotalCents() ?? 0;
  if (limitCents > 0) {
    lines.push("");
    lines.push(`Fleet: $${(totalCents / 100).toFixed(2)} / $${(limitCents / 100).toFixed(2)} daily`);
  }

  await this.ctx.adapter.sendText(msg.chatId, lines.join("\n"));
}
```

Add imports at top of `topic-commands.ts`:

```typescript
import { readFileSync } from "node:fs";
```

- [ ] **Step 3: Register /status in bot commands**

In `registerBotCommands()`, add to the commands array:

```typescript
{ command: "status", description: "Show fleet status" },
```

- [ ] **Step 4: Commit**

```bash
git add src/topic-commands.ts src/fleet-context.ts
git commit -m "feat: add Telegram /status command for fleet overview"
```

---

### Task 5: Graceful Shutdown Notification

**Files:**
- Modify: `src/daemon.ts`
- Modify: `src/fleet-manager.ts`

- [ ] **Step 1: Add graceful stop to Daemon**

In `src/daemon.ts`, add a new method:

```typescript
/** Send save-state prompt to Claude before stopping */
async gracefulStop(): Promise<void> {
  if (this.tmux && await this.tmux.isWindowAlive()) {
    this.logger.info("Sending save-state prompt before shutdown");
    await this.tmux.sendKeys("The system is shutting down. Please save any important state to memory files now. You have 30 seconds.");
    await new Promise(r => setTimeout(r, 500));
    await this.tmux.sendSpecialKey("Enter");

    // Wait for idle or timeout
    await Promise.race([
      this.waitForTranscriptIdle(10_000),
      new Promise(r => setTimeout(r, 30_000)),
    ]);
  }
  await this.stop();
}
```

- [ ] **Step 2: Use gracefulStop in FleetManager.stopAll()**

In `src/fleet-manager.ts`, modify `stopAll()` to use `gracefulStop` instead of `stopInstance`:

```typescript
async stopAll(): Promise<void> {
  // ... existing cleanup code (timers, watchers, etc.) ...

  await Promise.allSettled(
    [...this.daemons.entries()].map(async ([name, daemon]) => {
      try {
        await daemon.gracefulStop();
      } catch (err) {
        this.logger.warn({ name, err }, "Graceful stop failed, force stopping");
        await this.stopInstance(name);
      }
      this.daemons.delete(name);
    })
  );

  // ... rest of existing cleanup ...
}
```

- [ ] **Step 3: Commit**

```bash
git add src/daemon.ts src/fleet-manager.ts
git commit -m "feat: graceful shutdown — prompt Claude to save state before kill"
```

---

### Task 6: npm Publish Preparation

**Files:**
- Modify: `package.json`

- [ ] **Step 1: Update package.json**

Add/update these fields in `package.json`:

```json
{
  "files": [
    "dist/",
    "templates/",
    "README.md",
    "README.zh-TW.md"
  ],
  "repository": {
    "type": "git",
    "url": "https://github.com/suzuke/claude-channel-daemon.git"
  },
  "keywords": [
    "claude",
    "claude-code",
    "telegram",
    "daemon",
    "ai-agent",
    "fleet-management"
  ]
}
```

Add to `scripts`:

```json
"prepublishOnly": "npm run build"
```

- [ ] **Step 2: Verify build and pack**

Run: `npm run build && npm pack --dry-run`
Expected: Lists the files that would be included. Verify dist/, templates/, README.md are included but no src/, tests/, .env, etc.

- [ ] **Step 3: Commit**

```bash
git add package.json
git commit -m "chore: prepare package.json for npm publish"
```

---

### Task 7: GitHub Actions CI

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create CI workflow**

```yaml
# .github/workflows/ci.yml
name: CI

on:
  pull_request:
  push:
    tags:
      - 'v*'

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 20
      - run: npm ci
      - run: npx tsc --noEmit
      - run: npm test

  publish:
    needs: test
    if: startsWith(github.ref, 'refs/tags/v')
    runs-on: ubuntu-latest
    permissions:
      contents: read
      id-token: write
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          registry-url: https://registry.npmjs.org
      - run: npm ci
      - run: npm publish --provenance --access public
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
```

- [ ] **Step 2: Commit**

```bash
mkdir -p .github/workflows
git add .github/workflows/ci.yml
git commit -m "ci: add GitHub Actions for test + npm publish on tag"
```

---

## Build Order

```
Task 1 (Event Log) ──────── independent, do first
Task 2 (Cost Guard) ─────── depends on Task 1 (uses EventLog)
Task 3 (Integration) ────── depends on Tasks 1 + 2
Task 4 (/status) ────────── depends on Task 3 (reads CostGuard)
Task 5 (Graceful shutdown)─ independent
Task 6 (npm publish) ────── independent
Task 7 (CI) ──────────────── independent
```

Tasks 1, 5, 6, 7 can run in parallel. Tasks 2-4 are sequential.
