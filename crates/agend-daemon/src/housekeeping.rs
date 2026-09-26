//! Housekeeping (gate 6 P5, P8): at boot and then every hour, whatever of
//! today's work is not done yet. Each step is idempotent, so "every hour" is
//! also how a laptop that slept through a day catches up:
//!
//! 1. `prune` the DB (gate 5 P8) and take today's DB snapshot unless it
//!    exists (gate 5 P9);
//! 2. delete daemon logs older than 7 days (`logs/daemon-YYYY-MM-DD.log`);
//! 3. rotate the shim audit log: `audit/shim.jsonl` last written before today
//!    becomes `audit/shim-<that UTC date>.jsonl`; rotated files older than 14
//!    days are deleted;
//! 4. delete holder logs (`run/holders/<id>.log`) unchanged for 7 days whose
//!    holder is not running.
//!
//! Periods come from `store::retention::RETENTION`. A failed step is logged
//! and never stops the daemon.
//!
//! Must NOT: delete a file whose name does not match its rule's pattern, or
//! the log of a running holder.

use std::fs;
use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::log;
use crate::runtime::files;
use crate::store::SqliteStore;
use crate::store::retention::{AUDIT_LOG, DAEMON_LOG, DAY_MS, HOLDER_LOGS, Target, file_keep_days};
use crate::store::snapshot::utc_date;

/// A file step: what it changed (for the log).
type FileStep = fn(&Path, u64) -> io::Result<Vec<String>>;

