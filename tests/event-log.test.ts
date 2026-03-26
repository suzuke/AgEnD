import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { EventLog } from "../src/event-log.js";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdirSync, rmSync } from "node:fs";

describe("EventLog", () => {
  let tmpDir: string;
  let log: EventLog;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `ccd-event-log-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    log = new EventLog(join(tmpDir, "events.db"));
  });

  afterEach(() => {
    log.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("creates table on init", () => {
    const rows = log.query();
    expect(rows).toEqual([]);
  });

  it("inserts and retrieves an event", () => {
    log.insert("worker-1", "task.start");
    const rows = log.query();
    expect(rows).toHaveLength(1);
    expect(rows[0].instance_name).toBe("worker-1");
    expect(rows[0].event_type).toBe("task.start");
    expect(rows[0].payload).toBeNull();
    expect(rows[0].created_at).toBeTruthy();
    expect(rows[0].id).toBe(1);
  });

  it("parses payload JSON", () => {
    log.insert("worker-1", "task.done", { result: "ok", tokens: 42 });
    const rows = log.query();
    expect(rows[0].payload).toEqual({ result: "ok", tokens: 42 });
  });

  it("filters by instance", () => {
    log.insert("worker-1", "task.start");
    log.insert("worker-2", "task.start");
    log.insert("worker-1", "task.done");

    const rows = log.query({ instance: "worker-1" });
    expect(rows).toHaveLength(2);
    expect(rows.every((r) => r.instance_name === "worker-1")).toBe(true);
  });

  it("filters by event type", () => {
    log.insert("worker-1", "task.start");
    log.insert("worker-2", "task.start");
    log.insert("worker-1", "task.done");

    const rows = log.query({ type: "task.start" });
    expect(rows).toHaveLength(2);
    expect(rows.every((r) => r.event_type === "task.start")).toBe(true);
  });

  it("filters by since date", () => {
    // Insert a past event by manipulating created_at directly
    const db = (log as unknown as { db: import("better-sqlite3").Database }).db;
    db.prepare("INSERT INTO events (instance_name, event_type, created_at) VALUES (?, ?, ?)").run(
      "worker-1",
      "old.event",
      "2000-01-01 00:00:00",
    );
    log.insert("worker-1", "new.event");

    const rows = log.query({ since: "2024-01-01 00:00:00" });
    expect(rows.every((r) => r.event_type !== "old.event")).toBe(true);
    expect(rows.some((r) => r.event_type === "new.event")).toBe(true);
  });

  it("respects limit", () => {
    for (let i = 0; i < 10; i++) {
      log.insert("worker-1", "task.start");
    }
    const rows = log.query({ limit: 3 });
    expect(rows).toHaveLength(3);
  });

  it("prunes old events", () => {
    const db = (log as unknown as { db: import("better-sqlite3").Database }).db;
    db.prepare("INSERT INTO events (instance_name, event_type, created_at) VALUES (?, ?, ?)").run(
      "worker-1",
      "old.event",
      "2000-01-01 00:00:00",
    );
    log.insert("worker-1", "new.event");

    log.prune(30);

    const rows = log.query({ limit: 100 });
    expect(rows.every((r) => r.event_type !== "old.event")).toBe(true);
    expect(rows.some((r) => r.event_type === "new.event")).toBe(true);
  });
});
