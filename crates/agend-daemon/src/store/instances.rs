//! The `instances` table (gate 6 P2): which agents the daemon keeps running.
//! Rows are only added and removed on purpose (`daemon_probe` now, `agend
//! instance` in gate 9); the daemon itself only changes `status`.
//!
//! Must NOT: delete a row on its own (retention: forever).

use agend_core::model::Backend;
use rusqlite::{Connection, OptionalExtension, Row};

use super::{PRIMARY_KEY, StoreError, is_constraint};

/// Longest instance id: keeps `run/holders/<id>.sock` short (gate 6 P2).
pub const MAX_ID: usize = 24;

/// Where an instance stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceStatus {
    /// Never started: the first start is fresh.
    New,
    /// The daemon keeps it running; every start after the first resumes.
    Running,
    /// The daemon gave up (gate 6 P6); a human decides.
    Failed,
}

impl InstanceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Running => "running",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::New, Self::Running, Self::Failed]
            .into_iter()
            .find(|v| v.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub id: String,
    pub backend: Backend,
    pub program: String,
    /// The agent's base arguments; session arguments are added per start.
    pub args: Vec<String>,
    pub working_directory: String,
    /// The backend session to resume: claude's session id, codex's thread
    /// id (created by the daemon, gate 7 P3); `None` when there is none yet
    /// (opencode until gate 12).
    pub session_id: Option<String>,
    pub status: InstanceStatus,
    /// The backend session was created (the first `Spawn` was
    /// acknowledged); set with `running` and never cleared (migration 0003).
    pub session_started: bool,
    /// The agent's pid (its own process group) from the last `Spawned`;
    /// cleared by the codex sweep after its holder died (gate 7 P2).
    pub agent_pid: Option<u32>,
    /// A codex instance migration 0004 found without a thread id that may
    /// hold a conversation: never started again, a human decides (gate 7 P3).
    pub legacy_no_thread: bool,
}

/// `[a-z0-9-]{1,24}`: the id names files under `run/holders/`.
pub fn validate_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid instance id {id:?}: use 1-{MAX_ID} characters from a-z 0-9 -"
        ))
    }
}

/// A new random session id in UUID v4 form (claude's `--session-id` needs
/// one), from `/dev/urandom`.
pub fn new_session_id() -> std::io::Result<String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<Result<Instance, StoreError>> {
    let id: String = row.get(0)?;
    let backend: String = row.get(1)?;
    let args: String = row.get(3)?;
    let status: String = row.get(6)?;
    let program = row.get(2)?;
    let working_directory = row.get(4)?;
    let session_id = row.get(5)?;
    let session_started = row.get(7)?;
    let agent_pid: Option<i64> = row.get(8)?;
    let legacy_no_thread = row.get(9)?;
    Ok((|| {
        let invalid = |what: String| StoreError::Invalid(format!("instance {id}: {what}"));
        Ok(Instance {
            backend: Backend::parse(&backend)
                .ok_or_else(|| invalid(format!("backend {backend:?}")))?,
            args: serde_json::from_str(&args).map_err(|e| invalid(format!("args: {e}")))?,
            status: InstanceStatus::parse(&status)
                .ok_or_else(|| invalid(format!("status {status:?}")))?,
            program,
            working_directory,
            session_id,
            session_started,
            agent_pid: agent_pid
                .map(|p| u32::try_from(p).map_err(|_| invalid(format!("agent_pid {p}"))))
                .transpose()?,
            legacy_no_thread,
            id: id.clone(),
        })
    })())
}

const COLUMNS: &str = "id, backend, program, args, working_directory, session_id, status, \
                       session_started, agent_pid, legacy_no_thread";

pub(super) fn list(conn: &Connection) -> Result<Vec<Instance>, StoreError> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM instances ORDER BY id"))?;
    let rows = stmt.query_map([], from_row)?;
    rows.map(|row| row?).collect()
}

pub(crate) fn get(conn: &Connection, id: &str) -> Result<Option<Instance>, StoreError> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM instances WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()?
    .transpose()
}

pub(super) fn insert(conn: &Connection, instance: &Instance) -> Result<(), StoreError> {
    let Instance {
        id,
        backend,
        program,
        args,
        working_directory,
        session_id,
        status,
        session_started,
        agent_pid,
        legacy_no_thread,
    } = instance;
    validate_id(id).map_err(StoreError::Invalid)?;
    let args = serde_json::to_string(args).map_err(|e| StoreError::Invalid(e.to_string()))?;
    let inserted = conn.execute(
        &format!(
            "INSERT INTO instances ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
        ),
        rusqlite::params![
            id,
            backend.as_str(),
            program,
            args,
            working_directory,
            session_id,
            status.as_str(),
            session_started,
            agent_pid,
            legacy_no_thread
        ],
    );
    match inserted {
        Ok(_) => Ok(()),
        Err(e) if is_constraint(&e, PRIMARY_KEY) => {
            Err(StoreError::Exists(format!("instance {id}")))
        }
        Err(e) => Err(e.into()),
    }
}

/// Removes the row; false when there was none.
pub(super) fn remove(conn: &Connection, id: &str) -> Result<bool, StoreError> {
    Ok(conn.execute("DELETE FROM instances WHERE id = ?1", [id])? == 1)
}

/// Sets `status`; `running` also records `session_started` in the same
/// statement (the first `Spawn` was acknowledged, gate 8 P5).
pub(super) fn set_status(
    conn: &Connection,
    id: &str,
    status: InstanceStatus,
) -> Result<(), StoreError> {
    match conn.execute(
        "UPDATE instances SET status = ?2, \
         session_started = CASE WHEN ?2 = 'running' THEN 1 ELSE session_started END \
         WHERE id = ?1",
        [id, status.as_str()],
    )? {
        1 => Ok(()),
        _ => Err(StoreError::Invalid(format!("no instance {id}"))),
    }
}

/// Sets the backend session id (codex: the thread the daemon created, gate
/// 7 P3).
pub(crate) fn set_session_id(conn: &Connection, id: &str, session: &str) -> Result<(), StoreError> {
    match conn.execute(
        "UPDATE instances SET session_id = ?2 WHERE id = ?1",
        [id, session],
    )? {
        1 => Ok(()),
        _ => Err(StoreError::Invalid(format!("no instance {id}"))),
    }
}

/// Sets or clears the agent's pid (gate 7 P2).
pub(crate) fn set_agent_pid(
    conn: &Connection,
    id: &str,
    pid: Option<u32>,
) -> Result<(), StoreError> {
    match conn.execute(
        "UPDATE instances SET agent_pid = ?2 WHERE id = ?1",
        rusqlite::params![id, pid],
    )? {
        1 => Ok(()),
        _ => Err(StoreError::Invalid(format!("no instance {id}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_short_and_file_name_safe() {
        for good in ["g6-1", "a", &"x".repeat(MAX_ID)] {
            assert_eq!(validate_id(good), Ok(()), "{good}");
        }
        for bad in ["", "G6", "a_b", "a/b", "..", &"x".repeat(MAX_ID + 1)] {
            assert!(validate_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn session_ids_are_uuid_v4() {
        let a = new_session_id().unwrap();
        let b = new_session_id().unwrap();
        assert_ne!(a, b);
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'), "{a}");
        assert!("89ab".contains(&parts[3][..1]), "{a}");
    }
}
