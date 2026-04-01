import Database from "better-sqlite3";
import { randomUUID } from "node:crypto";
import type { Schedule, ScheduleRun, CreateScheduleParams, UpdateScheduleParams, Decision, CreateDecisionParams, UpdateDecisionParams, Task, CreateTaskParams, UpdateTaskParams } from "./types.js";

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
      CREATE TABLE IF NOT EXISTS decisions (
        id            TEXT PRIMARY KEY,
        project_root  TEXT NOT NULL,
        scope         TEXT NOT NULL DEFAULT 'project',
        title         TEXT NOT NULL,
        content       TEXT NOT NULL,
        tags          TEXT,
        status        TEXT NOT NULL DEFAULT 'active',
        superseded_by TEXT,
        created_by    TEXT NOT NULL,
        created_at    TEXT NOT NULL,
        expires_at    TEXT,
        updated_at    TEXT NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_decisions_project ON decisions(project_root);
      CREATE INDEX IF NOT EXISTS idx_decisions_status ON decisions(status);
    `);

    // Migration: add scope column to existing decisions tables that lack it
    const cols = this.db.prepare("PRAGMA table_info(decisions)").all() as { name: string }[];
    if (cols.length > 0 && !cols.some(c => c.name === "scope")) {
      this.db.exec("ALTER TABLE decisions ADD COLUMN scope TEXT NOT NULL DEFAULT 'project'");
    }
    this.db.exec("CREATE INDEX IF NOT EXISTS idx_decisions_scope ON decisions(scope)");

    // Tasks table
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS tasks (
        id          TEXT PRIMARY KEY,
        title       TEXT NOT NULL,
        description TEXT,
        status      TEXT NOT NULL DEFAULT 'open',
        priority    TEXT NOT NULL DEFAULT 'normal',
        assignee    TEXT,
        created_by  TEXT NOT NULL,
        depends_on  TEXT,
        result      TEXT,
        created_at  TEXT NOT NULL,
        updated_at  TEXT NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
      CREATE INDEX IF NOT EXISTS idx_tasks_assignee ON tasks(assignee);
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
    const existing = this.get(id);
    if (!existing) throw new Error(`Schedule "${id}" not found`);

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

    return this.get(id) ?? existing;
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
    return this.db.prepare("SELECT * FROM schedule_runs WHERE schedule_id = ? ORDER BY triggered_at DESC, id DESC LIMIT ?").all(scheduleId, limit) as ScheduleRun[];
  }

  pruneOldRuns(days = 30): void {
    this.db.prepare("DELETE FROM schedule_runs WHERE triggered_at < datetime('now', '-' || ? || ' days')").run(days);
  }

  // ── Decisions ───────────────────────────────────────────────

  private rowToDecision(row: Record<string, unknown>): Decision {
    return {
      id: row.id as string,
      project_root: row.project_root as string,
      scope: (row.scope as Decision["scope"]) ?? "project",
      title: row.title as string,
      content: row.content as string,
      tags: row.tags ? JSON.parse(row.tags as string) : [],
      status: row.status as Decision["status"],
      superseded_by: row.superseded_by as string | null,
      created_by: row.created_by as string,
      created_at: row.created_at as string,
      expires_at: row.expires_at as string | null,
      updated_at: row.updated_at as string,
    };
  }

  createDecision(params: CreateDecisionParams): Decision {
    const id = randomUUID();
    const now = new Date().toISOString();
    const ttl = params.ttl_days ?? 0; // default: permanent
    const expiresAt = ttl > 0 ? new Date(Date.now() + ttl * 86_400_000).toISOString() : null;

    const scope = params.scope ?? "project";
    this.db.prepare(`
      INSERT INTO decisions (id, project_root, scope, title, content, tags, created_by, created_at, expires_at, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `).run(id, params.project_root, scope, params.title, params.content, params.tags?.length ? JSON.stringify(params.tags) : null, params.created_by, now, expiresAt, now);

    if (params.supersedes) {
      this.supersedeDecision(params.supersedes, id);
    }

    return this.getDecision(id)!;
  }

  getDecision(id: string): Decision | null {
    const row = this.db.prepare("SELECT * FROM decisions WHERE id = ?").get(id) as Record<string, unknown> | undefined;
    return row ? this.rowToDecision(row) : null;
  }

  listDecisions(projectRoot: string, opts?: { includeArchived?: boolean; tags?: string[] }): Decision[] {
    // Return project-scoped decisions for this root + all fleet-scoped decisions
    let sql = "SELECT * FROM decisions WHERE ((project_root = ? AND scope = 'project') OR scope = 'fleet')";
    const values: unknown[] = [projectRoot];

    if (!opts?.includeArchived) {
      sql += " AND status = 'active'";
    }

    // Fleet decisions first, then project, newest first within each group
    sql += " ORDER BY CASE scope WHEN 'fleet' THEN 0 ELSE 1 END, created_at DESC";
    let rows = this.db.prepare(sql).all(...values) as Record<string, unknown>[];

    if (opts?.tags?.length) {
      rows = rows.filter(r => {
        const tags = r.tags ? JSON.parse(r.tags as string) as string[] : [];
        return opts.tags!.some(t => tags.includes(t));
      });
    }

    return rows.map(r => this.rowToDecision(r));
  }

  updateDecision(id: string, params: UpdateDecisionParams): Decision {
    const existing = this.getDecision(id);
    if (!existing) throw new Error(`Decision "${id}" not found`);

    const sets: string[] = ["updated_at = ?"];
    const values: unknown[] = [new Date().toISOString()];

    if (params.content !== undefined) { sets.push("content = ?"); values.push(params.content); }
    if (params.tags !== undefined) { sets.push("tags = ?"); values.push(JSON.stringify(params.tags)); }
    if (params.ttl_days !== undefined) {
      const expiresAt = params.ttl_days > 0 ? new Date(Date.now() + params.ttl_days * 86_400_000).toISOString() : null;
      sets.push("expires_at = ?"); values.push(expiresAt);
    }

    values.push(id);
    this.db.prepare(`UPDATE decisions SET ${sets.join(", ")} WHERE id = ?`).run(...values);
    return this.getDecision(id)!;
  }

  archiveDecision(id: string): void {
    this.db.prepare("UPDATE decisions SET status = 'archived', updated_at = ? WHERE id = ?").run(new Date().toISOString(), id);
  }

  supersedeDecision(oldId: string, newId: string): void {
    this.db.prepare("UPDATE decisions SET status = 'superseded', superseded_by = ?, updated_at = ? WHERE id = ?")
      .run(newId, new Date().toISOString(), oldId);
  }

  pruneExpiredDecisions(): number {
    const result = this.db.prepare("UPDATE decisions SET status = 'archived', updated_at = ? WHERE status = 'active' AND expires_at IS NOT NULL AND expires_at < ?")
      .run(new Date().toISOString(), new Date().toISOString());
    return result.changes;
  }

  // ── Tasks ──────────────────────────────────────────────────

  private rowToTask(row: Record<string, unknown>): Task {
    return {
      id: row.id as string,
      title: row.title as string,
      description: row.description as string | null,
      status: row.status as Task["status"],
      priority: row.priority as Task["priority"],
      assignee: row.assignee as string | null,
      created_by: row.created_by as string,
      depends_on: row.depends_on ? JSON.parse(row.depends_on as string) : [],
      result: row.result as string | null,
      created_at: row.created_at as string,
      updated_at: row.updated_at as string,
    };
  }

  createTask(params: CreateTaskParams): Task {
    const id = randomUUID();
    const now = new Date().toISOString();
    this.db.prepare(`
      INSERT INTO tasks (id, title, description, priority, assignee, created_by, depends_on, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
    `).run(id, params.title, params.description ?? null, params.priority ?? "normal",
      params.assignee ?? null, params.created_by,
      params.depends_on?.length ? JSON.stringify(params.depends_on) : null, now, now);
    return this.getTask(id)!;
  }

  getTask(id: string): Task | null {
    const row = this.db.prepare("SELECT * FROM tasks WHERE id = ?").get(id) as Record<string, unknown> | undefined;
    return row ? this.rowToTask(row) : null;
  }

  listTasks(opts?: { assignee?: string; status?: string }): Task[] {
    let sql = "SELECT * FROM tasks WHERE 1=1";
    const values: unknown[] = [];
    if (opts?.assignee) { sql += " AND assignee = ?"; values.push(opts.assignee); }
    if (opts?.status) { sql += " AND status = ?"; values.push(opts.status); }
    sql += " ORDER BY CASE priority WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'normal' THEN 2 ELSE 3 END, created_at";
    return (this.db.prepare(sql).all(...values) as Record<string, unknown>[]).map(r => this.rowToTask(r));
  }

  updateTask(id: string, params: UpdateTaskParams): Task {
    const existing = this.getTask(id);
    if (!existing) throw new Error(`Task "${id}" not found`);
    const sets: string[] = ["updated_at = ?"];
    const values: unknown[] = [new Date().toISOString()];
    if (params.status !== undefined) { sets.push("status = ?"); values.push(params.status); }
    if (params.assignee !== undefined) { sets.push("assignee = ?"); values.push(params.assignee); }
    if (params.result !== undefined) { sets.push("result = ?"); values.push(params.result); }
    if (params.priority !== undefined) { sets.push("priority = ?"); values.push(params.priority); }
    values.push(id);
    this.db.prepare(`UPDATE tasks SET ${sets.join(", ")} WHERE id = ?`).run(...values);
    return this.getTask(id)!;
  }

  claimTask(id: string, assignee: string): Task {
    const task = this.getTask(id);
    if (!task) throw new Error(`Task "${id}" not found`);
    if (task.status !== "open") throw new Error(`Task "${id}" is ${task.status}, cannot claim`);
    // Check dependencies
    if (task.depends_on.length > 0) {
      for (const depId of task.depends_on) {
        const dep = this.getTask(depId);
        if (dep && dep.status !== "done") {
          throw new Error(`Task "${id}" is blocked by "${depId}" (status: ${dep?.status ?? "not found"})`);
        }
      }
    }
    return this.updateTask(id, { status: "claimed", assignee });
  }

  completeTask(id: string, result?: string): Task {
    const task = this.getTask(id);
    if (!task) throw new Error(`Task "${id}" not found`);
    if (task.status !== "claimed") throw new Error(`Task "${id}" is ${task.status}, cannot complete (must be claimed first)`);
    return this.updateTask(id, { status: "done", result: result ?? undefined });
  }

  close(): void {
    this.db.close();
  }
}
