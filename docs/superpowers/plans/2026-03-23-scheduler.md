# Scheduler Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a fleet-level scheduling system that lets users create cron-based schedules via Telegram, with Claude calling MCP tools to register jobs that Fleet Manager triggers at the specified times.

**Architecture:** Scheduler is a module inside Fleet Manager. It uses croner for cron parsing/execution, better-sqlite3 for persistence (fleet-level `scheduler.db`), and communicates with daemons via existing IPC protocol. CLI operates directly on the DB and signals Fleet Manager via SIGHUP.

**Tech Stack:** TypeScript, croner, better-sqlite3 (existing), vitest, commander (existing)

**Spec:** `docs/superpowers/specs/2026-03-23-scheduler-design.md`

---

### Task 1: Install croner dependency

**Files:**
- Modify: `package.json`

- [ ] **Step 1: Install croner**

```bash
npm install croner
```

- [ ] **Step 2: Verify installation**

```bash
node -e "import('croner').then(m => console.log('croner OK'));"
```

Expected: `croner OK`

- [ ] **Step 3: Commit**

```bash
git add package.json package-lock.json
git commit -m "chore: add croner dependency for scheduler"
```

---

### Task 2: Scheduler types + config type extension

**Files:**
- Create: `src/scheduler/types.ts`
- Modify: `src/types.ts`

- [ ] **Step 1: Create the types file**

```typescript
export interface Schedule {
  id: string;
  cron: string;
  message: string;
  source: string;
  target: string;
  reply_chat_id: string;
  reply_thread_id: string | null;
  label: string | null;
  enabled: boolean;
  timezone: string;
  created_at: string;
  last_triggered_at: string | null;
  last_status: string | null;
}

export interface ScheduleRun {
  id: number;
  schedule_id: string;
  triggered_at: string;
  status: "delivered" | "delivered_fallback" | "retry" | "instance_offline" | "channel_dead";
  detail: string | null;
}

export interface CreateScheduleParams {
  cron: string;
  message: string;
  source: string;
  target: string;
  reply_chat_id: string;
  reply_thread_id: string | null;
  label?: string;
  timezone?: string;
}

export interface UpdateScheduleParams {
  cron?: string;
  message?: string;
  target?: string;
  label?: string;
  timezone?: string;
  enabled?: boolean;
}

export interface SchedulerConfig {
  max_schedules: number;
  default_timezone: string;
  retry_count: number;
  retry_interval_ms: number;
}

export const DEFAULT_SCHEDULER_CONFIG: SchedulerConfig = {
  max_schedules: 100,
  default_timezone: "Asia/Taipei",
  retry_count: 3,
  retry_interval_ms: 30_000,
};
```

- [ ] **Step 2: Extend FleetConfig in `src/types.ts`**

Add `scheduler` property to fleet defaults. In `src/types.ts`, update the `FleetConfig` interface's defaults to accept scheduler config:

```typescript
// Add to src/types.ts (after existing interfaces)
export interface FleetDefaults extends Partial<InstanceConfig> {
  scheduler?: {
    max_schedules?: number;
    default_timezone?: string;
    retry_count?: number;
    retry_interval_ms?: number;
  };
}
```

Update `FleetConfig.defaults` type from `Partial<InstanceConfig>` to `FleetDefaults`.

- [ ] **Step 3: Verify it compiles**

```bash
npx tsc --noEmit
```

Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add src/scheduler/types.ts src/types.ts
git commit -m "feat(scheduler): add type definitions and config types"
```

---

### Task 3: Scheduler DB layer

**Files:**
- Create: `src/scheduler/db.ts`
- Test: `src/scheduler/db.test.ts`

- [ ] **Step 1: Write the failing test for DB initialization and CRUD**

```typescript
// src/scheduler/db.test.ts
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { SchedulerDb } from "./db.js";

