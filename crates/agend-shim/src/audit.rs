//! Audit record of refusals, bypasses and snapshots: one JSON object per line
//! in `$AGEND_HOME/audit/shim.jsonl`. Allowed and routed calls are not
//! recorded (they are every git call).
//!
//! Must NOT: block or change the outcome of the command if writing fails.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One audit line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Unix time in seconds.
    pub ts: u64,
    pub instance: Option<String>,
    /// `git`, `kill`, `killall` or `pkill`.
    pub tool: String,
    /// `refuse`, `bypass` or `snapshot`.
    pub event: String,
    /// Refusal code or snapshot operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `$AGEND_HOME/audit/shim.jsonl`.
pub fn log_path(home: &Path) -> PathBuf {
    home.join("audit").join("shim.jsonl")
}

/// Appends `record` under `home`; errors are ignored by design.
pub fn append(home: Option<&Path>, record: &Record) {
    let Some(home) = home else { return };
    let path = log_path(home);
    let Ok(mut line) = serde_json::to_string(record) else {
        return;
    };
    line.push('\n');
    let _ = path
        .parent()
        .map(std::fs::create_dir_all)
        .transpose()
        .and_then(|_| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
        })
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

/// Reads every record back (for tests and the demo).
pub fn read(home: &Path) -> Vec<Record> {
    std::fs::read_to_string(log_path(home))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
