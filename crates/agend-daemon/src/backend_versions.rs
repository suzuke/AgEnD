//! Read-only verification of operator-imported backend artifacts.
//! Managed launches require a canary bound to the startup executable fingerprint.
use agend_core::setup::backend::{ImportedBackend, valid_version};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
pub(crate) mod launcher;
mod running;
mod snapshot;

/// Read-only ownership check for service reconciliation of its pinned shims.
pub fn verified_pinned_executable(home: &Path, digest: &str) -> Result<std::path::PathBuf, String> {
    snapshot::existing(home, digest)
}

/// Captured once when the runtime is created, never refreshed at launch time.
pub struct ExecutableBinding {
    digest: String,
    identity: (u64, u64, u64, i64, i64, i64, i64),
}
fn identity(m: &fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
impl ExecutableBinding {
    /// Publish an immutable-by-policy launcher copy with the running image's
    /// digest. The private cache outlives the daemon, like its holders.
    pub fn pin_running(home: &Path, source: &Path) -> Result<(std::path::PathBuf, Self), String> {
        let running = Self::capture_running(source)?;
        // Snapshot verification already hashes the published file and binds
        // its identity. Reuse that proof instead of hashing it a third time.
        snapshot::pin_bound(home, source, &running.digest)
    }
    pub fn capture_running(path: &Path) -> Result<Self, String> {
        running::matches(path)?;
        let binding = Self::capture(path)?;
        running::matches(path)?;
        binding.digest_if_unchanged(path)?;
        Ok(binding)
    }
    pub fn capture(path: &Path) -> Result<Self, String> {
        let before = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let digest = fingerprint(path)?;
        let after = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !before.is_file() || identity(&before) != identity(&after) {
            return Err("AgEnD executable changed during startup fingerprinting".into());
        }
        Ok(Self {
            digest,
            identity: identity(&after),
        })
    }
    pub fn digest_if_unchanged(&self, path: &Path) -> Result<&str, String> {
        let now = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !now.is_file() || identity(&now) != self.identity {
            return Err("AgEnD executable changed since runtime startup; restart before admitting a backend".into());
        }
        Ok(&self.digest)
    }
}

fn owned(meta: &fs::Metadata) -> bool {
    // SAFETY: getuid has no side effects.
    meta.uid() == unsafe { libc::getuid() } && meta.mode() & 0o022 == 0
}
fn directory(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_dir() || !owned(&meta) {
        return Err("backend storage must use owned real directories".into());
    }
    Ok(())
}
fn regular(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || !owned(&meta) || meta.nlink() != 1 {
        return Err("backend artifact must be an owned singly linked regular file".into());
    }
    Ok(file)
}

/// Reads only; does not execute the artifact or trust its claimed version.
pub fn inspect(home: &Path, backend: &str, version: &str) -> Result<ImportedBackend, String> {
    if agend_core::model::Backend::parse(backend).is_none() || !valid_version(version) {
        return Err("invalid backend identity".into());
    }
    let root = home.join("backends");
    let parent = root.join(backend);
    let dir = parent.join(version);
    for path in [home, &root, &parent, &dir] {
        directory(path)?;
    }
    let mut bytes = Vec::new();
    regular(&dir.join("import.json"))?
        .take(16385)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16384 {
        return Err("import manifest exceeds 16 KiB".into());
    }
    let record: ImportedBackend =
        serde_json::from_slice(&bytes).map_err(|_| "invalid import manifest")?;
    if record.format != 1
        || record.backend != backend
        || record.version != version
        || record.bytes > 1024 * 1024 * 1024
        || record.sha256.len() != 64
        || !record.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("import manifest identity mismatch".into());
    }
    let file = regular(&dir.join("program"))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if meta.len() != record.bytes || meta.mode() & 0o111 == 0 {
        return Err("imported executable changed; activation refused".into());
    }
    let mut digest = Sha256::new();
    let mut reader = file.take(record.bytes + 1);
    let mut buffer = [0u8; 65536];
    let mut count = 0;
    loop {
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count += n as u64;
        digest.update(&buffer[..n]);
    }
    if count != record.bytes || format!("{:x}", digest.finalize()) != record.sha256 {
        return Err("imported executable changed; activation refused".into());
    }
    Ok(record)
}

/// Recognizes both the configured home spelling and canonical program aliases.
/// Legacy external programs retain their existing startup policy.
pub fn check_launch(
    home: &Path,
    backend: &str,
    program: &str,
    cwd: &Path,
    search_path: &str,
    agend_executable: &Path,
    binding: Result<&ExecutableBinding, &String>,
) -> Result<Option<ImportedBackend>, String> {
    let Some(artifact) = inspect_launch(home, backend, program, cwd, search_path)? else {
        return Ok(None);
    };
    if fs::symlink_metadata(
        home.join("backends")
            .join(backend)
            .join(&artifact.version)
            .join("canary.json"),
    )
    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        return Err(
            "managed backend canary not run; imported version is not admitted to the fleet".into(),
        );
    }
    let binding =
        binding.map_err(|e| format!("startup executable fingerprint unavailable: {e}"))?;
    let digest = binding.digest_if_unchanged(agend_executable)?;
    verify_canary(home, backend, &artifact.version, digest)?;
    Ok(Some(artifact))
}

