import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdirSync, rmSync } from "node:fs";
import { SchedulerDb } from "../src/scheduler/db.js";

describe("SchedulerDb — Tasks", () => {
  let tmpDir: string;
  let db: SchedulerDb;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `tasks-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    db = new SchedulerDb(join(tmpDir, "scheduler.db"));
  });

  afterEach(() => {
    db.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("creates and retrieves a task", () => {
    const t = db.createTask({ title: "Fix bug", created_by: "general" });
    expect(t.id).toBeTruthy();
    expect(t.title).toBe("Fix bug");
    expect(t.status).toBe("open");
    expect(t.priority).toBe("normal");
    expect(t.assignee).toBeNull();
    expect(t.depends_on).toEqual([]);

    const retrieved = db.getTask(t.id);
    expect(retrieved).toEqual(t);
  });

  it("creates task with all fields", () => {
    const t = db.createTask({
      title: "Review PR",
      description: "Check auth module",
      priority: "high",
      assignee: "worker-1",
      depends_on: ["task-a", "task-b"],
      created_by: "general",
    });
    expect(t.description).toBe("Check auth module");
    expect(t.priority).toBe("high");
    expect(t.assignee).toBe("worker-1");
    expect(t.depends_on).toEqual(["task-a", "task-b"]);
  });

  it("lists tasks ordered by priority", () => {
    db.createTask({ title: "Low", priority: "low", created_by: "x" });
    db.createTask({ title: "Urgent", priority: "urgent", created_by: "x" });
    db.createTask({ title: "Normal", created_by: "x" });

    const tasks = db.listTasks();
    expect(tasks[0].title).toBe("Urgent");
    expect(tasks[1].title).toBe("Normal");
    expect(tasks[2].title).toBe("Low");
  });

  it("filters by assignee and status", () => {
    db.createTask({ title: "A", assignee: "worker-1", created_by: "x" });
    db.createTask({ title: "B", assignee: "worker-2", created_by: "x" });

    expect(db.listTasks({ assignee: "worker-1" })).toHaveLength(1);
    expect(db.listTasks({ status: "open" })).toHaveLength(2);
    expect(db.listTasks({ status: "done" })).toHaveLength(0);
  });

  it("claims an open task", () => {
    const t = db.createTask({ title: "Do thing", created_by: "general" });
    const claimed = db.claimTask(t.id, "worker-1");
    expect(claimed.status).toBe("claimed");
    expect(claimed.assignee).toBe("worker-1");
  });

  it("rejects claim on non-open task", () => {
    const t = db.createTask({ title: "Do thing", created_by: "general" });
    db.claimTask(t.id, "worker-1");
    expect(() => db.claimTask(t.id, "worker-2")).toThrow(/claimed/);
  });

  it("blocks claim when dependency is not done", () => {
    const dep = db.createTask({ title: "Dep", created_by: "x" });
    const t = db.createTask({ title: "Main", depends_on: [dep.id], created_by: "x" });
    expect(() => db.claimTask(t.id, "worker")).toThrow(/blocked/i);
  });

  it("allows claim after dependency is done", () => {
    const dep = db.createTask({ title: "Dep", created_by: "x" });
    db.claimTask(dep.id, "w");
    db.completeTask(dep.id, "done");

    const t = db.createTask({ title: "Main", depends_on: [dep.id], created_by: "x" });
    const claimed = db.claimTask(t.id, "worker");
    expect(claimed.status).toBe("claimed");
  });

  it("completes a claimed task", () => {
    const t = db.createTask({ title: "Do thing", created_by: "x" });
    db.claimTask(t.id, "worker");
    const done = db.completeTask(t.id, "All done");
    expect(done.status).toBe("done");
    expect(done.result).toBe("All done");
  });

  it("rejects complete on non-claimed task", () => {
    const t = db.createTask({ title: "Do thing", created_by: "x" });
    expect(() => db.completeTask(t.id)).toThrow(/must be claimed/);
  });

  it("updates task fields", () => {
    const t = db.createTask({ title: "Task", created_by: "x" });
    const updated = db.updateTask(t.id, { priority: "urgent", assignee: "worker" });
    expect(updated.priority).toBe("urgent");
    expect(updated.assignee).toBe("worker");
  });

  it("throws on update non-existent task", () => {
    expect(() => db.updateTask("fake", { status: "done" })).toThrow(/not found/i);
  });
});
