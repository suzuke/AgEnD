//! Runs the probes behind `agend doctor` (gate 9 P8); the rules themselves
//! (git minimum, install commands, thresholds) are data in
//! `agend_core::setup`. Gate 13 adds service registration and the
//! agent-PATH shims.
//!
//! Must NOT: change anything, or hold setup rules itself.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub use agend_daemon::backend_versions::version_probe::version_line;

/// The first executable `name` on `PATH`, skipping `skip` (the agents'
/// shim directory `$AGEND_HOME/bin`).
pub fn find_on_path(name: &str, skip: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir != skip)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Free bytes for unprivileged users on the disk of `path` (`statvfs`).
pub fn free_bytes(path: &Path) -> std::io::Result<u64> {
    let c = CString::new(path.as_os_str().as_bytes())?;
    // SAFETY: statvfs fills the zeroed struct we own from a NUL-terminated path.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    #[allow(clippy::unnecessary_cast)]
    Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// Whether this user may create files in `dir` (`access(W_OK | X_OK)`).
pub fn writable(dir: &Path) -> bool {
    let Ok(c) = CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: access only reads a NUL-terminated path.
    unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

/// Bytes of the regular files under `dir` (symlinks are not followed).
pub fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_bytes(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

/// `212 GB`, `38 MB`, `4 KB`.
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{} {}", value.round() as u64, UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_rounded_to_the_largest_unit() {
        assert_eq!(size(4096), "4 KB");
        assert_eq!(size(38 << 20), "38 MB");
        assert_eq!(size(212 << 30), "212 GB");
    }

    #[test]
    fn free_space_and_writability_of_tmp() {
        assert!(free_bytes(Path::new("/tmp")).unwrap() > 0);
        assert!(writable(Path::new("/tmp")));
        assert!(!writable(Path::new("/nonexistent-g9")));
    }
}
