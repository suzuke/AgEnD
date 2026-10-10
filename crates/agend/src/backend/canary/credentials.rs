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
    if backend == "codex" {
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "invalid Codex credentials")?;
        if value.get("tokens").is_some() {
            if value["auth_mode"] != "chatgpt" {
                return Err("Codex OAuth credentials require chatgpt mode".into());
            }
            let token = value["tokens"]["access_token"].as_str().unwrap_or("");
            let account = value["tokens"]["account_id"].as_str().unwrap_or("");
            if token.is_empty()
                || account.is_empty()
                || !token.bytes().all(|b| b.is_ascii_graphic())
                || !account.bytes().all(|b| b.is_ascii_graphic())
            {
                return Err("Codex OAuth credentials require access token and account id".into());
            }
            // Only these two fields cross into the isolated home. In particular,
            // never publish the source's refresh token or ID token.
            bytes = serde_json::to_vec(&serde_json::json!({
                "type": "chatgptAuthTokens", "accessToken": token,
                "chatgptAccountId": account
            }))
            .map_err(|_| "cannot prepare Codex external credentials")?;
        } else {
            let key = value["OPENAI_API_KEY"].as_str().unwrap_or("");
            if key.is_empty() || !key.bytes().all(|b| b.is_ascii_graphic()) {
                return Err("Codex credentials require native chatgpt tokens or an API key".into());
            }
            bytes = serde_json::to_vec(&serde_json::json!({"OPENAI_API_KEY": key}))
                .map_err(|_| "cannot prepare Codex API credentials")?;
        }
    }
    Ok(bytes)
}

pub fn install(home: &Path, backend: &str, bytes: &[u8]) -> Result<(), String> {
    let external = backend == "codex"
        && serde_json::from_slice::<serde_json::Value>(bytes)
            .is_ok_and(|v| v["type"] == "chatgptAuthTokens");
    let relative = match backend {
        "codex" if external => "canary-auth/codex-external.json",
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
        let config = home.join("probe-home/.codex");
        files::private_dir(&home.join("probe-home"))?;
        files::private_dir(&config)?;
        files::publish(&config.join("config.toml"), 0o600, false, |out| {
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
    fn codex_oauth_snapshot_excludes_refresh_and_id_tokens_and_stored_auth() {
        let root = TempDir::new("g13-codex-access-only").unwrap();
        let original = br#"{"auth_mode":"chatgpt","tokens":{"access_token":"synthetic-access","account_id":"synthetic-account","refresh_token":"NEVER-COPY-REFRESH","id_token":"NEVER-COPY-ID"}}"#;
        let src = source(root.path(), original);
        let home = root.path().join("lab");
        files::private_dir(&home).unwrap();
        let snapshot = read("codex", &src).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&snapshot).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert_eq!(value["accessToken"], "synthetic-access");
        assert!(!String::from_utf8_lossy(&snapshot).contains("NEVER-COPY"));
        install(&home, "codex", &snapshot).unwrap();
        assert!(!home.join("probe-home/.codex/auth.json").exists());
        assert_eq!(
            fs::read(home.join("canary-auth/codex-external.json")).unwrap(),
            snapshot
        );
        assert_eq!(fs::read(&src).unwrap(), original);
        assert!(install(&home, "codex", &snapshot).is_err());
        fs::write(
            &src,
            br#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"NEVER-ECHO"}}"#,
        )
        .unwrap();
        let error = read("codex", &src).unwrap_err();
        assert!(!error.contains("NEVER-ECHO"));
    }

    #[test]
    fn codex_api_projection_never_copies_unknown_secrets() {
        let root = TempDir::new("g13-codex-api-projection").unwrap();
        let src = source(root.path(), br#"{"OPENAI_API_KEY":"test-only","refresh_token":"NEVER-COPY","id_token":"NEVER-COPY"}"#);
        let snapshot = read("codex", &src).unwrap();
        assert_eq!(snapshot, br#"{"OPENAI_API_KEY":"test-only"}"#);
        fs::write(&src, br#"{"type":"chatgptAuthTokens","accessToken":"test","chatgptAccountId":"test","refresh_token":"NEVER-COPY"}"#).unwrap();
        assert!(read("codex", &src).is_err());
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
