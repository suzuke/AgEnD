//! `Task` <-> `tasks` row, and `StoredEvent` <-> `task_events` row (gate 5
//! P3). Both directions name every field of `Task` (a full destructure and a
//! full struct literal), so a field added in core stops this file from
//! compiling until it gets a column and a migration.
//!
//! Must NOT: use `..` in a `Task` pattern or literal.

use agend_core::pipeline::task::{Task, TaskStatus};
use agend_core::traits::{CasResult, StoredEvent, VersionedTask};
use rusqlite::{Connection, OptionalExtension, Row, ToSql};

use super::{FOREIGN_KEY, PRIMARY_KEY, StoreError, is_constraint};

/// Version of a newly created task (as `FakeStore`, #1483).
pub(super) const FIRST_VERSION: i64 = 1;

const COLUMNS: &str = "id, title, team_id, workflow_id, workflow_version, parent, depends_on, \
                       superseded_by, assignee, status, requires_repo, merge_commit, version";

pub(super) fn to_i64(value: u64, what: &str) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Invalid(format!("{what} {value} is too large")))
}

fn to_u64(value: i64, what: &str) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Invalid(format!("{what} {value} is negative")))
}

/// The `tasks.status` text of `status` (also the client protocol's).
pub fn status_text(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Open => "open",
        TaskStatus::Running => "running",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Done => "done",
        TaskStatus::Superseded => "superseded",
    }
}

fn parse_status(text: &str) -> Result<TaskStatus, StoreError> {
    Ok(match text {
        "open" => TaskStatus::Open,
        "running" => TaskStatus::Running,
        "blocked" => TaskStatus::Blocked,
        "done" => TaskStatus::Done,
        "superseded" => TaskStatus::Superseded,
        other => return Err(StoreError::Invalid(format!("task status {other:?}"))),
    })
}

/// Writes every column of `task` with `version`: `sql` is the INSERT or the
/// CAS UPDATE (which also names `:expected`), both using the same named
/// parameters.
fn write(
    conn: &Connection,
    sql: &str,
    task: &Task,
    version: i64,
    expected: Option<i64>,
) -> Result<usize, StoreError> {
    let Task {
        id,
        title,
        team_id,
        workflow_id,
        workflow_version,
        parent,
        depends_on,
        superseded_by,
        assignee,
        status,
        requires_repo,
        merge_commit,
    } = task;
    let depends_on = serde_json::to_string(depends_on)
        .map_err(|e| StoreError::Invalid(format!("depends_on of {id}: {e}")))?;
    let workflow_version = to_i64(*workflow_version, "workflow version")?;
    let status = status_text(*status);
    let mut params: Vec<(&str, &dyn ToSql)> = vec![
        (":id", id),
        (":title", title),
        (":team_id", team_id),
        (":workflow_id", workflow_id),
        (":workflow_version", &workflow_version),
        (":parent", parent),
        (":depends_on", &depends_on),
        (":superseded_by", superseded_by),
        (":assignee", assignee),
        (":status", &status),
        (":requires_repo", requires_repo),
        (":merge_commit", merge_commit),
        (":version", &version),
    ];
    if let Some(expected) = &expected {
        params.push((":expected", expected));
    }
    Ok(conn.execute(sql, params.as_slice())?)
}

fn read(row: &Row<'_>) -> Result<VersionedTask, StoreError> {
    let depends_on: String = row.get("depends_on")?;
    let depends_on: Vec<String> = serde_json::from_str(&depends_on)
        .map_err(|e| StoreError::Invalid(format!("depends_on {depends_on:?}: {e}")))?;
    let status: String = row.get("status")?;
    let task = Task {
        id: row.get("id")?,
        title: row.get("title")?,
        team_id: row.get("team_id")?,
        workflow_id: row.get("workflow_id")?,
        workflow_version: to_u64(row.get("workflow_version")?, "workflow version")?,
        parent: row.get("parent")?,
        depends_on,
        superseded_by: row.get("superseded_by")?,
        assignee: row.get("assignee")?,
        status: parse_status(&status)?,
        requires_repo: row.get("requires_repo")?,
        merge_commit: row.get("merge_commit")?,
    };
    Ok(VersionedTask {
        version: to_u64(row.get("version")?, "task version")?,
        task,
    })
}

