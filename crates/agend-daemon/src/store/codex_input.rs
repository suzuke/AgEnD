//! Monotonic receipt attribution for threads that admitted manual Codex input.
use super::StoreError;
use rusqlite::Connection;

pub fn requires_identified(conn: &Connection, thread: &str) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_input_threads WHERE thread_id = ?1)",
        [thread],
        |row| row.get(0),
    )?)
}

/// Commit before the frontend starts and before reconciliation; failures deny admission.
pub fn enable_identified(conn: &Connection, thread: &str) -> Result<(), StoreError> {
    conn.execute(
        "INSERT OR IGNORE INTO codex_input_threads(thread_id) VALUES (?1)",
        [thread],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SqliteStore;
    use agend_testkit::{block_on, tempdir::TempDir};
    #[test]
    fn strict_attribution_is_monotonic_durable_and_not_pruned() {
        let dir = TempDir::new("codex-input-durable").unwrap();
        {
            let store = SqliteStore::open(dir.path(), 0).unwrap();
            store
                .call_blocking(|conn| {
                    assert!(!requires_identified(conn, "thread")?);
                    enable_identified(conn, "thread")?;
                    enable_identified(conn, "thread")?;
                    Ok(())
                })
                .unwrap();
        }
        let store = SqliteStore::open(dir.path(), 0).unwrap();
        block_on(store.prune(100 * crate::store::retention::DAY_MS)).unwrap();
        let snapshot = block_on(store.snapshot(100 * crate::store::retention::DAY_MS)).unwrap();
        assert!(
            snapshot.taken && !snapshot.empty,
            "strict attribution is persistent data"
        );
        let saved = Connection::open(&snapshot.path).unwrap();
        assert!(requires_identified(&saved, "thread").unwrap());
        store
            .call_blocking(|conn| {
                assert!(requires_identified(conn, "thread")?);
                assert!(!requires_identified(conn, "another-thread")?);
                let count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM codex_input_threads", [], |r| r.get(0))?;
                assert_eq!(count, 1);
                Ok(())
            })
            .unwrap();
    }
}
