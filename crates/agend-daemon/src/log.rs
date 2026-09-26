//! The daemon log (gate 6 P8). Every line goes to stderr (the foreground
//! terminal) and, once the daemon owns `agend.db` ([`to_files`]), is
//! appended to `$AGEND_HOME/logs/daemon-YYYY-MM-DD.log` (UTC date, one file
//! per day, 0600). Old files are deleted by `crate::housekeeping` (7 days).
//! A daemon that could not get `agend.db` never writes to the files, so a
//! refused second daemon leaves the first one's log unchanged.
//!
//! Must NOT: be used for program output meant for another program; it is
//! for people.

use std::fs::{DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::store::snapshot::utc_date;

/// The log directory inside the AgEnD home.
pub const LOGS_DIR: &str = "logs";

static DIR: OnceLock<PathBuf> = OnceLock::new();
static WRITE: Mutex<()> = Mutex::new(());

/// `daemon-YYYY-MM-DD.log` for a unix time in milliseconds.
pub fn file_name(unix_ms: u64) -> String {
    format!("daemon-{}.log", utc_date(unix_ms))
}

/// `YYYY-MM-DDTHH:MM:SSZ`.
pub fn timestamp(unix_ms: u64) -> String {
    let secs = (unix_ms / 1000) % 86_400;
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        utc_date(unix_ms),
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// From now on, also append to `<home>/logs/` (created 0700).
pub fn to_files(home: &Path) -> io::Result<()> {
    let dir = home.join(LOGS_DIR);
    DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    let _ = DIR.set(dir);
    Ok(())
}

/// Logs one line.
pub fn line(message: &str) {
    let now = now_unix_ms();
    let text = format!("{} {message}\n", timestamp(now));
    let _guard = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let _ = io::stderr().write_all(text.as_bytes());
    if let Some(dir) = DIR.get() {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(dir.join(file_name(now)));
        if let Ok(mut file) = file {
            let _ = file.write_all(text.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_timestamps_are_utc() {
        // 2026-09-21T14:13:20Z (python3: datetime.utcfromtimestamp(1790000000))
        let ms = 1_790_000_000_000;
        assert_eq!(file_name(ms), "daemon-2026-09-21.log");
        assert_eq!(timestamp(ms), "2026-09-21T14:13:20Z");
        assert_eq!(timestamp(0), "1970-01-01T00:00:00Z");
    }
}
