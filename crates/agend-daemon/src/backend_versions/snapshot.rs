//! Private, content-addressed executable copies. Published entries are never
//! overwritten or automatically removed: holders and hook helpers outlive a
//! daemon. This protects against normal upgrades of the source pathname, not
//! a hostile process with the same UID editing the private cache itself.
use super::{ExecutableBinding, fingerprint, identity};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

fn directory(path: &Path, mode: u32) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    // SAFETY: getuid only reads the caller's identity.
    if !meta.is_dir() || meta.uid() != unsafe { libc::getuid() } || meta.mode() & 0o777 != mode {
        return Err("executable cache directory is not private and owned".into());
    }
    Ok(())
}

fn verify(dir: &Path, digest: &str) -> Result<(PathBuf, ExecutableBinding), String> {
    directory(dir, 0o700)?;
    let path = dir.join("agend");
    let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    // SAFETY: getuid only reads the caller's identity.
    if !meta.is_file()
        || meta.uid() != unsafe { libc::getuid() }
        || meta.mode() & 0o777 != 0o500
        || meta.nlink() != 1
    {
        return Err("pinned executable changed; preserving cache and refusing launch".into());
    }
    let binding = ExecutableBinding::capture(&path)?;
    if binding.digest != digest || binding.identity != identity(&meta) {
        return Err("pinned executable changed; preserving cache and refusing launch".into());
    }
    Ok((path, binding))
}

pub(super) fn existing(home: &Path, digest: &str) -> Result<PathBuf, String> {
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid pinned executable digest".into());
    }
    let root = home.join("runtime-binaries");
    directory(&root, 0o700)?;
    verify(&root.join(digest), digest).map(|(path, _)| path)
}

pub(super) fn pin_bound(
    home: &Path,
    source: &Path,
    digest: &str,
) -> Result<(PathBuf, ExecutableBinding), String> {
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid pinned executable digest".into());
    }
    let root = home.join("runtime-binaries");
    match fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => File::open(home)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.to_string()),
    }
    directory(&root, 0o700)?;
    let dir = root.join(digest);
    match fs::symlink_metadata(&dir) {
        Ok(_) => return verify(&dir, digest),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let temp = root.join(format!(
        ".prepare-{}",
        crate::store::instances::new_session_id().map_err(|e| e.to_string())?
    ));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&temp)
        .map_err(|e| e.to_string())?;
    let created = fs::symlink_metadata(&temp).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(source)
            .map_err(|e| e.to_string())?;
        let before = input.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.len() > 1024 * 1024 * 1024 {
            return Err("source executable is not a bounded regular file".into());
        }
        let target = temp.join("agend");
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&target)
            .map_err(|e| e.to_string())?;
        let count = std::io::copy(
            &mut Read::by_ref(&mut input).take(before.len() + 1),
            &mut output,
        )
        .map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
        output
            .set_permissions(fs::Permissions::from_mode(0o500))
            .map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        drop(output);
        let after = input.metadata().map_err(|e| e.to_string())?;
        if count != before.len()
            || identity(&before) != identity(&after)
            || fingerprint(&target)? != digest
        {
            return Err("source executable changed while pinning; no snapshot published".into());
        }
        File::open(&temp)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        match fs::rename(&temp, &dir) {
            Ok(()) => {
                // Keep the owned directory writable for explicit teardown.
                // Immutability is the no-overwrite publication policy; the file is 0500.
                File::open(&dir)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| e.to_string())?;
                File::open(&root)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| e.to_string())?;
            }
            Err(e) if fs::symlink_metadata(&dir).is_ok() => {
                let _ = e;
            }
            Err(e) => return Err(e.to_string()),
        }
        verify(&dir, digest)
    })();
    // Only the exclusive temporary directory created above is removable here.
    // Published entries are kept even if a final fsync/verification fails.
    if fs::symlink_metadata(&temp)
        .is_ok_and(|m| m.is_dir() && (m.dev(), m.ino()) == (created.dev(), created.ino()))
    {
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
        match fs::remove_file(temp.join("agend")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("snapshot cleanup failed: {e}")),
        }
        fs::remove_dir(&temp).map_err(|e| format!("snapshot cleanup failed: {e}"))?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;
    use std::os::unix::fs::symlink;
    use std::process::Command;

    fn pin(home: &Path, source: &Path, digest: &str) -> Result<PathBuf, String> {
        pin_bound(home, source, digest).map(|(path, _)| path)
    }

    #[test]
    fn native_snapshot_survives_source_replacement_and_reuses_exact_bytes() {
        let root = TempDir::new("g13-executable-pin").unwrap();
        let source = root.path().join("source");
        fs::copy("/usr/bin/true", &source).unwrap();
        let digest = fingerprint(&source).unwrap();
        let (pinned, binding) = pin_bound(root.path(), &source, &digest).unwrap();
        assert_eq!(binding.digest_if_unchanged(&pinned).unwrap(), digest);
        assert!(binding.digest_if_unchanged(&source).is_err());
        let inode = fs::metadata(&pinned).unwrap().ino();
        let replacement = root.path().join("replacement");
        fs::copy("/usr/bin/false", &replacement).unwrap();
        fs::rename(replacement, &source).unwrap();
        assert!(!Command::new(&source).status().unwrap().success());
        assert!(Command::new(&pinned).status().unwrap().success());
        assert_eq!(pin(root.path(), &source, &digest).unwrap(), pinned);
        assert_eq!(fs::metadata(&pinned).unwrap().ino(), inode);
        // Explicit teardown only after the test's executable child has exited.
        fs::set_permissions(pinned.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn wrong_digest_and_redirected_cache_never_publish_or_modify_foreign_data() {
        let root = TempDir::new("g13-executable-pin-refuse").unwrap();
        let foreign = TempDir::new("g13-executable-pin-foreign").unwrap();
        let digest = fingerprint(Path::new("/usr/bin/true")).unwrap();
        symlink(foreign.path(), root.path().join("runtime-binaries")).unwrap();
        assert!(pin(root.path(), Path::new("/usr/bin/true"), &digest).is_err());
        assert_eq!(fs::read_dir(foreign.path()).unwrap().count(), 0);
        fs::remove_file(root.path().join("runtime-binaries")).unwrap();
        assert!(pin(root.path(), Path::new("/usr/bin/false"), &digest).is_err());
        assert_eq!(
            fs::read_dir(root.path().join("runtime-binaries"))
                .unwrap()
                .count(),
            0
        );
    }
}
