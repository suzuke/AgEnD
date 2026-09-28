//! Retention (D31, gate 5 P8): one rule per table and per growing file under
//! `$AGEND_HOME`. A test fails when a table in `sqlite_master` has no rule,
//! so "every table has a retention period" is a check, not a comment.
//!
//! [`prune`] applies the table rules in one transaction and never VACUUMs
//! (the space is reused by later writes; the daily snapshot is compacted by
//! `VACUUM INTO` anyway). File rules are enforced by the daemon's
//! housekeeping (`crate::housekeeping`, gate 6 P8), which reads its periods
//! from this table ([`file_keep_days`]).
//!
//! Must NOT: delete from a table whose rule is [`Keep::Forever`].

use rusqlite::Connection;

use super::StoreError;

pub const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Forever,
    Days(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A table in `agend.db`; rows older than the limit (by `time_column`,
    /// unix ms) are deleted.
    Table {
        name: &'static str,
        time_column: Option<&'static str>,
    },
    /// A file under `$AGEND_HOME`, rotated daily into dated files; dated
    /// files older than the limit are deleted.
    DailyRotatedFile { path: &'static str },
    /// Files under `$AGEND_HOME` matching `pattern`, deleted once unchanged
    /// for longer than the limit (and, for holder logs, only while their
    /// holder is not running).
    IdleFiles { pattern: &'static str },
}

/// The shim's audit log (gate 3 T10), rotated to `audit/shim-YYYY-MM-DD.jsonl`.
pub const AUDIT_LOG: &str = "audit/shim.jsonl";
/// The daemon's log, written directly as `logs/daemon-YYYY-MM-DD.log`.
pub const DAEMON_LOG: &str = "logs/daemon.log";
/// Holder logs (gate 4 P3).
pub const HOLDER_LOGS: &str = "run/holders/<id>.log";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub target: Target,
    pub keep: Keep,
    /// Where the rule comes from and who enforces it.
    pub why: &'static str,
}

/// The retention table.
pub const RETENTION: &[Rule] = &[
    Rule {
        target: Target::Table {
            name: "tasks",
            time_column: None,
        },
        keep: Keep::Forever,
        why: "D31: tasks are kept forever (small); rows are never deleted",
    },
    Rule {
        target: Target::Table {
            name: "workflows",
            time_column: None,
        },
        keep: Keep::Forever,
        why: "D31: workflow versions are kept forever; tasks pin them (D21)",
    },
    Rule {
        target: Target::Table {
            name: "task_events",
            time_column: Some("occurred_at_unix_ms"),
        },
        keep: Keep::Days(14),
        why: "D31: events and state transitions 14 days; history for people only (P4)",
    },
    Rule {
        target: Target::Table {
            name: "instances",
            time_column: None,
        },
        keep: Keep::Forever,
        why: "gate 6 P2: an instance is kept until it is removed on purpose",
    },
    Rule {
        target: Target::Table {
            name: "messages",
            time_column: Some("created_at_unix_ms"),
        },
        keep: Keep::Days(30),
        why: "D31: messages 30 days; the only idempotency layer (gate 7 P5)",
    },
    Rule {
        target: Target::DailyRotatedFile { path: AUDIT_LOG },
        keep: Keep::Days(14),
        why: "gate 3 T10 audit log; daily rotation, 14 days (gate 6 P8); daemon housekeeping",
    },
    Rule {
        target: Target::DailyRotatedFile { path: DAEMON_LOG },
        keep: Keep::Days(7),
        why: "gate 6 P8: the daemon writes one file per UTC day, 7 days; daemon housekeeping",
    },
    Rule {
        target: Target::IdleFiles {
            pattern: HOLDER_LOGS,
        },
        keep: Keep::Days(7),
        why: "gate 6 P8: deleted when the holder is not running and 7 days unchanged",
    },
];

/// Days a file rule keeps (`None` for a table target or a missing rule).
pub fn file_keep_days(target: Target) -> Option<u64> {
    RETENTION
        .iter()
        .find(|rule| rule.target == target)
        .and_then(|rule| match rule.keep {
            Keep::Days(days) => Some(days),
            Keep::Forever => None,
        })
}

/// Row counts of one table before and after a prune.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TablePrune {
    pub table: &'static str,
    pub before: u64,
    pub after: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneReport {
    pub now_unix_ms: u64,
    /// Every table rule, in [`RETENTION`] order.
    pub tables: Vec<TablePrune>,
}

impl PruneReport {
    pub fn table(&self, name: &str) -> Option<&TablePrune> {
        self.tables.iter().find(|t| t.table == name)
    }
}

fn table_rules() -> impl Iterator<Item = (&'static str, Option<&'static str>, Keep)> {
    RETENTION.iter().filter_map(|rule| match rule.target {
        Target::Table { name, time_column } => Some((name, time_column, rule.keep)),
        Target::DailyRotatedFile { .. } | Target::IdleFiles { .. } => None,
    })
}

fn count(conn: &Connection, table: &str) -> Result<u64, StoreError> {
    // `table` comes from RETENTION, never from input.
    let n: i64 = conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
    Ok(n as u64)
}

pub(super) fn counts(conn: &Connection) -> Result<Vec<(&'static str, u64)>, StoreError> {
    table_rules()
        .map(|(name, _, _)| Ok((name, count(conn, name)?)))
        .collect()
}

pub(super) fn prune(conn: &mut Connection, now_unix_ms: u64) -> Result<PruneReport, StoreError> {
    let tx = conn.transaction()?;
    let mut tables = Vec::new();
    for (name, time_column, keep) in table_rules() {
        let before = count(&tx, name)?;
        if let Keep::Days(days) = keep {
            let column = time_column.ok_or_else(|| {
                StoreError::Invalid(format!("retention rule for {name} has no time column"))
            })?;
            let cutoff = now_unix_ms.saturating_sub(days.saturating_mul(DAY_MS));
            let cutoff = i64::try_from(cutoff).unwrap_or(i64::MAX);
            tx.execute(&format!("DELETE FROM {name} WHERE {column} < ?1"), [cutoff])?;
        }
        let after = count(&tx, name)?;
        tables.push(TablePrune {
            table: name,
            before,
            after,
        });
    }
    tx.commit()?;
    Ok(PruneReport {
        now_unix_ms,
        tables,
    })
}
