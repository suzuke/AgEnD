//! Exclusive publication and ownership checks for the local installation.
//! Unknown or edited artifacts are preserved, never adopted or overwritten.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use agend_core::setup::service::Installation;
use sha2::{Digest, Sha256};

pub const RECORD: &str = "installation.json";

pub fn canonical_location(path: &Path) -> Result<PathBuf, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => path.canonicalize().map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or("path has no parent")?;
            let name = path.file_name().ok_or("path has no final component")?;
            Ok(canonical_location(parent)?.join(name))
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn private_dir(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && owned(&m) && m.mode() & 0o022 == 0 => Ok(()),
        Ok(_) => Err(format!(
            "{} must be an owned directory, not writable by others",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or("directory has no parent")?;
            // Existing ancestors may be shared OS roots (e.g. /tmp). The final
            // directory and each missing component must be private and owned.
            if !parent.is_dir() {
                private_dir(parent)?;
            }
            fs::DirBuilder::new()
                .mode(0o700)
                .create(path)
                .or_else(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        Ok(())
                    } else {
                        Err(e)
                    }
                })
                .map_err(|e| e.to_string())?;
            private_dir(path)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Inspect existing children without following links or creating directories.
/// Call for every component below the canonical installation home.
pub fn existing_directory(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && owned(&m) && m.mode() & 0o022 == 0 => Ok(true),
        Ok(_) => Err(format!(
            "{} is not an owned real directory; preserving it",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

pub fn holder_socket(path: &Path) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.file_type().is_socket() || !owned(&m) || m.nlink() != 1 {
        return Err(format!(
            "{} is not an owned, singly linked socket; refusing shutdown",
            path.display()
        ));
    }
    Ok(())
}

fn owned(meta: &fs::Metadata) -> bool {
    // SAFETY: getuid only returns this process's real user id.
    meta.uid() == unsafe { libc::getuid() }
}

/// Serialize installers with a kernel lock, released even on process death.
/// Keep the lock inode after uninstall so waiters cannot acquire a replaced lock.
pub fn lock(dir: &Path) -> Result<InstallLock, String> {
    private_dir(dir)?;
    let path = dir.join("install.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || !owned(&meta) || meta.mode() & 0o077 != 0 {
        return Err("installation lock must be an owned private regular file".into());
    }
    // SAFETY: fd is owned; the returned guard releases this flock.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("another installation operation is running; retry after it finishes".into());
    }
    Ok(InstallLock(file))
}

/// Explicitly unlock before close: fork may briefly retain the same open-file
/// description until exec, even though Rust opens descriptors with CLOEXEC.
pub struct InstallLock(File);
impl Drop for InstallLock {
    fn drop(&mut self) {
        // SAFETY: this guard owns the live descriptor and its exclusive lock.
        while unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) } != 0 {
            if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                break;
            }
        }
    }
}

pub fn regular(path: &Path) -> Result<Option<File>, String> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let m = file.metadata().map_err(|e| e.to_string())?;
    if !m.is_file() || !owned(&m) || m.mode() & 0o022 != 0 {
        return Err(format!(
            "{} must be an owned regular file, not writable by others",
            path.display()
        ));
    }
    Ok(Some(file))
}

pub fn load(dir: &Path) -> Result<Option<Installation>, String> {
    let Some(file) = regular(&dir.join(RECORD))? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("installation record exceeds 64 KiB".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "invalid installation record; nothing changed".into())
}

pub fn save(dir: &Path, record: &Installation, replace: bool) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(record).map_err(|e| e.to_string())?;
    publish(&dir.join(RECORD), 0o600, replace, |f| f.write_all(&bytes))
}

/// The caller holds the installation lock and has already checked ownership.
pub fn publish(
    path: &Path,
    mode: u32,
    replace: bool,
    write: impl FnOnce(&mut File) -> std::io::Result<()>,
) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or("publication has no parent")?;
    let temp = parent.join(format!(
        ".agend-install-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        write(&mut f)
            .and_then(|()| f.sync_all())
            .map_err(|e| e.to_string())?;
        if replace {
            // Replacing the private record, never a binary or external unit.
            if regular(path)?.is_none() {
                return Err("installation record disappeared".into());
            }
            fs::rename(&temp, path).map_err(|e| e.to_string())?;
        } else {
            fs::hard_link(&temp, path)
                .map_err(|e| format!("cannot publish {} exclusively: {e}", path.display()))?;
        }
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    })();
    drop(f);
    match fs::remove_file(&temp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot clean installation temporary: {e}")),
    }
    result
}

pub fn sha256(mut file: impl Read) -> Result<String, String> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn matches_bytes(path: &Path, expected: &[u8]) -> Result<bool, String> {
    let Some(file) = regular(path)? else {
        return Ok(false);
    };
    let mut bytes = Vec::new();
    file.take(expected.len() as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes != expected {
        return Err(format!("{} changed; preserving it", path.display()));
    }
    Ok(true)
}

pub fn matches_program(record: &Installation) -> Result<bool, String> {
    let Some(file) = regular(Path::new(&record.spec.program))? else {
        return Ok(false);
    };
    if sha256(file)? != record.program_sha256 {
        return Err("managed executable changed; preserving it".into());
    }
    Ok(true)
}

#[cfg(test)]
mod lock_tests {
    use super::*;

    #[test]
    fn dropping_install_guard_unlocks_even_with_a_duplicated_description() {
        let root = agend_testkit::tempdir::TempDir::new("g13-lock-fork").unwrap();
        let guard = lock(root.path()).unwrap();
        let inherited = guard.0.try_clone().unwrap();
        assert!(lock(root.path()).is_err());
        drop(guard);
        let next = lock(root.path()).expect("inherited description must not retain the lock");
        assert!(lock(root.path()).is_err());
        drop(inherited);
        assert!(
            lock(root.path()).is_err(),
            "closing old description must not unlock new owner"
        );
        drop(next);
        assert!(lock(root.path()).is_ok());
    }
}