/// Runs every step, logging what changed and what failed.
pub async fn run(store: &SqliteStore, home: &Path, now_unix_ms: u64) {
    match store.prune(now_unix_ms).await {
        Ok(report) => {
            let removed: u64 = report.tables.iter().map(|t| t.before - t.after).sum();
            if removed > 0 {
                log::line(&format!("housekeeping: prune removed {removed} row(s)"));
            }
        }
        Err(e) => log::line(&format!("housekeeping: prune failed: {e}")),
    }
    match store.snapshot(now_unix_ms).await {
        Ok(report) if report.taken => log::line(&format!(
            "housekeeping: DB snapshot {} ({} bytes)",
            report.path.display(),
            report.bytes
        )),
        Ok(_) => {}
        Err(e) => log::line(&format!("housekeeping: DB snapshot failed: {e}")),
    }
    let steps: [(&str, FileStep); 3] = [
        ("daemon logs", daemon_logs),
        ("audit log", audit_log),
        ("holder logs", holder_logs),
    ];
    for (what, step) in steps {
        match step(home, now_unix_ms) {
            Ok(changed) if !changed.is_empty() => {
                log::line(&format!("housekeeping: {what}: {}", changed.join(", ")));
            }
            Ok(_) => {}
            Err(e) => log::line(&format!("housekeeping: {what} failed: {e}")),
        }
    }
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` date (H. Hinnant's
/// `days_from_civil`).
pub fn day_number(date: &str) -> Option<i64> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let s = date.get(r)?;
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())?
    };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn today(now_unix_ms: u64) -> i64 {
    (now_unix_ms / DAY_MS) as i64
}

fn keep(target: Target) -> io::Result<i64> {
    file_keep_days(target)
        .map(|d| d as i64)
        .ok_or_else(|| io::Error::other("no retention rule"))
}

/// Deletes files in `dir` named `<prefix>YYYY-MM-DD<suffix>` whose date is
/// `keep_days` or more days before today; returns their names.
fn delete_dated(
    dir: &Path,
    prefix: &str,
    suffix: &str,
    keep_days: i64,
    now_unix_ms: u64,
) -> io::Result<Vec<String>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut deleted = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let day = name
            .strip_prefix(prefix)
            .and_then(|n| n.strip_suffix(suffix))
            .and_then(day_number);
        if let Some(day) = day
            && entry.file_type()?.is_file()
            && today(now_unix_ms) - day >= keep_days
        {
            fs::remove_file(entry.path())?;
            deleted.push(format!("deleted {name}"));
        }
    }
    deleted.sort();
    Ok(deleted)
}

/// `logs/daemon-YYYY-MM-DD.log`, 7 days (today counts as one).
pub fn daemon_logs(home: &Path, now_unix_ms: u64) -> io::Result<Vec<String>> {
    let keep = keep(Target::DailyRotatedFile { path: DAEMON_LOG })?;
    delete_dated(
        &home.join(log::LOGS_DIR),
        "daemon-",
        ".log",
        keep,
        now_unix_ms,
    )
}

/// Modification time of `path` in unix milliseconds.
fn modified_ms(path: &Path) -> io::Result<u64> {
    let modified = fs::metadata(path)?.modified()?;
    Ok(modified
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64))
}

/// Rotates `audit/shim.jsonl` and deletes rotated files after 14 days.
pub fn audit_log(home: &Path, now_unix_ms: u64) -> io::Result<Vec<String>> {
    let live = home.join(AUDIT_LOG);
    let dir = live.parent().expect("audit log has a directory");
    let mut changed = Vec::new();
    match modified_ms(&live) {
        Ok(written) if today(written) < today(now_unix_ms) => {
            let name = format!("shim-{}.jsonl", utc_date(written));
            let rotated = dir.join(&name);
            if rotated.exists() {
                changed.push(format!("{name} exists; shim.jsonl not rotated"));
            } else {
                fs::rename(&live, &rotated)?;
                changed.push(format!("shim.jsonl -> {name}"));
            }
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let keep = keep(Target::DailyRotatedFile { path: AUDIT_LOG })?;
    changed.extend(delete_dated(dir, "shim-", ".jsonl", keep, now_unix_ms)?);
    Ok(changed)
}

/// Deletes `run/holders/<id>.log` unchanged for 7 days whose holder is not
/// running.
pub fn holder_logs(home: &Path, now_unix_ms: u64) -> io::Result<Vec<String>> {
    let keep_ms = keep(Target::IdleFiles {
        pattern: HOLDER_LOGS,
    })? as u64
        * DAY_MS;
    let dir = files::holders_dir(home);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut deleted = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".log") else {
            continue;
        };
        if !entry.file_type()?.is_file()
            || now_unix_ms.saturating_sub(modified_ms(&entry.path())?) < keep_ms
            || !matches!(files::running(home, id), Ok(None))
        {
            continue;
        }
        fs::remove_file(entry.path())?;
        deleted.push(format!("deleted {name}"));
    }
    deleted.sort();
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::fakes::FakeClock;
    use agend_testkit::tempdir::TempDir;
    use std::fs::File;
    use std::os::fd::AsRawFd;
    use std::time::{Duration, SystemTime};

    const NOW: u64 = FakeClock::DEFAULT_START_UNIX_MS; // 2026-09-21

    fn set_mtime(path: &Path, unix_ms: u64) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_millis(unix_ms))
            .unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn day_numbers_match_the_snapshot_dates() {
        for ms in [0, 951_782_400_000, NOW, NOW + 400 * DAY_MS] {
            assert_eq!(day_number(&utc_date(ms)), Some(today(ms)), "{ms}");
        }
        for bad in ["", "2026-9-21", "2026-13-01", "20x6-09-21", "2026-09-21x"] {
            assert_eq!(day_number(bad), None, "{bad}");
        }
    }

    #[test]
    fn daemon_logs_keep_seven_days_and_other_files() {
        let dir = TempDir::new("g6-hk-logs").unwrap();
        let logs = dir.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let clock = FakeClock::new(NOW);
        for day in 0..10 {
            fs::write(logs.join(log::file_name(NOW + day * DAY_MS)), "x").unwrap();
        }
        fs::write(logs.join("notes.txt"), "mine").unwrap();
        fs::write(logs.join("daemon-latest.log"), "mine").unwrap();
        clock.advance(9 * DAY_MS); // 2026-09-30
        let deleted = daemon_logs(dir.path(), clock.peek()).unwrap();
        assert_eq!(
            deleted,
            [
                "deleted daemon-2026-09-21.log",
                "deleted daemon-2026-09-22.log",
                "deleted daemon-2026-09-23.log"
            ]
        );
        let left = names(&logs);
        assert_eq!(left.len(), 9, "{left:?}");
        assert_eq!(
            left[..2],
            ["daemon-2026-09-24.log", "daemon-2026-09-25.log"]
        );
        assert!(left.contains(&"notes.txt".to_string()));
        assert!(left.contains(&"daemon-latest.log".to_string()));
        assert_eq!(
            left.iter().filter(|n| n.starts_with("daemon-2026")).count(),
            7
        );
    }

    #[test]
    fn the_audit_log_rotates_daily_and_keeps_fourteen() {
        let dir = TempDir::new("g6-hk-audit").unwrap();
        let audit = dir.path().join("audit");
        fs::create_dir(&audit).unwrap();
        let live = audit.join("shim.jsonl");
        // Written today: stays.
        fs::write(&live, "{}\n").unwrap();
        set_mtime(&live, NOW);
        assert!(audit_log(dir.path(), NOW).unwrap().is_empty());
        // Twenty days, one line each day, rotated the next day.
        for day in 1..=20 {
            let now = NOW + day * DAY_MS;
            let changed = audit_log(dir.path(), now).unwrap();
            let name = format!("shim-{}.jsonl", utc_date(now - DAY_MS));
            assert!(
                changed.contains(&format!("shim.jsonl -> {name}")),
                "{changed:?}"
            );
            fs::write(&live, "{}\n").unwrap();
            set_mtime(&live, now);
        }
        let left = names(&audit);
        assert_eq!(
            left.len(),
            14,
            "13 rotated days + today's live file: {left:?}"
        );
        assert_eq!(
            left[0],
            format!("shim-{}.jsonl", utc_date(NOW + 7 * DAY_MS))
        );
        assert_eq!(left[13], "shim.jsonl");
    }

    #[test]
    fn holder_logs_go_only_when_idle_seven_days_and_the_holder_is_gone() {
        let dir = TempDir::new("g6-hk-holders").unwrap();
        let home = dir.path();
        let holders = files::holders_dir(home);
        fs::create_dir_all(&holders).unwrap();
        for id in ["old", "live", "fresh"] {
            let log = files::log_path(home, id);
            fs::write(&log, "x").unwrap();
            set_mtime(&log, NOW - 30 * DAY_MS);
        }
        set_mtime(&files::log_path(home, "fresh"), NOW - 6 * DAY_MS);
        fs::write(
            files::lock_path(home, "live"),
            format!("{}\n", std::process::id()),
        )
        .unwrap();
        let held = File::open(files::lock_path(home, "live")).unwrap();
        // SAFETY: flock on an fd we own.
        assert_eq!(
            unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        assert_eq!(holder_logs(home, NOW).unwrap(), ["deleted old.log"]);
        assert_eq!(names(&holders), ["fresh.log", "live.lock", "live.log"]);
    }
}
