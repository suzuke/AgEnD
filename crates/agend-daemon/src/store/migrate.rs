//! Forward-only schema migrations (gate 5 P5). Migration N (1-based) moves
//! `PRAGMA user_version` from N-1 to N; each runs in its own transaction, so
//! a failing one leaves the database and its version as they were. There
//! are no down migrations: downgrading is restoring the pre-upgrade DB
//! snapshot.
//!
//! Adding a migration: append a file here and to [`MIGRATIONS`], re-bless
//! `golden/schema.sql`, and add `fixtures/schema-vN.sql` for the new version
//! (the tests require one fixture per version).
//!
//! Must NOT: edit or reorder a migration that has shipped.

use rusqlite::Connection;

use super::StoreError;

/// One schema step: a name for errors and the SQL it runs.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub name: &'static str,
    pub sql: &'static str,
}

/// Every migration of this binary, in order.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        name: "0001_init",
        sql: include_str!("migrations/0001_init.sql"),
    },
    Migration {
        name: "0002_instances",
        sql: include_str!("migrations/0002_instances.sql"),
    },
    Migration {
        name: "0003_session_started",
        sql: include_str!("migrations/0003_session_started.sql"),
    },
    Migration {
        name: "0004_messages",
        sql: include_str!("migrations/0004_messages.sql"),
    },
    Migration {
        name: "0005_pipeline",
        sql: include_str!("migrations/0005_pipeline.sql"),
    },
    Migration {
        name: "0006_codex_input_threads",
        sql: include_str!("migrations/0006_codex_input_threads.sql"),
    },
    Migration {
        name: "0007_claude_delivery",
        sql: include_str!("migrations/0007_claude_delivery.sql"),
    },
    Migration {
        name: "0008_claude_owned_files",
        sql: include_str!("migrations/0008_claude_owned_files.sql"),
    },
    Migration {
        name: "0009_claude_startup",
        sql: include_str!("migrations/0009_claude_startup.sql"),
    },
    Migration {
        name: "0010_opencode_permissions",
        sql: include_str!("migrations/0010_opencode_permissions.sql"),
    },
];

/// The schema version this binary creates and supports.
pub const LATEST_VERSION: i64 = MIGRATIONS.len() as i64;

/// The schema version a migration list ends at.
pub(super) fn version_of(migrations: &[Migration]) -> i64 {
    migrations.len() as i64
}

/// Runs every migration after `from`, each in its own transaction.
pub(super) fn apply(
    conn: &mut Connection,
    from: i64,
    migrations: &[Migration],
) -> Result<(), StoreError> {
    let from = usize::try_from(from)
        .map_err(|_| StoreError::Invalid(format!("negative schema version {from}")))?;
    for (index, migration) in migrations.iter().enumerate().skip(from) {
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)
            .map_err(|source| StoreError::Migration {
                name: migration.name,
                source,
            })?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