/// Identify immutable managed bytes for an inherited holder, without admitting
/// any new execution under the current daemon build.
pub fn inspect_launch(
    home: &Path,
    backend: &str,
    program: &str,
    cwd: &Path,
    search_path: &str,
) -> Result<Option<ImportedBackend>, String> {
    let root = home
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join("backends");
    let requested = Path::new(program);
    let via_path = || {
        search_path
            .split(':')
            .map(|entry| {
                let directory = Path::new(entry);
                if directory.is_absolute() {
                    directory.join(requested)
                } else {
                    cwd.join(directory).join(requested)
                }
            })
            .find(|candidate| executable(candidate))
    };
    let selected = if requested.is_absolute() {
        requested.to_owned()
    } else if program.starts_with("./") || program.starts_with("../") {
        cwd.join(requested)
    } else {
        // Shell launchers treat other slash paths as cwd-relative; portable-pty
        // searches PATH. Refuse an ambiguous spelling rather than admit one path
        // and execute another. An operator can always use an absolute path.
        if program.contains('/') {
            return Err(
                "ambiguous relative backend program; use an absolute path or ./ prefix".into(),
            );
        }
        via_path().unwrap_or_else(|| cwd.join(requested))
    };
    let path = selected.as_path();
    let resolved = path.canonicalize().ok();
    let relative = path
        .strip_prefix(home.join("backends"))
        .ok()
        .or_else(|| path.strip_prefix(&root).ok())
        .or_else(|| resolved.as_deref().and_then(|p| p.strip_prefix(&root).ok()));
    let Some(relative) = relative else {
        return Ok(None);
    };
    let parts: Vec<_> = relative.components().collect();
    if parts.len() != 3 || parts[0].as_os_str() != backend || parts[2].as_os_str() != "program" {
        return Err("managed backend path does not match its backend identity".into());
    }
    let version = parts[1]
        .as_os_str()
        .to_str()
        .ok_or("invalid backend version path")?;
    let artifact = inspect(home, backend, version)?;
    Ok(Some(artifact))
}

fn executable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path_string) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: CString is NUL-terminated and lives through the read-only access call.
    fs::metadata(path).is_ok_and(|m| m.is_file())
        && unsafe { libc::access(path_string.as_ptr(), libc::X_OK) == 0 }
}

/// Hash an explicitly selected AgEnD executable with a bounded read. Callers
/// that keep running must retain this fingerprint rather than hash a replacement
/// at the same pathname and mistake it for their loaded executable.
pub fn fingerprint(program: &Path) -> Result<String, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(program)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > 1024 * 1024 * 1024 {
        return Err("invalid AgEnD executable for canary binding".into());
    }
    let mut reader = file.take(meta.len() + 1);
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    let mut count = 0;
    loop {
        let size = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 {
            break;
        }
        count += size as u64;
        digest.update(&buffer[..size]);
    }
    if count != meta.len() {
        return Err("AgEnD executable changed while hashing".into());
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Verify retained evidence against freshly inspected artifact bytes and a
/// caller's explicit AgEnD fingerprint. This is not activation or fleet mutation.
pub fn verify_canary(
    home: &Path,
    backend: &str,
    version: &str,
    agend_sha256: &str,
) -> Result<agend_core::setup::backend::canary::CanaryReport, String> {
    let artifact = inspect(home, backend, version)?;
    let path = home
        .join("backends")
        .join(backend)
        .join(version)
        .join("canary.json");
    let mut bytes = Vec::new();
    regular(&path)?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("canary report exceeds 64 KiB".into());
    }
    let report: agend_core::setup::backend::canary::CanaryReport =
        serde_json::from_slice(&bytes).map_err(|_| "invalid canary report")?;
    report.validate(
        &artifact,
        agend_sha256,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;

    #[test]
    fn running_binding_rejects_a_different_inode_with_identical_bytes() {
        let exe = std::env::current_exe().unwrap();
        assert!(ExecutableBinding::capture_running(&exe).is_ok());
        let dir = TempDir::new("g13-loaded-binding").unwrap();
        let copy = dir.path().join("agend");
        fs::copy(&exe, &copy).unwrap();
        assert!(ExecutableBinding::capture(&copy).is_ok());
        assert!(ExecutableBinding::capture_running(&copy).is_err());
    }

    #[test]
    fn startup_binding_never_adopts_a_replacement_even_with_identical_bytes() {
        let dir = TempDir::new("g13-boot-binding").unwrap();
        let exe = dir.path().join("agend");
        fs::copy("/usr/bin/true", &exe).unwrap();
        let binding = ExecutableBinding::capture(&exe).unwrap();
        assert_eq!(
            binding.digest_if_unchanged(&exe).unwrap(),
            fingerprint(&exe).unwrap()
        );
        let replacement = dir.path().join("replacement");
        fs::copy(&exe, &replacement).unwrap();
        fs::rename(replacement, &exe).unwrap();
        assert!(
            binding
                .digest_if_unchanged(&exe)
                .unwrap_err()
                .contains("changed since runtime startup")
        );
        assert!(
            ExecutableBinding::capture(&exe)
                .unwrap()
                .digest_if_unchanged(&exe)
                .is_ok()
        );
        fs::write(&exe, b"different bytes").unwrap();
        assert!(binding.digest_if_unchanged(&exe).is_err());
    }
}
