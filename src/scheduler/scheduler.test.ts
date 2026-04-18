import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { Scheduler } from "./scheduler.js";
import type { Schedule } from "./types.js";
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
      (instanceName: string) => true,
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
      (name: string) => name === "proj-a",
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

  it("delete removes schedule and cron job", () => {
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
    expect(scheduler.list()).toHaveLength(1);
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

  it("P2.3: catches up a missed fire on init when within window", async () => {
    // Create schedule via first instance, seed last_triggered_at to be
    // older than the most recent expected fire.
    const s = scheduler.create({
      cron: "*/5 * * * *",           // every 5 minutes
      message: "catchup",
      source: "proj-a",
      target: "proj-a",
      reply_chat_id: "1",
      reply_thread_id: null,
    });
    // Directly poke the db to simulate "daemon was down for 10 minutes after
    // the last trigger" — last_triggered 20 min ago; previous expected fire
    // ≤5 min ago, which is strictly newer than last_triggered.
    const twentyMinAgo = new Date(Date.now() - 20 * 60_000).toISOString();
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ((scheduler.db as unknown) as { db: import("better-sqlite3").Database })
      .db.prepare("UPDATE schedules SET last_triggered_at = ? WHERE id = ?")
      .run(twentyMinAgo, s.id);
    scheduler.shutdown();

    // Reopen scheduler — init() should catch up.
    triggered.length = 0;
    scheduler = new Scheduler(
      join(dir, "scheduler.db"),
      (schedule) => { triggered.push(schedule); },
      DEFAULT_SCHEDULER_CONFIG,
      () => true,
    );
    scheduler.init();
    // setImmediate — wait a tick for the catch-up to fire.
    await new Promise((r) => setImmediate(r));
    expect(triggered.map((t) => t.id)).toContain(s.id);
  });

  it("P2.3: does not catch up outside window", async () => {
    const s = scheduler.create({
      cron: "*/5 * * * *",
      message: "stale",
      source: "proj-a",
      target: "proj-a",
      reply_chat_id: "1",
      reply_thread_id: null,
    });
    const oneHourAgo = new Date(Date.now() - 60 * 60_000).toISOString();
    ((scheduler.db as unknown) as { db: import("better-sqlite3").Database })
      .db.prepare("UPDATE schedules SET last_triggered_at = ? WHERE id = ?")
      .run(oneHourAgo, s.id);
    scheduler.shutdown();

    // Re-open with a 0.001-min window — any prev fire older than 60ms skips.
    triggered.length = 0;
    scheduler = new Scheduler(
      join(dir, "scheduler.db"),
      (schedule) => { triggered.push(schedule); },
      { ...DEFAULT_SCHEDULER_CONFIG, catchup_window_minutes: 0.001 },
      () => true,
    );
    scheduler.init();
    await new Promise((r) => setImmediate(r));
    expect(triggered.map((t) => t.id)).not.toContain(s.id);
  });

  it("P2.3: catchup_window_minutes=0 disables catch-up entirely", async () => {
    const s = scheduler.create({
      cron: "*/5 * * * *",
      message: "disabled-catchup",
      source: "proj-a",
      target: "proj-a",
      reply_chat_id: "1",
      reply_thread_id: null,
    });
    const twentyMinAgo = new Date(Date.now() - 20 * 60_000).toISOString();
    ((scheduler.db as unknown) as { db: import("better-sqlite3").Database })
      .db.prepare("UPDATE schedules SET last_triggered_at = ? WHERE id = ?")
      .run(twentyMinAgo, s.id);
    scheduler.shutdown();

    triggered.length = 0;
    scheduler = new Scheduler(
      join(dir, "scheduler.db"),
      (schedule) => { triggered.push(schedule); },
      { ...DEFAULT_SCHEDULER_CONFIG, catchup_window_minutes: 0 },
      () => true,
    );
    scheduler.init();
    await new Promise((r) => setImmediate(r));
    expect(triggered.map((t) => t.id)).not.toContain(s.id);
  });

  it("P2.3: does not catch up a fresh schedule with no prior trigger", async () => {
    // Create and immediately restart — schedule has null last_triggered_at.
    scheduler.create({
      cron: "*/5 * * * *",
      message: "fresh",
      source: "proj-a",
      target: "proj-a",
      reply_chat_id: "1",
      reply_thread_id: null,
    });
    scheduler.shutdown();

    triggered.length = 0;
    scheduler = new Scheduler(
      join(dir, "scheduler.db"),
      (schedule) => { triggered.push(schedule); },
      DEFAULT_SCHEDULER_CONFIG,
      () => true,
    );
    scheduler.init();
    await new Promise((r) => setImmediate(r));
    expect(triggered).toHaveLength(0);
  });
});
