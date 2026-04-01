import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdirSync, rmSync } from "node:fs";
import { SchedulerDb } from "../src/scheduler/db.js";

describe("SchedulerDb — Decisions", () => {
  let tmpDir: string;
  let db: SchedulerDb;

  beforeEach(() => {
    tmpDir = join(tmpdir(), `decisions-test-${Date.now()}`);
    mkdirSync(tmpDir, { recursive: true });
    db = new SchedulerDb(join(tmpDir, "scheduler.db"));
  });

  afterEach(() => {
    db.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("creates and retrieves a decision", () => {
    const d = db.createDecision({
      project_root: "/projects/web",
      title: "Use TypeScript strict mode",
      content: "All new files must use strict: true in tsconfig",
      created_by: "web-agent",
    });
    expect(d.id).toBeTruthy();
    expect(d.title).toBe("Use TypeScript strict mode");
    expect(d.status).toBe("active");
    expect(d.created_by).toBe("web-agent");
    expect(d.expires_at).toBeNull(); // default = permanent (no expiry)

    const retrieved = db.getDecision(d.id);
    expect(retrieved).toEqual(d);
  });

  it("lists decisions by project_root", () => {
    db.createDecision({ project_root: "/a", title: "A1", content: "c", created_by: "x" });
    db.createDecision({ project_root: "/a", title: "A2", content: "c", created_by: "x" });
    db.createDecision({ project_root: "/b", title: "B1", content: "c", created_by: "x" });

    const aDecisions = db.listDecisions("/a");
    expect(aDecisions).toHaveLength(2);
    const bDecisions = db.listDecisions("/b");
    expect(bDecisions).toHaveLength(1);
  });

  it("filters by tags", () => {
    db.createDecision({ project_root: "/p", title: "T1", content: "c", tags: ["arch", "db"], created_by: "x" });
    db.createDecision({ project_root: "/p", title: "T2", content: "c", tags: ["style"], created_by: "x" });
    db.createDecision({ project_root: "/p", title: "T3", content: "c", created_by: "x" }); // no tags

    const archDecisions = db.listDecisions("/p", { tags: ["arch"] });
    expect(archDecisions).toHaveLength(1);
    expect(archDecisions[0].title).toBe("T1");
  });

  it("updates a decision", () => {
    const d = db.createDecision({ project_root: "/p", title: "Old", content: "old content", created_by: "x" });
    const updated = db.updateDecision(d.id, { content: "new content", tags: ["updated"] });
    expect(updated.content).toBe("new content");
    expect(updated.tags).toEqual(["updated"]);
  });

  it("archives a decision", () => {
    const d = db.createDecision({ project_root: "/p", title: "Temp", content: "c", created_by: "x" });
    db.archiveDecision(d.id);
    const archived = db.getDecision(d.id);
    expect(archived?.status).toBe("archived");

    // Not visible in default list
    const active = db.listDecisions("/p");
    expect(active).toHaveLength(0);

    // Visible with includeArchived
    const all = db.listDecisions("/p", { includeArchived: true });
    expect(all).toHaveLength(1);
  });

  it("supersedes a decision", () => {
    const old = db.createDecision({ project_root: "/p", title: "V1", content: "old", created_by: "x" });
    const newer = db.createDecision({ project_root: "/p", title: "V2", content: "new", created_by: "x", supersedes: old.id });

    const oldAfter = db.getDecision(old.id);
    expect(oldAfter?.status).toBe("superseded");
    expect(oldAfter?.superseded_by).toBe(newer.id);

    // Only V2 is active
    const active = db.listDecisions("/p");
    expect(active).toHaveLength(1);
    expect(active[0].title).toBe("V2");
  });

  it("creates expiring decision with explicit ttl_days", () => {
    const d = db.createDecision({ project_root: "/p", title: "Temp", content: "c", created_by: "x", ttl_days: 7 });
    expect(d.expires_at).toBeTruthy();
  });

  it("prunes expired decisions", () => {
    // Create a decision with expires_at in the past
    const d = db.createDecision({ project_root: "/p", title: "Expired", content: "c", created_by: "x", ttl_days: 1 });
    // Manually set expires_at to past
    const pastDate = new Date(Date.now() - 86_400_000).toISOString();
    db["db"].prepare("UPDATE decisions SET expires_at = ? WHERE id = ?").run(pastDate, d.id);

    const pruned = db.pruneExpiredDecisions();
    expect(pruned).toBe(1);

    const after = db.getDecision(d.id);
    expect(after?.status).toBe("archived");
  });

  it("throws on update non-existent decision", () => {
    expect(() => db.updateDecision("fake-id", { content: "x" })).toThrow(/not found/i);
  });
});
