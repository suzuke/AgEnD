//! Where a holder lives on disk: `$AGEND_HOME/run/holders/<id>.{sock,lock,log}`
//! (gate 4 P3), and how to tell from outside whether it is running.
//!
//! The holder keeps an exclusive `flock` on `<id>.lock` for its whole life and
//! writes its pid into it. The kernel drops the lock when the process dies, so
//! a stale file never looks like a live holder and pid reuse cannot fool us.
//!
//! Must NOT: signal or stop a holder, or connect to its socket; this module
//! only looks at files.

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

/// Longest socket path a holder accepts (macOS `sun_path` holds 104 bytes).
pub const MAX_SOCKET_PATH: usize = 100;
/// Longest instance id; keeps `<id>.sock` inside [`MAX_SOCKET_PATH`].
pub const MAX_INSTANCE_ID: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderPaths {
    pub dir: PathBuf,
    pub socket: PathBuf,
    pub lock: PathBuf,
    pub log: PathBuf,
}

impl HolderPaths {
    pub fn new(agend_home: &Path, instance_id: &str) -> Self {
        let dir = agend_home.join("run").join("holders");
        Self {
            socket: dir.join(format!("{instance_id}.sock")),
            lock: dir.join(format!("{instance_id}.lock")),
            log: dir.join(format!("{instance_id}.log")),
            dir,
        }
    }

    /// Paths under `$AGEND_HOME`, which must be set and absolute.
    pub fn from_env(instance_id: &str) -> Result<Self, String> {
        Ok(Self::new(&agend_home()?, instance_id))
    }

    /// Refuses a socket path the OS may truncate (P3).
    pub fn check_socket_len(&self) -> Result<(), String> {
        let len = self.socket.as_os_str().len();
        if len > MAX_SOCKET_PATH {
            return Err(format!(
                "socket path is {len} bytes, over the {MAX_SOCKET_PATH}-byte limit: {}",
                self.socket.display()
            ));
        }
        Ok(())
    }
}

/// `$AGEND_HOME`, required to be set and absolute.
pub fn agend_home() -> Result<PathBuf, String> {
    match std::env::var_os("AGEND_HOME") {
        Some(home) if Path::new(&home).is_absolute() => Ok(PathBuf::from(home)),
        Some(home) => Err(format!(
            "AGEND_HOME must be an absolute path, got {}",
            Path::new(&home).display()
        )),
        None => Err("AGEND_HOME is not set".into()),
    }
}

/// Instance ids name files, so only `[A-Za-z0-9_-]`, 1 to 32 bytes, and not
/// starting with `-`.
pub fn validate_instance_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= MAX_INSTANCE_ID
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid instance id {id:?}: use 1-{MAX_INSTANCE_ID} characters from A-Z a-z 0-9 _ -, not starting with -"
        ))
    }
}

/// The pid written in a lock file, if it parses.
pub fn read_lock_pid(lock: &Path) -> Option<u32> {
    let mut text = String::new();
    File::open(lock).ok()?.read_to_string(&mut text).ok()?;
    text.trim().parse().ok()
}

/// `Some(pid)` while a holder holds `lock`; `None` when nobody does (or the
/// file does not exist). Probes with a shared lock that it drops at once.
pub fn lock_holder(lock: &Path) -> io::Result<Option<u32>> {
    let file = match File::open(lock) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    // SAFETY: flock on an fd we own; the lock is released when `file` drops.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(None);
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(Some(read_lock_pid(lock).unwrap_or(0)))
    } else {
        Err(err)
    }
}

/// Whether a holder runs for these paths, judged by the lock alone: `Some(pid)`
/// while the flock is held. It never connects to the socket, because a new
/// connection takes over the daemon's own long-lived one (P4); gate 6's
/// `recover` relies on this.
pub fn is_running(paths: &HolderPaths) -> io::Result<Option<u32>> {
    lock_holder(&paths.lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_ids_are_file_name_safe() {
        for good in ["dev-1", "a", "demo_2", &"x".repeat(MAX_INSTANCE_ID)] {
            assert_eq!(validate_instance_id(good), Ok(()), "{good}");
        }
        for bad in [
            "",
            "-x",
            "a/b",
            "..",
            "a b",
            "é",
            &"x".repeat(MAX_INSTANCE_ID + 1),
        ] {
            assert!(validate_instance_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn long_socket_paths_are_refused_with_path_and_length() {
        let ok = HolderPaths::new(Path::new("/tmp/h"), "dev-1");
        assert_eq!(ok.socket, Path::new("/tmp/h/run/holders/dev-1.sock"));
        assert_eq!(ok.check_socket_len(), Ok(()));

        let home = format!("/{}", "d".repeat(80));
        let long = HolderPaths::new(Path::new(&home), "dev-1");
        let err = long.check_socket_len().unwrap_err();
        assert!(err.contains("socket path is 104 bytes"), "{err}");
        assert!(err.contains(&home), "{err}");
    }

    #[test]
    fn an_unlocked_or_missing_lock_file_is_not_a_running_holder() {
        let dir = agend_testkit::tempdir::TempDir::new("hp").unwrap();
        let lock = dir.path().join("x.lock");
        assert_eq!(lock_holder(&lock).unwrap(), None);
        std::fs::write(&lock, "4242\n").unwrap();
        assert_eq!(lock_holder(&lock).unwrap(), None);
        assert_eq!(read_lock_pid(&lock), Some(4242));

        let held = File::open(&lock).unwrap();
        // SAFETY: flock on an fd we own.
        assert_eq!(
            unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        assert_eq!(lock_holder(&lock).unwrap(), Some(4242));
        drop(held);
        assert_eq!(lock_holder(&lock).unwrap(), None);
    }
}