pub(super) fn load(conn: &Connection, task_id: &str) -> Result<Option<VersionedTask>, StoreError> {
    let mut stmt = conn.prepare_cached(&format!("SELECT {COLUMNS} FROM tasks WHERE id = ?1"))?;
    let mut rows = stmt.query([task_id])?;
    rows.next()?.map(read).transpose()
}

/// Every task, by id.
pub(super) fn list(conn: &Connection) -> Result<Vec<Task>, StoreError> {
    let mut stmt = conn.prepare_cached(&format!("SELECT {COLUMNS} FROM tasks ORDER BY id"))?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(read(row)?.task);
    }
    Ok(out)
}

pub(super) fn insert(conn: &Connection, task: &Task) -> Result<(), StoreError> {
    let sql = format!(
        "INSERT INTO tasks ({COLUMNS}) VALUES (:id, :title, :team_id, :workflow_id, \
         :workflow_version, :parent, :depends_on, :superseded_by, :assignee, :status, \
         :requires_repo, :merge_commit, :version)"
    );
    match write(conn, &sql, task, FIRST_VERSION, None) {
        Err(StoreError::Sqlite(e)) if is_constraint(&e, PRIMARY_KEY) => {
            Err(StoreError::Exists(format!("task {}", task.id)))
        }
        other => other.map(|_| ()),
    }
}

/// `UPDATE ... WHERE id = ? AND version = ?`: writes only when the stored
/// version equals `expected_version`; the version then grows by 1.
pub(super) fn compare_and_swap(
    conn: &Connection,
    task: &Task,
    expected_version: u64,
) -> Result<CasResult, StoreError> {
    // A version this large was never issued, so it cannot match.
    if let Ok(expected) = i64::try_from(expected_version) {
        let sql = "UPDATE tasks SET title = :title, team_id = :team_id, \
                   workflow_id = :workflow_id, workflow_version = :workflow_version, \
                   parent = :parent, depends_on = :depends_on, superseded_by = :superseded_by, \
                   assignee = :assignee, status = :status, requires_repo = :requires_repo, \
                   merge_commit = :merge_commit, version = :version \
                   WHERE id = :id AND version = :expected";
        let next = expected
            .checked_add(1)
            .ok_or_else(|| StoreError::Invalid("task version overflow".into()))?;
        if write(conn, sql, task, next, Some(expected))? == 1 {
            return Ok(CasResult::Written {
                new_version: to_u64(next, "task version")?,
            });
        }
    }
    let current: Option<i64> = conn
        .query_row("SELECT version FROM tasks WHERE id = ?1", [&task.id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(CasResult::Conflict {
        current_version: current.map(|v| to_u64(v, "task version")).transpose()?,
    })
}

pub(super) fn append_event(
    conn: &Connection,
    task_id: &str,
    event: &StoredEvent,
) -> Result<(), StoreError> {
    let StoredEvent {
        id,
        occurred_at_unix_ms,
        kind,
        detail,
    } = event;
    let inserted = conn.execute(
        "INSERT INTO task_events (task_id, event_id, occurred_at_unix_ms, kind, detail) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            task_id,
            id,
            to_i64(*occurred_at_unix_ms, "event time")?,
            kind,
            detail
        ],
    );
    match inserted {
        Ok(_) => Ok(()),
        Err(e) if is_constraint(&e, FOREIGN_KEY) => Err(StoreError::UnknownTask(task_id.into())),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn load_events(
    conn: &Connection,
    task_id: &str,
) -> Result<Vec<StoredEvent>, StoreError> {
    let mut stmt = conn.prepare_cached(
        "SELECT event_id, occurred_at_unix_ms, kind, detail FROM task_events \
         WHERE task_id = ?1 ORDER BY seq",
    )?;
    let mut rows = stmt.query([task_id])?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        events.push(StoredEvent {
            id: row.get(0)?,
            occurred_at_unix_ms: to_u64(row.get(1)?, "event time")?,
            kind: row.get(2)?,
            detail: row.get(3)?,
        });
    }
    Ok(events)
}
