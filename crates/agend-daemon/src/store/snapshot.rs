//! DB snapshots (gate 5 P9): `backups/agend-YYYY-MM-DD.db` once a day (UTC
//! date) and `backups/agend-YYYY-MM-DD-pre-vN.db` before a migration to
//! schema version N. Each is written as `VACUUM INTO` a hidden temporary
//! file, checked read-only with `PRAGMA quick_check`, then renamed, so a
//! crash never leaves half a snapshot under a snapshot name. Only the
//! [`KEEP`] newest files matching the name pattern are kept.
//!
//! Restoring is manual (README): stop the daemon, copy a snapshot over
//! `agend.db`, delete `agend.db-wal`.
//!
//! Must NOT: delete or overwrite a file whose name does not match the
//! snapshot pattern (or its temporary-file pattern).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags};

use super::{BACKUPS_DIR, StoreError, create_private_dir};

/// DB snapshots kept (D31).
pub const KEEP: usize = 7;

const PREFIX: &str = "agend-";
const SUFFIX: &str = ".db";
const TMP_SUFFIX: &str = ".tmp";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotReport {
    /// Today's snapshot.
    pub path: PathBuf,
    /// Whether this call wrote it (false: it already existed).
    pub taken: bool,
    pub bytes: u64,
    /// Time spent in `VACUUM INTO` + `quick_check` (zero when not taken).
    pub elapsed: Duration,
    /// Snapshot files deleted by the rotation, oldest first.
    pub rotated_out: Vec<String>,
    /// Leftover temporary files from an interrupted snapshot, deleted.
    pub stale_tmp_removed: Vec<String>,
    /// Snapshot files in `backups/` after the rotation.
    pub kept: Vec<String>,
}

/// `YYYY-MM-DD` (UTC) of a unix time in milliseconds.
pub fn utc_date(unix_ms: u64) -> String {
    let (y, m, d) = civil_from_days((unix_ms / super::retention::DAY_MS) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Proleptic Gregorian date of a day count since 1970-01-01 (H. Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Daily snapshot file name for `unix_ms`.
pub fn daily_name(unix_ms: u64) -> String {
    format!("{PREFIX}{}{SUFFIX}", utc_date(unix_ms))
}

/// Pre-upgrade snapshot file name, before migrating to `to_version`.
pub fn pre_upgrade_name(unix_ms: u64, to_version: i64) -> String {
    format!("{PREFIX}{}-pre-v{to_version}{SUFFIX}", utc_date(unix_ms))
}

/// Whether `name` is a snapshot file name: `agend-YYYY-MM-DD.db` or
/// `agend-YYYY-MM-DD-pre-vN.db`.
pub fn is_snapshot_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(PREFIX) else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(SUFFIX) else {
        return false;
    };
    let (date, tail) = rest.split_at_checked(10).unwrap_or((rest, "invalid"));
    let date_ok = date.bytes().enumerate().all(|(i, b)| match i {
        4 | 7 => b == b'-',
        _ => b.is_ascii_digit(),
    });
    let tail_ok = tail.is_empty()
        || tail
            .strip_prefix("-pre-v")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    date.len() == 10 && date_ok && tail_ok
}

fn tmp_name(name: &str) -> String {
    format!(".{name}{TMP_SUFFIX}")
}

fn is_tmp_name(name: &str) -> bool {
    name.strip_prefix('.')
        .and_then(|n| n.strip_suffix(TMP_SUFFIX))
        .is_some_and(is_snapshot_name)
}

/// Names in `dir` accepted by `keep`, sorted.
fn names(dir: &Path, keep: fn(&str) -> bool) -> Result<Vec<String>, StoreError> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str().filter(|n| keep(n)) {
            out.push(name.to_owned());
        }
    }
    out.sort();
    Ok(out)
}

/// `VACUUM INTO` a temporary file, `quick_check` it read-only, rename it to
/// `backups/<name>`. Returns the time spent.
fn write(conn: &Connection, backups: &Path, name: &str) -> Result<Duration, StoreError> {
    let started = Instant::now();
    let tmp = backups.join(tmp_name(name));
    if tmp.exists() {
        fs::remove_file(&tmp)?;
    }
    let tmp_text = tmp
        .to_str()
        .ok_or_else(|| StoreError::Invalid(format!("non-UTF-8 path {}", tmp.display())))?;
    conn.execute("VACUUM INTO ?1", [tmp_text])?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    let check = {
        let copy = Connection::open_with_flags(
            &tmp,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        copy.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))?
    };
    if check != "ok" {
        fs::remove_file(&tmp)?;
        return Err(StoreError::SnapshotCheck(format!("{name}: {check}")));
    }
    fs::rename(&tmp, backups.join(name))?;
    Ok(started.elapsed())
}

/// Deletes all but the [`KEEP`] newest snapshot files; returns the deleted
/// ones (oldest first) and the kept ones.
fn rotate(backups: &Path) -> Result<(Vec<String>, Vec<String>), StoreError> {
    // Sorted by name = by date; on one date the pre-upgrade snapshot sorts
    // before the daily one ('-' < '.'), which was taken after it.
    let mut kept = names(backups, is_snapshot_name)?;
    let excess = kept.len().saturating_sub(KEEP);
    let removed: Vec<String> = kept.drain(..excess).collect();
    for name in &removed {
        fs::remove_file(backups.join(name))?;
    }
    Ok((removed, kept))
}

/// Opens `backups/` (0700) and deletes leftover temporary files.
fn prepare(home: &Path) -> Result<(PathBuf, Vec<String>), StoreError> {
    let backups = home.join(BACKUPS_DIR);
    create_private_dir(&backups)?;
    let stale = names(&backups, is_tmp_name)?;
    for name in &stale {
        fs::remove_file(backups.join(name))?;
    }
    Ok((backups, stale))
}

pub(super) fn daily(
    conn: &Connection,
    home: &Path,
    now_unix_ms: u64,
) -> Result<SnapshotReport, StoreError> {
    let (backups, stale_tmp_removed) = prepare(home)?;
    let name = daily_name(now_unix_ms);
    let path = backups.join(&name);
    let (taken, elapsed) = if path.exists() {
        (false, Duration::ZERO)
    } else {
        (true, write(conn, &backups, &name)?)
    };
    let bytes = fs::metadata(&path)?.len();
    let (rotated_out, kept) = rotate(&backups)?;
    Ok(SnapshotReport {
        path,
        taken,
        bytes,
        elapsed,
        rotated_out,
        stale_tmp_removed,
        kept,
    })
}

/// The snapshot taken before migrating an existing database to `to_version`.
pub(super) fn pre_upgrade(
    conn: &Connection,
    home: &Path,
    now_unix_ms: u64,
    to_version: i64,
) -> Result<(), StoreError> {
    let (backups, _) = prepare(home)?;
    write(conn, &backups, &pre_upgrade_name(now_unix_ms, to_version))?;
    rotate(&backups)?;
    Ok(())
}
