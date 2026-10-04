//! Private, ordered, atomically published helper receipts. No expiry. One
//! home-wide flock serializes publication and replay; SQLite stays in daemon.
use agend_core::protocol::client::{ClaudePendingRecord, ClaudeRequestData, MAX_LINE_BYTES};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

type Pending = ClaudePendingRecord;
pub struct Spool {
    dir: PathBuf,
    lock: File,
    deadline: Instant,
}
fn directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(io::Error::other(format!(
            "refusing non-directory {}",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            match fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => File::open(path.parent().unwrap())?.sync_all(),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => directory(path),
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    }
}
fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}
fn load(path: &Path) -> io::Result<Vec<u8>> {
    let f = open(path)?;
    if !f.metadata()?.is_file() || f.metadata()?.len() > MAX_LINE_BYTES as u64 {
        return Err(io::Error::other("invalid spool file"));
    }
    let mut bytes = vec![];
    f.take((MAX_LINE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_LINE_BYTES {
        return Err(io::Error::other("spool file exceeds line limit"));
    }
    Ok(bytes)
}
impl Spool {
    pub fn open(home: &Path, kind: &str, deadline: Instant) -> io::Result<Self> {
        directory(home)?;
        let root = home.join("spool");
        directory(&root)?;
        let dir = root.join(kind);
        directory(&dir)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(root.join("lock"))?;
        if !lock.metadata()?.is_file() {
            return Err(io::Error::other("invalid spool lock"));
        }
        Ok(Self {
            dir,
            lock,
            deadline,
        })
    }
    pub fn locked<T>(&self, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        // Wait only inside the caller's existing deadline. A competing
        // daemon replay is normal contention, not a disk persistence failure.
        loop {
            if Instant::now() >= self.deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "spool lock deadline elapsed",
                ));
            }
            if unsafe { libc::flock(self.lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                break;
            }
            let e = io::Error::last_os_error();
            if !matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) {
                return Err(e);
            }
            std::thread::sleep(
                Duration::from_millis(2)
                    .min(self.deadline.saturating_duration_since(Instant::now())),
            );
        }
        let result = f();
        unsafe {
            libc::flock(self.lock.as_raw_fd(), libc::LOCK_UN);
        }
        result
    }
    fn atomic(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let temp = self
            .dir
            .join(format!(".{}-{}.tmp", std::process::id(), super::uuid()?));
        let result = (|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            fs::rename(&temp, path)?;
            File::open(&self.dir)?.sync_all()
        })();
        if temp.exists() {
            let _ = fs::remove_file(temp);
        }
        result
    }
    pub fn publish(&self, request: ClaudeRequestData) -> io::Result<PathBuf> {
        let pending = Pending {
            version: 1,
            request,
        };
        let bytes = serde_json::to_vec(&pending)?;
        if bytes.len() > MAX_LINE_BYTES {
            return Err(io::Error::other("spool payload exceeds line limit"));
        }
        self.locked(|| {
            let counter = self.dir.join("counter");
            let seq = match load(&counter) {
                Ok(bytes) => std::str::from_utf8(&bytes)
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .ok_or_else(|| io::Error::other("invalid spool counter"))?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
                Err(e) => return Err(e),
            }
            .checked_add(1)
            .ok_or_else(|| io::Error::other("spool counter exhausted"))?;
            self.atomic(&counter, seq.to_string().as_bytes())?;
            let path = self.dir.join(format!("{seq:020}-{}.json", super::uuid()?));
            self.atomic(&path, &bytes)?;
            Ok(path)
        })
    }
    pub fn list(&self) -> io::Result<Vec<PathBuf>> {
        let mut paths = vec![];
        for e in fs::read_dir(&self.dir)? {
            let e = e?;
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".json") {
                paths.push(e.path());
            }
            // Temps have never been published. Under the home-wide lock no
            // writer can own them, so crash remnants are safe to remove.
            if name.starts_with('.') && name.ends_with(".tmp") && e.file_type()?.is_file() {
                fs::remove_file(e.path())?;
            }
        }
        paths.sort();
        Ok(paths)
    }
    pub fn read(&self, path: &Path) -> io::Result<Pending> {
        let p: Pending = serde_json::from_slice(&load(path)?)?;
        if p.version != 1 {
            return Err(io::Error::other("unsupported spool version"));
        }
        Ok(p)
    }
    pub fn remove(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)?;
        File::open(&self.dir)?.sync_all()
    }
}
