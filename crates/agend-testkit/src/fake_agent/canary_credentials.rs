//! Native fake-only assertion of credential delivery, never real authentication.
use std::{fs, path::PathBuf};

pub fn check(backend: &str) -> Result<(), String> {
    let Some(home) = std::env::var_os("AGEND_HOME").map(PathBuf::from) else {
        return Ok(());
    };
    if !home.join("canary-scope.json").is_file() {
        return Ok(());
    }
    let home = home.canonicalize().map_err(|_| "fixture home missing")?;
    // The model marker comes from CLI args, independently of credential staging.
    // Removing the install call must fail authenticated native tests, not skip.
    let scope: serde_json::Value = serde_json::from_slice(
        &fs::read(home.join("canary-scope.json")).map_err(|_| "fixture scope missing")?,
    )
    .map_err(|_| "fixture scope invalid")?;
    let required = scope["args"].as_array().is_some_and(|args| {
        args.iter()
            .any(|v| matches!(v.as_str(), Some("test/canary" | "model=\"test/canary\"")))
    });
    let user = home.join("probe-home");
    let path = match backend {
        "claude" => home.join("canary-auth/claude-oauth-token"),
        "codex" => user.join(".codex/auth.json"),
        "opencode" => home.join("opencode/canary/data/opencode/auth.json"),
        _ => return Err("unknown fixture backend".into()),
    };
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => return Ok(()),
        Err(_) => return Err("fixture credential unreadable".into()),
    };
    // Opt in only for the synthetic credential producer used by native_canary.
    let expected = if backend == "claude" {
        b"test-only-token".as_slice()
    } else {
        b"{\"test-only\":true}".as_slice()
    };
    if bytes != expected {
        return Err("fixture only accepts synthetic canary credentials".into());
    }
    let expect_env = |name: &str, value: &str| -> Result<(), String> {
        if std::env::var(name).ok().as_deref() == Some(value) {
            Ok(())
        } else {
            Err(format!("fixture isolated {name} mismatch"))
        }
    };
    expect_env("HOME", user.to_str().ok_or("fixture non-UTF8 home")?)?;
    match backend {
        "claude" => {
            expect_env("CLAUDE_CONFIG_DIR", user.join(".claude").to_str().unwrap())?;
            expect_env("CLAUDE_CODE_OAUTH_TOKEN", "test-only-token")?;
        }
        "codex" => {
            expect_env("CODEX_HOME", user.join(".codex").to_str().unwrap())?;
            if fs::read(user.join(".codex/config.toml")).map_err(|_| "fixture config missing")?
                != b"cli_auth_credentials_store = \"file\"\n"
            {
                return Err("fixture credential store is not file".into());
            }
        }
        "opencode" => {
            let data = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .and_then(|p| p.canonicalize().ok());
            if data != Some(home.join("opencode/canary/data")) {
                return Err("fixture isolated XDG_DATA_HOME mismatch".into());
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}
