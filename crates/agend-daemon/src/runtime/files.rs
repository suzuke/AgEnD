//! A holder's files as the daemon sees them: `$AGEND_HOME/run/holders/<id>.
//! {sock,lock,log}` (gate 4 P3). A holder holds an exclusive `flock` on its
//! lock file (content: its pid) for its whole life; the kernel drops it when
//! the holder dies, so the lock alone says whether a holder runs.
//!
//! The daemon does not link `agend-holder` (gate 6 P9), so the lock probe
//! below mirrors `agend_holder::paths::lock_holder`; the cross-process tests
//! run both against the same real holders.
//!
//! Must NOT: connect to a socket (a new connection takes over the daemon's
//! own, gate 4 P4) or signal anything.

use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// `run/holders` under the AgEnD home.
pub fn holders_dir(home: &Path) -> PathBuf {
    home.join("run").join("holders")
}

pub fn socket_path(home: &Path, id: &str) -> PathBuf {
    holders_dir(home).join(format!("{id}.sock"))
}

pub fn lock_path(home: &Path, id: &str) -> PathBuf {
    holders_dir(home).join(format!("{id}.lock"))
}

pub fn log_path(home: &Path, id: &str) -> PathBuf {
    holders_dir(home).join(format!("{id}.log"))
}

/// How long a locked file may show no live pid (a holder writes its pid
/// right after taking the lock).
const PID_SETTLE: Duration = Duration::from_secs(1);

/// `Some(pid)` while a holder holds `lock`, `None` when nobody does (or there
/// is no such file). Never 0, 1 or a pid that does not exist: a locked file
/// without a live pid is re-read for up to 1 s, then `WouldBlock`.
pub fn lock_holder(lock: &Path) -> io::Result<Option<u32>> {
    let deadline = Instant::now() + PID_SETTLE;
    loop {
        let file = match File::open(lock) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        // SAFETY: flock on an fd we own; released when `file` drops.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } == 0 {
            return Ok(None);
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(err);
        }
        drop(file);
        if let Some(pid) = read_pid(lock).filter(|&p| p > 1 && process_exists(p)) {
            return Ok(Some(pid));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("{} is locked but holds no live pid", lock.display()),
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn read_pid(lock: &Path) -> Option<u32> {
    let mut text = String::new();
    File::open(lock).ok()?.read_to_string(&mut text).ok()?;
    text.trim().parse().ok()
}

/// Whether a process with this pid exists (`getpgid`: sends no signal).
fn process_exists(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: getpgid only reads the process table.
    let found = unsafe { libc::getpgid(pid) } != -1;
    found || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The holder of instance `id`: `Some(pid)` while it runs.
pub fn running(home: &Path, id: &str) -> io::Result<Option<u32>> {
    lock_holder(&lock_path(home, id))
}

/// Every instance whose holder runs now, by id, with the holder's pid:
/// the `*.lock` files in `run/holders/` whose lock is held.
pub fn running_holders(home: &Path) -> io::Result<Vec<(String, u32)>> {
    let dir = holders_dir(home);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in entries {
        let name = entry?.file_name();
        let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".lock")) else {
            continue;
        };
        if let Some(pid) = lock_holder(&dir.join(&name))? {
            out.push((id.to_owned(), pid));
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;

    fn flock_ex(file: &File) {
        // SAFETY: flock on an fd we own.
        assert_eq!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
    }

    #[test]
    fn only_held_lock_files_are_running_holders() {
        let dir = TempDir::new("g6-files").unwrap();
        let home = dir.path();
        assert_eq!(running_holders(home).unwrap(), []);
        fs::create_dir_all(holders_dir(home)).unwrap();
        let me = std::process::id();
        for id in ["a", "b"] {
            fs::write(lock_path(home, id), format!("{me}\n")).unwrap();
        }
        fs::write(log_path(home, "c"), "not a lock").unwrap();
        let held = File::open(lock_path(home, "b")).unwrap();
        flock_ex(&held);
        assert_eq!(running_holders(home).unwrap(), [("b".to_string(), me)]);
        assert_eq!(running(home, "a").unwrap(), None);
        drop(held);
        assert_eq!(running_holders(home).unwrap(), []);
    }

    #[test]
    fn a_locked_file_without_a_live_pid_is_never_a_pid() {
        let dir = TempDir::new("g6-files-pid").unwrap();
        let lock = dir.path().join("x.lock");
        for content in ["", "0\n", "1\n", "99999999\n"] {
            fs::write(&lock, content).unwrap();
            let held = File::open(&lock).unwrap();
            flock_ex(&held);
            let err = lock_holder(&lock).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::WouldBlock, "{content:?}");
        }
    }
}
