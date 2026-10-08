//! Explicit dedicated credentials; never discover, log, or update the source.
use super::files;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
const LIMIT: u64 = 1024 * 1024;

pub fn read(backend: &str, source: &Path) -> Result<Vec<u8>, String> {
    if !source.is_absolute() {
        return Err("canary auth file must be an absolute path".into());
    }
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(source)
        .map_err(|_| "cannot open canary auth file")?;
    let before = file
        .metadata()
        .map_err(|_| "cannot inspect canary auth file")?;
    // SAFETY: getuid only reads the current identity.
    if !before.is_file()
        || before.uid() != unsafe { libc::getuid() }
        || before.mode() & 0o077 != 0
        || before.nlink() != 1
        || before.len() > LIMIT
    {
        return Err(
            "canary auth file must be owned, private, singly linked, regular and at most 1 MiB"
                .into(),
        );
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read canary auth file")?;
    let after = file
        .metadata()
        .map_err(|_| "cannot inspect canary auth file")?;
    let current = fs::symlink_metadata(source).map_err(|_| "canary auth file changed")?;
    if bytes.len() as u64 > LIMIT
        || (
            before.dev(),
            before.ino(),
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (
            after.dev(),
            after.ino(),
            after.len(),
            after.mtime(),
            after.mtime_nsec(),
            after.ctime(),
            after.ctime_nsec(),
        )
        || (current.dev(), current.ino()) != (after.dev(), after.ino())
    {
        return Err("canary auth file changed while reading".into());
    }
    if backend == "claude" {
        let token = std::str::from_utf8(&bytes)
            .map_err(|_| "invalid Claude OAuth token file")?
            .trim_end_matches(['\r', '\n']);
        if token.is_empty() || token.len() > 16384 || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("Claude auth file must contain a single OAuth access token".into());
        }
        bytes = token.as_bytes().to_vec();
    } else {
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| "canary auth file must contain a JSON object")?;
        if !value.is_object() {
            return Err("canary auth file must contain a JSON object".into());
        }
    }
    Ok(bytes)
}

pub fn install(home: &Path, backend: &str, bytes: &[u8]) -> Result<(), String> {
    let relative = match backend {
        "codex" => "probe-home/.codex/auth.json",
        "opencode" => "opencode/canary/data/opencode/auth.json",
        "claude" => "canary-auth/claude-oauth-token",
        _ => return Err("unsupported credential backend".into()),
    };
    let path = home.join(relative);
    let mut parent = home.to_path_buf();
    for component in Path::new(relative).parent().unwrap().components() {
        parent.push(component);
        files::private_dir(&parent)?;
    }
    files::publish(&path, 0o600, false, |out| out.write_all(bytes))?;
    if backend == "codex" {
        files::publish(&parent.join("config.toml"), 0o600, false, |out| {
            out.write_all(b"cli_auth_credentials_store = \"file\"\n")
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn source(root: &Path, bytes: &[u8]) -> std::path::PathBuf {
        let path = root.join("source");
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    #[test]
    fn explicit_credentials_are_copied_privately_without_updating_the_source() {
        for (backend, bytes, destination) in [
            (
                "codex",
                b"{\"OPENAI_API_KEY\":\"test-only\"}".as_slice(),
                "probe-home/.codex/auth.json",
            ),
            (
                "opencode",
                b"{\"test\":{\"type\":\"api\",\"key\":\"test-only\"}}".as_slice(),
                "opencode/canary/data/opencode/auth.json",
            ),
            (
                "claude",
                b"test-only-oauth\n".as_slice(),
                "canary-auth/claude-oauth-token",
            ),
        ] {
            let root = TempDir::new("g13-canary-auth").unwrap();
            let src = source(root.path(), bytes);
            let home = root.path().join("lab");
            files::private_dir(&home).unwrap();
            let copied = read(backend, &src).unwrap();
            install(&home, backend, &copied).unwrap();
            assert_eq!(fs::read(home.join(destination)).unwrap(), copied);
            assert_eq!(
                fs::metadata(home.join(destination)).unwrap().mode() & 0o777,
                0o600
            );
            assert_eq!(fs::read(&src).unwrap(), bytes);
            assert!(
                install(&home, backend, &copied).is_err(),
                "never overwrite credentials"
            );
            if backend == "codex" {
                assert_eq!(
                    fs::read_to_string(home.join("probe-home/.codex/config.toml")).unwrap(),
                    "cli_auth_credentials_store = \"file\"\n"
                );
            }
            // Exercise the real OpenCode layout producer: preparing the holder
            // must preserve the credentials staged before InstanceAdd.
            if backend == "opencode" {
                agend_daemon::driver::opencode::launch::Layout::new(&home, "canary")
                    .unwrap()
                    .prepare()
                    .unwrap();
                assert_eq!(fs::read(home.join(destination)).unwrap(), copied);
            }
            fs::remove_dir_all(&home).unwrap();
            assert_eq!(fs::read(src).unwrap(), bytes);
        }
    }

    #[test]
    fn unsafe_sources_and_invalid_contents_are_refused_without_echoing_secrets() {
        let root = TempDir::new("g13-canary-auth-refuse").unwrap();
        let src = source(root.path(), b"DO-NOT-ECHO-THIS");
        assert!(
            !read("codex", &src)
                .unwrap_err()
                .contains("DO-NOT-ECHO-THIS")
        );
        fs::set_permissions(&src, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read("claude", &src).is_err());
        fs::set_permissions(&src, fs::Permissions::from_mode(0o600)).unwrap();
        let alias = root.path().join("alias");
        symlink(&src, &alias).unwrap();
        assert!(read("claude", &alias).is_err());
        fs::remove_file(alias).unwrap();
        fs::hard_link(&src, root.path().join("hardlink")).unwrap();
        assert!(read("claude", &src).is_err());
        fs::remove_file(root.path().join("hardlink")).unwrap();
        fs::write(&src, b"first\nsecond").unwrap();
        assert!(read("claude", &src).is_err());
        fs::OpenOptions::new()
            .write(true)
            .open(&src)
            .unwrap()
            .set_len(LIMIT + 1)
            .unwrap();
        assert!(read("codex", &src).is_err());
        assert!(read("codex", Path::new("relative")).is_err());
    }
}