describe("SchedulerDb", () => {
  let dir: string;
  let db: SchedulerDb;

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "scheduler-test-"));
    db = new SchedulerDb(join(dir, "scheduler.db"));
  });

  afterEach(() => {
    db.close();
    rmSync(dir, { recursive: true, force: true });
  });

  it("creates tables on init", () => {
    // If constructor didn't throw, tables exist
    const schedules = db.list();
    expect(schedules).toEqual([]);
  });

  it("creates and retrieves a schedule", () => {
    const s = db.create({
      cron: "0 7 * * *",
      message: "test message",
      source: "proj-a",
      target: "proj-a",
      reply_chat_id: "-100123",
      reply_thread_id: "42",
      label: "daily test",
      timezone: "Asia/Taipei",
    });

    expect(s.id).toBeTruthy();
    expect(s.cron).toBe("0 7 * * *");
    expect(s.enabled).toBe(true);

    const fetched = db.get(s.id);
    expect(fetched).toEqual(s);
  });

  it("lists schedules with optional target filter", () => {
    db.create({ cron: "0 7 * * *", message: "a", source: "a", target: "a", reply_chat_id: "1", reply_thread_id: null });
    db.create({ cron: "0 8 * * *", message: "b", source: "a", target: "b", reply_chat_id: "1", reply_thread_id: null });

    expect(db.list()).toHaveLength(2);
    expect(db.list("a")).toHaveLength(1);
    expect(db.list("b")).toHaveLength(1);
  });

  it("updates a schedule", () => {
    const s = db.create({ cron: "0 7 * * *", message: "old", source: "a", target: "a", reply_chat_id: "1", reply_thread_id: null });
    const updated = db.update(s.id, { message: "new", enabled: false });

    expect(updated.message).toBe("new");
    expect(updated.enabled).toBe(false);
    expect(updated.cron).toBe("0 7 * * *"); // unchanged
  });

  it("deletes a schedule and cascades runs", () => {
    const s = db.create({ cron: "0 7 * * *", message: "x", source: "a", target: "a", reply_chat_id: "1", reply_thread_id: null });
    db.recordRun(s.id, "delivered");
    expect(db.getRuns(s.id)).toHaveLength(1);

    db.delete(s.id);
    expect(db.get(s.id)).toBeNull();
    expect(db.getRuns(s.id)).toHaveLength(0);
  });

  it("deleteByInstanceOrThread removes matching schedules", () => {
    db.create({ cron: "0 7 * * *", message: "a", source: "a", target: "proj-a", reply_chat_id: "1", reply_thread_id: "42" });
    db.create({ cron: "0 8 * * *", message: "b", source: "b", target: "proj-b", reply_chat_id: "1", reply_thread_id: "42" });
    db.create({ cron: "0 9 * * *", message: "c", source: "c", target: "proj-c", reply_chat_id: "1", reply_thread_id: "99" });

    const count = db.deleteByInstanceOrThread("proj-a", "42");
    expect(count).toBe(2); // proj-a by target + proj-b by thread_id
    expect(db.list()).toHaveLength(1);
  });

  it("records and retrieves runs", () => {
    const s = db.create({ cron: "0 7 * * *", message: "x", source: "a", target: "a", reply_chat_id: "1", reply_thread_id: null });
    db.recordRun(s.id, "delivered");
    db.recordRun(s.id, "instance_offline", "retry 3x failed");

    const runs = db.getRuns(s.id);
    expect(runs).toHaveLength(2);
    expect(runs[0].status).toBe("instance_offline"); // most recent first
    expect(runs[0].detail).toBe("retry 3x failed");
  });

  it("enforces max schedule count", () => {
    for (let i = 0; i < 5; i++) {
      db.create({ cron: "0 7 * * *", message: `m${i}`, source: "a", target: "a", reply_chat_id: "1", reply_thread_id: null });
    }
    expect(() =>
      db.create({ cron: "0 7 * * *", message: "over", source: "a", target: "a", reply_chat_id: "1", reply_thread_id: null }, 5)
    ).toThrow(/limit/i);
  });

  it("prunes old runs on init", () => {
    const s = db.create({ cron: "0 7 * * *", message: "x", source: "a", target: "a", reply_chat_id: "1", reply_thread_id: null });
    // Insert a run with old timestamp manually
    db["db"].prepare(
      "INSERT INTO schedule_runs (schedule_id, triggered_at, status) VALUES (?, datetime('now', '-60 days'), 'delivered')"
    ).run(s.id);
    db.recordRun(s.id, "delivered"); // recent one

    // Re-init triggers prune
    db.pruneOldRuns();
    const runs = db.getRuns(s.id);
    expect(runs).toHaveLength(1); // old one pruned
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npx vitest run src/scheduler/db.test.ts
```

Expected: FAIL — `Cannot find module './db.js'`

- [ ] **Step 3: Implement SchedulerDb**

```typescript
// src/scheduler/db.ts
import Database from "better-sqlite3";
import { randomUUID } from "node:crypto";
import type { Schedule, ScheduleRun, CreateScheduleParams, UpdateScheduleParams } from "./types.js";

export class SchedulerDb {
  private db: Database.Database;

  constructor(dbPath: string) {
    this.db = new Database(dbPath);
    this.db.pragma("journal_mode = WAL");
    this.db.pragma("foreign_keys = ON");
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS schedules (
        id              TEXT PRIMARY KEY,
        cron            TEXT NOT NULL,
        message         TEXT NOT NULL,
        source          TEXT NOT NULL,
        target          TEXT NOT NULL,
        reply_chat_id   TEXT NOT NULL,
        reply_thread_id TEXT,
        label           TEXT,
        enabled         INTEGER DEFAULT 1,
        timezone        TEXT DEFAULT 'Asia/Taipei',
        created_at      TEXT NOT NULL,
        last_triggered_at TEXT,
        last_status     TEXT
      );
      CREATE TABLE IF NOT EXISTS schedule_runs (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
        triggered_at TEXT NOT NULL DEFAULT (datetime('now')),
        status      TEXT NOT NULL,
        detail      TEXT
      );
      CREATE INDEX IF NOT EXISTS idx_schedule_runs_schedule_id ON schedule_runs(schedule_id);
    `);
  }

  private rowToSchedule(row: Record<string, unknown>): Schedule {
    return {
      id: row.id as string,
      cron: row.cron as string,
      message: row.message as string,
      source: row.source as string,
      target: row.target as string,
      reply_chat_id: row.reply_chat_id as string,
      reply_thread_id: row.reply_thread_id as string | null,
      label: row.label as string | null,
      enabled: row.enabled === 1,
      timezone: row.timezone as string,
      created_at: row.created_at as string,
      last_triggered_at: row.last_triggered_at as string | null,
      last_status: row.last_status as string | null,
    };
  }

  create(params: CreateScheduleParams, maxSchedules = 100): Schedule {
    const count = this.db.prepare("SELECT COUNT(*) as c FROM schedules").get() as { c: number };
    if (count.c >= maxSchedules) {
      throw new Error(`Schedule limit reached (${maxSchedules}). Delete existing schedules first.`);
    }

    const id = randomUUID();
    const now = new Date().toISOString();
    this.db.prepare(`
      INSERT INTO schedules (id, cron, message, source, target, reply_chat_id, reply_thread_id, label, timezone, created_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `).run(id, params.cron, params.message, params.source, params.target, params.reply_chat_id, params.reply_thread_id, params.label ?? null, params.timezone ?? "Asia/Taipei", now);

    return this.get(id)!;
  }

  get(id: string): Schedule | null {
    const row = this.db.prepare("SELECT * FROM schedules WHERE id = ?").get(id) as Record<string, unknown> | undefined;
    return row ? this.rowToSchedule(row) : null;
  }

  list(target?: string): Schedule[] {
    const rows = target
      ? this.db.prepare("SELECT * FROM schedules WHERE target = ? ORDER BY created_at").all(target) as Record<string, unknown>[]
      : this.db.prepare("SELECT * FROM schedules ORDER BY created_at").all() as Record<string, unknown>[];
    return rows.map((r) => this.rowToSchedule(r));
  }

  update(id: string, params: UpdateScheduleParams): Schedule {
    const sets: string[] = [];
    const values: unknown[] = [];

    if (params.cron !== undefined) { sets.push("cron = ?"); values.push(params.cron); }
    if (params.message !== undefined) { sets.push("message = ?"); values.push(params.message); }
    if (params.target !== undefined) { sets.push("target = ?"); values.push(params.target); }
    if (params.label !== undefined) { sets.push("label = ?"); values.push(params.label); }
    if (params.timezone !== undefined) { sets.push("timezone = ?"); values.push(params.timezone); }
    if (params.enabled !== undefined) { sets.push("enabled = ?"); values.push(params.enabled ? 1 : 0); }

    if (sets.length > 0) {
      values.push(id);
      this.db.prepare(`UPDATE schedules SET ${sets.join(", ")} WHERE id = ?`).run(...values);
    }

    return this.get(id)!;
  }

  delete(id: string): void {
    this.db.prepare("DELETE FROM schedules WHERE id = ?").run(id);
  }

  deleteByInstanceOrThread(instanceName: string, threadId: string): number {
    const result = this.db.prepare("DELETE FROM schedules WHERE target = ? OR reply_thread_id = ?").run(instanceName, threadId);
    return result.changes;
  }

  recordRun(scheduleId: string, status: string, detail?: string): void {
    this.db.prepare("INSERT INTO schedule_runs (schedule_id, status, detail) VALUES (?, ?, ?)").run(scheduleId, status, detail ?? null);
    this.db.prepare("UPDATE schedules SET last_triggered_at = datetime('now'), last_status = ? WHERE id = ?").run(status, scheduleId);
  }

  getRuns(scheduleId: string, limit = 50): ScheduleRun[] {
    return this.db.prepare("SELECT * FROM schedule_runs WHERE schedule_id = ? ORDER BY triggered_at DESC LIMIT ?").all(scheduleId, limit) as ScheduleRun[];
  }

  pruneOldRuns(days = 30): void {
    this.db.prepare(`DELETE FROM schedule_runs WHERE triggered_at < datetime('now', '-${days} days')`).run();
  }

  close(): void {
    this.db.close();
  }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
npx vitest run src/scheduler/db.test.ts
```

Expected: All 8 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/scheduler/db.ts src/scheduler/db.test.ts
git commit -m "feat(scheduler): add DB layer with CRUD and run history"
```

---

### Task 4: Scheduler engine (cron management)

**Files:**
- Create: `src/scheduler/scheduler.ts`
- Test: `src/scheduler/scheduler.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
// src/scheduler/scheduler.test.ts
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { Scheduler } from "./scheduler.js";
import type { Schedule, SchedulerConfig } from "./types.js";
import { DEFAULT_SCHEDULER_CONFIG } from "./types.js";

describe("Scheduler", () => {
  let dir: string;
  let scheduler: Scheduler;
  let triggered: Schedule[];

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "scheduler-engine-test-"));
    triggered = [];
    scheduler = new Scheduler(
      join(dir, "scheduler.db"),
      (schedule) => { triggered.push(schedule); },
      DEFAULT_SCHEDULER_CONFIG,
      (instanceName: string) => true, // all instances valid
    );
    scheduler.init();
  });

  afterEach(() => {
    scheduler.shutdown();
    rmSync(dir, { recursive: true, force: true });
  });

  it("creates a schedule and registers cron job", () => {
    const s = scheduler.create({
      cron: "0 7 * * *",
      message: "hello",
      source: "proj-a",
      target: "proj-a",
      reply_chat_id: "1",
      reply_thread_id: null,
    });

    expect(s.id).toBeTruthy();
    expect(scheduler.list()).toHaveLength(1);
  });

  it("rejects invalid cron expression", () => {
    expect(() =>
      scheduler.create({
        cron: "not a cron",
        message: "hello",
        source: "a",
        target: "a",
        reply_chat_id: "1",
        reply_thread_id: null,
      })
    ).toThrow(/cron/i);
  });

  it("rejects invalid target instance", () => {
    const s2 = new Scheduler(
      join(dir, "scheduler2.db"),
      () => {},
      DEFAULT_SCHEDULER_CONFIG,
      (name: string) => name === "proj-a", // only proj-a valid
    );
    s2.init();

    expect(() =>
      s2.create({
        cron: "0 7 * * *",
        message: "hello",
        source: "proj-a",
        target: "nonexistent",
        reply_chat_id: "1",
        reply_thread_id: null,
      })
    ).toThrow(/not found/i);

    s2.shutdown();
  });

  it("manual trigger calls onTrigger callback", () => {
    const s = scheduler.create({
      cron: "0 7 * * *",
      message: "hello",
      source: "proj-a",
      target: "proj-a",
      reply_chat_id: "1",
      reply_thread_id: null,
    });

    scheduler.trigger(s.id);
    expect(triggered).toHaveLength(1);
    expect(triggered[0].id).toBe(s.id);
  });

  it("delete removes cron job", () => {
    const s = scheduler.create({
      cron: "0 7 * * *",
      message: "hello",
      source: "a",
      target: "a",
      reply_chat_id: "1",
      reply_thread_id: null,
    });
    scheduler.delete(s.id);
    expect(scheduler.list()).toHaveLength(0);
  });

  it("update reschedules cron job", () => {
    const s = scheduler.create({
      cron: "0 7 * * *",
      message: "hello",
      source: "a",
      target: "a",
      reply_chat_id: "1",
      reply_thread_id: null,
    });
    const updated = scheduler.update(s.id, { cron: "0 8 * * *" });
    expect(updated.cron).toBe("0 8 * * *");
  });

  it("reload clears and re-registers all jobs", () => {
    scheduler.create({
      cron: "0 7 * * *",
      message: "hello",
      source: "a",
      target: "a",
      reply_chat_id: "1",
      reply_thread_id: null,
    });
    scheduler.reload();
    expect(scheduler.list()).toHaveLength(1); // still in DB, re-registered
  });

  it("deleteByInstanceOrThread cleans up and removes cron jobs", () => {
    scheduler.create({
      cron: "0 7 * * *",
      message: "a",
      source: "a",
      target: "proj-a",
      reply_chat_id: "1",
      reply_thread_id: "42",
    });
    const count = scheduler.deleteByInstanceOrThread("proj-a", "42");
    expect(count).toBe(1);
    expect(scheduler.list()).toHaveLength(0);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
npx vitest run src/scheduler/scheduler.test.ts
```

Expected: FAIL — `Cannot find module './scheduler.js'`

- [ ] **Step 3: Implement Scheduler class**

```typescript
// src/scheduler/scheduler.ts
import { Cron } from "croner";
import { SchedulerDb } from "./db.js";
import type { Schedule, CreateScheduleParams, UpdateScheduleParams, SchedulerConfig } from "./types.js";

export class Scheduler {
  private db: SchedulerDb;
  private jobs: Map<string, Cron> = new Map();
  private onTrigger: (schedule: Schedule) => void;
  private config: SchedulerConfig;
  private isValidInstance: (name: string) => boolean;

  constructor(
    dbPath: string,
    onTrigger: (schedule: Schedule) => void,
    config: SchedulerConfig,
    isValidInstance: (name: string) => boolean,
  ) {
    this.db = new SchedulerDb(dbPath);
    this.onTrigger = onTrigger;
    this.config = config;
    this.isValidInstance = isValidInstance;
  }

  init(): void {
    this.db.pruneOldRuns();
    this.registerAllJobs();
  }

  reload(): void {
    this.stopAllJobs();
    this.registerAllJobs();
  }

  shutdown(): void {
    this.stopAllJobs();
    this.db.close();
  }

  create(params: CreateScheduleParams): Schedule {
    // Validate cron
    try {
      new Cron(params.cron, { timezone: params.timezone ?? this.config.default_timezone });
    } catch (err) {
      throw new Error(`Invalid cron expression: ${(err as Error).message}`);
    }

    // Validate target
    if (!this.isValidInstance(params.target)) {
      throw new Error(`Instance "${params.target}" not found in fleet config.`);
    }

    const schedule = this.db.create(params, this.config.max_schedules);
    this.registerJob(schedule);
    return schedule;
  }

  list(target?: string): Schedule[] {
    return this.db.list(target);
  }

  get(id: string): Schedule | null {
    return this.db.get(id);
  }

  update(id: string, params: UpdateScheduleParams): Schedule {
    if (params.cron !== undefined) {
      try {
        new Cron(params.cron, { timezone: this.db.get(id)?.timezone ?? this.config.default_timezone });
      } catch (err) {
        throw new Error(`Invalid cron expression: ${(err as Error).message}`);
      }
    }

    if (params.target !== undefined && !this.isValidInstance(params.target)) {
      throw new Error(`Instance "${params.target}" not found in fleet config.`);
    }

    const updated = this.db.update(id, params);

    // Re-register cron job (may have changed cron, timezone, or enabled)
    this.stopJob(id);
    if (updated.enabled) {
      this.registerJob(updated);
    }

    return updated;
  }

  delete(id: string): void {
    this.stopJob(id);
    this.db.delete(id);
  }

  trigger(id: string): void {
    const schedule = this.db.get(id);
    if (!schedule) throw new Error(`Schedule "${id}" not found.`);
    this.onTrigger(schedule);
  }

  deleteByInstanceOrThread(instanceName: string, threadId: string): number {
    // Get affected IDs first so we can stop their cron jobs
    const affected = this.db.list().filter(
      (s) => s.target === instanceName || s.reply_thread_id === threadId,
    );
    for (const s of affected) {
      this.stopJob(s.id);
    }
    return this.db.deleteByInstanceOrThread(instanceName, threadId);
  }

  recordRun(scheduleId: string, status: string, detail?: string): void {
    this.db.recordRun(scheduleId, status, detail);
  }

  getRuns(scheduleId: string, limit?: number): import("./types.js").ScheduleRun[] {
    return this.db.getRuns(scheduleId, limit);
  }

  private registerAllJobs(): void {
    for (const schedule of this.db.list()) {
      if (schedule.enabled) {
        this.registerJob(schedule);
      }
    }
  }

  private registerJob(schedule: Schedule): void {
    const job = new Cron(schedule.cron, { timezone: schedule.timezone }, () => {
      // Re-read from DB to get latest state
      const current = this.db.get(schedule.id);
      if (current && current.enabled) {
        this.onTrigger(current);
      }
    });
    this.jobs.set(schedule.id, job);
  }

  private stopJob(id: string): void {
    const job = this.jobs.get(id);
    if (job) {
      job.stop();
      this.jobs.delete(id);
    }
  }

  private stopAllJobs(): void {
    for (const [id, job] of this.jobs) {
      job.stop();
    }
    this.jobs.clear();
  }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
npx vitest run src/scheduler/scheduler.test.ts
```

Expected: All 8 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/scheduler/scheduler.ts src/scheduler/scheduler.test.ts
git commit -m "feat(scheduler): add Scheduler engine with cron management"
```

---

### Task 5: Export scheduler module

**Files:**
- Create: `src/scheduler/index.ts`

- [ ] **Step 1: Create barrel export**

```typescript
// src/scheduler/index.ts
export { Scheduler } from "./scheduler.js";
export { SchedulerDb } from "./db.js";
export type { Schedule, ScheduleRun, CreateScheduleParams, UpdateScheduleParams, SchedulerConfig } from "./types.js";
export { DEFAULT_SCHEDULER_CONFIG } from "./types.js";
```

- [ ] **Step 2: Verify build**

```bash
npm run build
```

Expected: No errors.

- [ ] **Step 3: Commit**

```bash
git add src/scheduler/index.ts
git commit -m "feat(scheduler): add barrel export"
```

---

### Task 6: Fleet Manager — PID file + SIGHUP handler

**Files:**
- Modify: `src/fleet-manager.ts`

- [ ] **Step 1: Add PID file write to `startAll()` method**

In `fleet-manager.ts`, after the line that ensures the tmux session exists (around line 154), add PID file writing. Import `writeFileSync`, `unlinkSync` from `node:fs` if not already imported.

```typescript
// Add to startAll(), after tmux session setup:
const pidPath = join(this.baseDir, "fleet.pid");
writeFileSync(pidPath, String(process.pid), "utf-8");
```

- [ ] **Step 2: Add PID file cleanup on shutdown**

In the shutdown/cleanup logic (or add a new method called from signal handlers), add:

```typescript
// Add to shutdown/stopAll logic:
const pidPath = join(this.baseDir, "fleet.pid");
try { unlinkSync(pidPath); } catch {}
```

- [ ] **Step 3: Add SIGHUP handler placeholder**

After `startAll()` completes, add:

```typescript
process.on("SIGHUP", () => {
  this.logger.info("Received SIGHUP, reloading scheduler...");
  // Scheduler reload will be wired in Task 7
});
```

- [ ] **Step 4: Verify build**

```bash
npm run build
```

Expected: No errors.

- [ ] **Step 5: Commit**

```bash
git add src/fleet-manager.ts
git commit -m "feat(scheduler): add fleet.pid file and SIGHUP handler"
```

---

### Task 7: Fleet Manager — Scheduler integration

**Files:**
- Modify: `src/fleet-manager.ts`

- [ ] **Step 1: Import and instantiate Scheduler**

Add import at top of file:

```typescript
import { Scheduler } from "./scheduler/index.js";
import type { Schedule, SchedulerConfig } from "./scheduler/index.js";
import { DEFAULT_SCHEDULER_CONFIG } from "./scheduler/index.js";
```

Add field to class:

```typescript
private scheduler: Scheduler | null = null;
```

- [ ] **Step 2: Initialize Scheduler in `startAll()`**

After building routing table and before starting the adapter:

```typescript
const schedulerConfig: SchedulerConfig = {
  ...DEFAULT_SCHEDULER_CONFIG,
  ...(this.fleetConfig?.defaults as Record<string, unknown>)?.scheduler as Partial<SchedulerConfig> ?? {},
};

this.scheduler = new Scheduler(
  join(this.baseDir, "scheduler.db"),
  (schedule) => this.handleScheduleTrigger(schedule),
  schedulerConfig,
  (name) => this.fleetConfig?.instances?.[name] != null,
);
this.scheduler.init();
this.logger.info("Scheduler initialized");
```

Wire SIGHUP handler:

```typescript
process.on("SIGHUP", () => {
  this.logger.info("Received SIGHUP, reloading scheduler...");
  this.scheduler?.reload();
});
```

- [ ] **Step 3: Implement `handleScheduleTrigger`**

Add method to FleetManager class:

```typescript
private async handleScheduleTrigger(schedule: Schedule): Promise<void> {
  const { target, reply_chat_id, reply_thread_id, message, label, id, source } = schedule;
  const defaults = this.fleetConfig?.defaults as Record<string, unknown> | undefined;
  const schedulerDefaults = defaults?.scheduler as Record<string, unknown> | undefined;

  const retryCount = (schedulerDefaults?.retry_count as number) ?? 3;
  const retryInterval = (schedulerDefaults?.retry_interval_ms as number) ?? 30_000;

  const deliver = (): boolean => {
    const ipc = this.instanceIpcClients.get(target);
    if (!ipc?.connected) return false;

    ipc.send({
      type: "fleet_schedule_trigger",
      payload: { schedule_id: id, message: `[排程任務] ${message}`, label },
      meta: { chat_id: reply_chat_id, thread_id: reply_thread_id, user: "scheduler" },
    });
    return true;
  };

  // Try immediate delivery
  if (deliver()) {
    this.scheduler!.recordRun(id, "delivered");
    // Cross-instance notification
    if (source !== target) {
      this.notifySourceTopic(schedule);
    }
    return;
  }

  // Retry loop (async, non-blocking)
  for (let i = 0; i < retryCount; i++) {
    await new Promise((r) => setTimeout(r, retryInterval));
    if (deliver()) {
      this.scheduler!.recordRun(id, "delivered");
      if (source !== target) this.notifySourceTopic(schedule);
      return;
    }
  }

  // All retries failed
  this.scheduler!.recordRun(id, "instance_offline", `retry ${retryCount}x failed`);
  this.notifyScheduleFailure(schedule);
}

private notifySourceTopic(schedule: Schedule): void {
  if (!this.adapter) return;
  const text = `⏰ 排程「${schedule.label ?? schedule.id}」已觸發，目標實例：${schedule.target}`;
  this.adapter.sendText(schedule.reply_chat_id, text, {
    threadId: schedule.reply_thread_id ?? undefined,
  }).catch((err) => this.logger.error({ err }, "Failed to send cross-instance notification"));
}

private notifyScheduleFailure(schedule: Schedule): void {
  if (!this.adapter) return;
  const text = `⏰ 排程「${schedule.label ?? schedule.id}」觸發失敗：實例 ${schedule.target} 未在線。`;
  this.adapter.sendText(schedule.reply_chat_id, text, {
    threadId: schedule.reply_thread_id ?? undefined,
  }).catch((err) => this.logger.error({ err }, "Failed to send schedule failure notification"));
}
```

- [ ] **Step 4: Add schedule CRUD IPC handler in `connectIpcToInstance`**

In the IPC message handler (around line 252), add new conditions:

```typescript
} else if (msg.type === "fleet_schedule_create" || msg.type === "fleet_schedule_list" ||
           msg.type === "fleet_schedule_update" || msg.type === "fleet_schedule_delete") {
  this.handleScheduleCrud(name, msg);
}
```

Implement the handler:

```typescript
private handleScheduleCrud(instanceName: string, msg: Record<string, unknown>): void {
  const requestId = msg.requestId as string;
  const payload = (msg.payload ?? {}) as Record<string, unknown>;
  const meta = (msg.meta ?? {}) as Record<string, string>;
  const ipc = this.instanceIpcClients.get(instanceName);
  if (!ipc) return;

  try {
    let result: unknown;

    switch (msg.type) {
      case "fleet_schedule_create": {
        const params = {
          cron: payload.cron as string,
          message: payload.message as string,
          source: instanceName,
          target: (payload.target as string) || instanceName,
          reply_chat_id: meta.chat_id,
          reply_thread_id: meta.thread_id || null,
          label: payload.label as string | undefined,
          timezone: payload.timezone as string | undefined,
        };
        result = this.scheduler!.create(params);
        break;
      }
      case "fleet_schedule_list":
        result = this.scheduler!.list(payload.target as string | undefined);
        break;
      case "fleet_schedule_update":
        result = this.scheduler!.update(payload.id as string, payload as Record<string, unknown>);
        break;
      case "fleet_schedule_delete":
        this.scheduler!.delete(payload.id as string);
        result = "ok";
        break;
    }

    ipc.send({ type: "fleet_schedule_response", requestId, result });
  } catch (err) {
    ipc.send({ type: "fleet_schedule_response", requestId, error: (err as Error).message });
  }
}
```

- [ ] **Step 5: Hook into topic cleanup poller**

In the existing `handleTopicDeleted` method (or the topic cleanup poller logic), add:

```typescript
// After existing cleanup logic:
if (this.scheduler) {
  const instanceName = /* resolve threadId to instance name from routing table */;
  const count = this.scheduler.deleteByInstanceOrThread(instanceName, String(threadId));
  if (count > 0) {
    this.logger.info({ threadId, instanceName, count }, "Cleaned up schedules for deleted topic");
    this.adapter?.sendText(this.fleetConfig!.channel!.group_id!.toString(),
      `⚠️ Topic 已刪除，已清除 ${count} 條相關排程。`
    ).catch(() => {});
  }
}
```

- [ ] **Step 6: Add scheduler shutdown to `stopAll()`**

```typescript
// In stopAll() method:
this.scheduler?.shutdown();
```

- [ ] **Step 7: Verify build**

```bash
npm run build
```

Expected: No errors.

- [ ] **Step 8: Commit**

```bash
git add src/fleet-manager.ts
git commit -m "feat(scheduler): integrate Scheduler into Fleet Manager"
```

---

### Task 8: Daemon — route schedule tool calls to Fleet Manager

**Files:**
- Modify: `src/daemon.ts`

- [ ] **Step 1: Add schedule tool routing in `handleToolCall`**

In the `handleToolCall` method, before the existing tool switch/case, add detection for schedule tools:

```typescript
const SCHEDULE_TOOLS = new Set(["create_schedule", "list_schedules", "update_schedule", "delete_schedule"]);

if (SCHEDULE_TOOLS.has(tool)) {
  // Forward to fleet manager as schedule CRUD
  const scheduleType = `fleet_schedule_${tool.replace("_schedule", "").replace("_schedules", "_list")}`;
  // Normalize: create_schedule → fleet_schedule_create, list_schedules → fleet_schedule_list, etc.
  const typeMap: Record<string, string> = {
    create_schedule: "fleet_schedule_create",
    list_schedules: "fleet_schedule_list",
    update_schedule: "fleet_schedule_update",
    delete_schedule: "fleet_schedule_delete",
  };

  this.ipcServer?.broadcast({
    type: typeMap[tool],
    payload: args,
    meta: { chat_id: this.lastChatId, thread_id: this.lastThreadId, instance_name: this.name },
    requestId,
  });

  // Wait for fleet_schedule_response (reuse existing fleet_outbound_response pattern)
  const cleanup = () => {
    this.ipcServer?.removeListener("message", onResponse as (...a: unknown[]) => void);
    clearTimeout(timeout);
  };
  const onResponse = (respMsg: Record<string, unknown>) => {
    if (respMsg.type === "fleet_schedule_response" && respMsg.requestId === requestId) {
      cleanup();
      respond(respMsg.result, respMsg.error as string | undefined);
    }
  };
  const timeout = setTimeout(() => {
    cleanup();
    respond(null, "Schedule operation timed out after 30s");
  }, 30_000);
  this.ipcServer?.on("message", onResponse as (...a: unknown[]) => void);
  return;
}
```

- [ ] **Step 2: Handle `fleet_schedule_trigger` in IPC message handler**

In the IPC server message handler (around line 67), add:

```typescript
} else if (msg.type === "fleet_schedule_trigger") {
  const payload = msg.payload as Record<string, unknown>;
  const meta = msg.meta as Record<string, string>;
  this.lastChatId = meta.chat_id;
  this.lastThreadId = meta.thread_id;
  this.pushChannelMessage(payload.message as string, meta);
}
```

- [ ] **Step 3: Verify build**

```bash
npm run build
```

Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add src/daemon.ts
git commit -m "feat(scheduler): route schedule tools and triggers in daemon"
```

---

### Task 9: MCP Server — register schedule tools

**Files:**
- Modify: `src/channel/mcp-server.ts`

- [ ] **Step 1: Add schedule tools to ListToolsRequestSchema handler**

Add 4 new tools after the existing `download_attachment` tool definition:

```typescript
{
  name: "create_schedule",
  description: "Create a cron-based schedule. When triggered, sends a message to the target instance.",
  inputSchema: {
    type: "object",
    properties: {
      cron: { type: "string", description: "Cron expression, e.g. '0 7 * * *' (every day at 7 AM)" },
      message: { type: "string", description: "Message to inject when triggered" },
      target: { type: "string", description: "Target instance name. Defaults to this instance if omitted." },
      label: { type: "string", description: "Human-readable name for this schedule" },
      timezone: { type: "string", description: "IANA timezone, e.g. 'Asia/Taipei'. Defaults to Asia/Taipei." },
    },
    required: ["cron", "message"],
  },
},
{
  name: "list_schedules",
  description: "List all schedules. Optionally filter by target instance.",
  inputSchema: {
    type: "object",
    properties: {
      target: { type: "string", description: "Filter by target instance name" },
    },
  },
},
{
  name: "update_schedule",
  description: "Update an existing schedule. Only include fields you want to change.",
  inputSchema: {
    type: "object",
    properties: {
      id: { type: "string", description: "Schedule ID" },
      cron: { type: "string", description: "New cron expression" },
      message: { type: "string", description: "New message" },
      target: { type: "string", description: "New target instance" },
      label: { type: "string", description: "New label" },
      timezone: { type: "string", description: "New timezone" },
      enabled: { type: "boolean", description: "Enable/disable the schedule" },
    },
    required: ["id"],
  },
},
{
  name: "delete_schedule",
  description: "Delete a schedule by ID.",
  inputSchema: {
    type: "object",
    properties: {
      id: { type: "string", description: "Schedule ID to delete" },
    },
    required: ["id"],
  },
},
```

- [ ] **Step 2: Verify build**

No changes needed in `CallToolRequestSchema` handler — the existing generic `ipcRequest(req.params.name, args)` already forwards any tool name to the daemon. The daemon (Task 8) handles the routing.

Also update the `writeSettings()` method in `daemon.ts` to include schedule tools in the `permissions.allow` array, so Claude doesn't get prompted for each schedule tool call. Add these to the existing allow list:
- `mcp__ccd-channel__create_schedule`
- `mcp__ccd-channel__list_schedules`
- `mcp__ccd-channel__update_schedule`
- `mcp__ccd-channel__delete_schedule`

```bash
npm run build
```

Expected: No errors.

- [ ] **Step 3: Commit**

```bash
git add src/channel/mcp-server.ts
git commit -m "feat(scheduler): register schedule tools in MCP server"
```

---

### Task 10: CLI — `ccd schedule` commands

**Files:**
- Modify: `src/cli.ts`

- [ ] **Step 1: Import SchedulerDb**

```typescript
import { SchedulerDb } from "./scheduler/db.js";
import { Cron } from "croner";
```

- [ ] **Step 2: Add `schedule` command group**

After the existing `access` command group, add the full schedule command group. Use the same `commander` pattern as `topic` commands.

```typescript
const schedule = program.command("schedule").description("Manage scheduled tasks");

schedule
  .command("list")
  .option("--target <instance>", "Filter by target instance")
  .option("--json", "Output as JSON")
  .action((opts) => {
    const db = new SchedulerDb(join(DATA_DIR, "scheduler.db"));
    try {
      const schedules = db.list(opts.target);
      if (opts.json) {
        console.log(JSON.stringify(schedules, null, 2));
        return;
      }
      if (schedules.length === 0) {
        console.log("No schedules found.");
        return;
      }
      console.log("ID\t\t\t\t\tLabel\t\t\tCron\t\tTarget\tEnabled\tLast Status");
      for (const s of schedules) {
        console.log(`${s.id}\t${s.label ?? "-"}\t${s.cron}\t${s.target}\t${s.enabled ? "✅" : "❌"}\t${s.last_status ?? "-"}`);
      }
    } finally {
      db.close();
    }
  });

schedule
  .command("add")
  .requiredOption("--cron <expr>", "Cron expression")
  .requiredOption("--target <instance>", "Target instance")
  .requiredOption("--message <text>", "Message to send on trigger")
  .option("--label <text>", "Human-readable name")
  .option("--timezone <tz>", "IANA timezone", "Asia/Taipei")
  .action((opts) => {
    // Validate cron expression before writing to DB
    try { new Cron(opts.cron, { timezone: opts.timezone }); } catch (err) {
      console.error(`Invalid cron expression: ${(err as Error).message}`);
      process.exit(1);
    }
    const db = new SchedulerDb(join(DATA_DIR, "scheduler.db"));
    try {
      const s = db.create({
        cron: opts.cron,
        message: opts.message,
        source: opts.target, // CLI-created schedules use target as source
        target: opts.target,
        reply_chat_id: "", // CLI schedules have no reply target
        reply_thread_id: null,
        label: opts.label,
        timezone: opts.timezone,
      });
      console.log(`Created schedule ${s.id}`);
      signalFleetReload();
    } finally {
      db.close();
    }
  });

schedule
  .command("update")
  .argument("<id>", "Schedule ID")
  .option("--cron <expr>", "New cron expression")
  .option("--message <text>", "New message")
  .option("--target <instance>", "New target instance")
  .option("--label <text>", "New label")
  .option("--timezone <tz>", "New timezone")
  .option("--enabled <bool>", "Enable/disable (true/false)")
  .action((id, opts) => {
    const db = new SchedulerDb(join(DATA_DIR, "scheduler.db"));
    try {
      const params: Record<string, unknown> = {};
      if (opts.cron) params.cron = opts.cron;
      if (opts.message) params.message = opts.message;
      if (opts.target) params.target = opts.target;
      if (opts.label) params.label = opts.label;
      if (opts.timezone) params.timezone = opts.timezone;
      if (opts.enabled !== undefined) params.enabled = opts.enabled === "true";
      db.update(id, params);
      console.log(`Updated schedule ${id}`);
      signalFleetReload();
    } finally {
      db.close();
    }
  });

schedule
  .command("delete")
  .argument("<id>", "Schedule ID")
  .action((id) => {
    const db = new SchedulerDb(join(DATA_DIR, "scheduler.db"));
    try {
      db.delete(id);
      console.log(`Deleted schedule ${id}`);
      signalFleetReload();
    } finally {
      db.close();
    }
  });

schedule
  .command("enable")
  .argument("<id>", "Schedule ID")
  .action((id) => {
    const db = new SchedulerDb(join(DATA_DIR, "scheduler.db"));
    try {
      db.update(id, { enabled: true });
      console.log(`Enabled schedule ${id}`);
      signalFleetReload();
    } finally {
      db.close();
    }
  });

schedule
  .command("disable")
  .argument("<id>", "Schedule ID")
  .action((id) => {
    const db = new SchedulerDb(join(DATA_DIR, "scheduler.db"));
    try {
      db.update(id, { enabled: false });
      console.log(`Disabled schedule ${id}`);
      signalFleetReload();
    } finally {
      db.close();
    }
  });

schedule
  .command("history")
  .argument("<id>", "Schedule ID")
  .option("--limit <n>", "Number of runs to show", "20")
  .action((id, opts) => {
    const db = new SchedulerDb(join(DATA_DIR, "scheduler.db"));
    try {
      const runs = db.getRuns(id, parseInt(opts.limit, 10));
      if (runs.length === 0) {
        console.log("No runs found.");
        return;
      }
      console.log("Time\t\t\tStatus\t\t\tDetail");
      for (const r of runs) {
        console.log(`${r.triggered_at}\t${r.status}\t${r.detail ?? ""}`);
      }
    } finally {
      db.close();
    }
  });

schedule
  .command("trigger")
  .argument("<id>", "Schedule ID")
  .action((id) => {
    // Manual trigger needs fleet manager running — send via IPC
    console.log("Manual trigger requires fleet manager. Use SIGHUP workaround or call from Telegram.");
    // For v1, this is a limitation. Future: connect to fleet manager IPC.
  });
```

- [ ] **Step 3: Add `signalFleetReload` helper**

```typescript
function signalFleetReload(): void {
  const pidPath = join(DATA_DIR, "fleet.pid");
  try {
    const pid = parseInt(readFileSync(pidPath, "utf-8").trim(), 10);
    process.kill(pid, "SIGHUP");
    console.log("Fleet manager notified to reload schedules.");
  } catch {
    console.log("Fleet manager not running. Schedules will be loaded on next start.");
  }
}
```

- [ ] **Step 4: Verify build**

```bash
npm run build
```

Expected: No errors.

- [ ] **Step 5: Commit**

```bash
git add src/cli.ts
git commit -m "feat(scheduler): add ccd schedule CLI commands"
```

---

### Task 11: Run full test suite and verify build

**Files:** None (validation only)

- [ ] **Step 1: Run all tests**

```bash
npx vitest run
```

Expected: All tests pass (db.test.ts and scheduler.test.ts).

- [ ] **Step 2: Full build**

```bash
npm run build
```

Expected: No errors.

- [ ] **Step 3: Manual smoke test**

```bash
# Start fleet
ccd fleet start

# In another terminal, add a schedule via CLI
ccd schedule add --cron "*/1 * * * *" --target <instance-name> --message "Test schedule: 你好！"
ccd schedule list

# Watch the instance's topic — within 1 minute, "[排程任務] Test schedule: 你好！" should appear
# Clean up
ccd schedule delete <schedule-id>
```

- [ ] **Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix(scheduler): address issues found during smoke test"
```
