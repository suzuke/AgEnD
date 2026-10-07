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

pub use agend_core::runtime_records::{Instance, InstanceStatus};

/// `[a-z0-9-]{1,24}`: the id names files under `run/holders/`.
pub use agend_core::runtime_records::validate_id;

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
    let delivery = row.get(10)?;
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
            delivery,
            id: id.clone(),
        })
    })())
}

const COLUMNS: &str = "id, backend, program, args, working_directory, session_id, status, \
                       session_started, agent_pid, legacy_no_thread, delivery";

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
        delivery,
    } = instance;
    validate_id(id).map_err(StoreError::Invalid)?;
    let args = serde_json::to_string(args).map_err(|e| StoreError::Invalid(e.to_string()))?;
    let inserted = conn.execute(
        &format!(
            "INSERT INTO instances ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
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
            legacy_no_thread,
            delivery
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

/// Preserve a restored episode, or atomically mark a new failure episode.
pub(super) fn failure(
    conn: &mut Connection,
    id: &str,
    reason: &str,
    now: u64,
    new_episode: bool,
) -> Result<(String, u64), StoreError> {
    let tx = conn.transaction()?;
    let previous: Option<(String, u64)> = tx
        .query_row(
            "SELECT reason, since_ms FROM instance_failures WHERE instance_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if !new_episode && let Some(saved) = previous.as_ref() {
        return Ok(saved.clone());
    }
    let since = previous.map_or(now, |(_, old)| now.max(old.saturating_add(1)));
    if since > i64::MAX as u64 {
        return Err(StoreError::Invalid("failure episode exhausted".into()));
    }
    set_status(&tx, id, InstanceStatus::Failed)?;
    tx.execute(
        "INSERT INTO instance_failures(instance_id,reason,since_ms) VALUES(?1,?2,?3)
        ON CONFLICT(instance_id) DO UPDATE SET reason=excluded.reason,since_ms=excluded.since_ms",
        rusqlite::params![id, reason, since],
    )?;
    tx.commit()?;
    Ok((reason.to_owned(), since))
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
