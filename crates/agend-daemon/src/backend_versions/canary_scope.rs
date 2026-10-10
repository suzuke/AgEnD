//! Explicit canary-only version observation, never a fleet admission report.
use super::*;
use agend_core::runtime_records::Instance;
use agend_core::setup::backend::CanaryScope as Scope;
use std::io::Write;

const FILE: &str = "canary-scope.json";
/// Called only by the explicit canary runner before starting its private daemon.
pub fn create(
    home: &Path,
    source: &Path,
    artifact: &ImportedBackend,
    args: &[String],
) -> Result<(), String> {
    directory(home)?;
    let home = home.canonicalize().map_err(|e| e.to_string())?;
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    if home == source || inspect(&source, &artifact.backend, &artifact.version)? != *artifact {
        return Err("canary scope requires a separate home and exact imported artifact".into());
    }
    let meta = fs::metadata(&home).map_err(|e| e.to_string())?;
    let scope = Scope {
        args: args.to_vec(),
        program: source
            .join("backends")
            .join(&artifact.backend)
            .join(&artifact.version)
            .join("program")
            .to_str()
            .ok_or("non-UTF8 program")?
            .into(),
        source: source.to_str().ok_or("non-UTF8 source")?.into(),
        home: home.to_str().ok_or("non-UTF8 home")?.into(),
        device: meta.dev(),
        inode: meta.ino(),
        artifact: artifact.clone(),
    };
    let bytes = serde_json::to_vec(&scope).map_err(|e| e.to_string())?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(home.join(FILE))
        .and_then(|mut f| f.write_all(&bytes))
        .map_err(|e| e.to_string())
}

/// Missing scope retains the legacy version policy. A present scope must match
/// this exact private home, instance, workspace and still-intact imported bytes.
pub fn expected(home: &Path, instance: &Instance) -> Result<Option<String>, String> {
    let Some(scope) = read_scope(home)? else {
        return Ok(None);
    };
    let home = home.canonicalize().map_err(|e| e.to_string())?;
    if instance.id != "canary"
        || instance.backend.as_str() != scope.artifact.backend
        || Path::new(&instance.program) != Path::new(&scope.program)
        || Path::new(&instance.working_directory)
            .canonicalize()
            .map_err(|e| e.to_string())?
            != home.join("workspace")
        || instance.args != scope.args
        || instance.delivery != "push"
    {
        return Err("canary scope identity mismatch".into());
    }
    Ok(Some(scope.artifact.version))
}

fn read_scope(home: &Path) -> Result<Option<Scope>, String> {
    let path = home.join(FILE);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    directory(home)?;
    let mut bytes = Vec::new();
    regular(&path)?
        .take(16385)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16384 {
        return Err("canary scope exceeds limit".into());
    }
    let scope: Scope = serde_json::from_slice(&bytes).map_err(|_| "invalid canary scope")?;
    let home = home.canonicalize().map_err(|e| e.to_string())?;
    let meta = fs::metadata(&home).map_err(|e| e.to_string())?;
    if Path::new(&scope.home) != home
        || scope.device != meta.dev()
        || scope.inode != meta.ino()
        || Path::new(&scope.source) == home
        || Path::new(&scope.program)
            != Path::new(&scope.source)
                .join("backends")
                .join(&scope.artifact.backend)
                .join(&scope.artifact.version)
                .join("program")
        || inspect(
            Path::new(&scope.source),
            &scope.artifact.backend,
            &scope.artifact.version,
        )? != scope.artifact
    {
        return Err("canary scope identity mismatch".into());
    }
    Ok(Some(scope))
}

/// Only an identity-bound private canary home can provision credential variables.
/// Never inherit credentials or config-directory overrides from daemon env.
pub fn isolate_environment(
    home: &Path,
    id: &str,
    backend: &str,
    workspace: &Path,
    env: &mut std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let Some(scope) = read_scope(home)? else {
        return Ok(());
    };
    let home = home.canonicalize().map_err(|e| e.to_string())?;
    if id != "canary"
        || backend != scope.artifact.backend
        || workspace.canonicalize().map_err(|e| e.to_string())? != home.join("workspace")
    {
        return Err("canary credential scope mismatch".into());
    }
    let user = home.join("probe-home");
    directory(&user)?;
    env.insert("HOME".into(), user.display().to_string());
    match backend {
        "codex" => {
            env.insert(
                "CODEX_HOME".into(),
                user.join(".codex").display().to_string(),
            );
        }
        "claude" => {
            env.insert(
                "CLAUDE_CONFIG_DIR".into(),
                user.join(".claude").display().to_string(),
            );
            let path = home.join("canary-auth/claude-oauth-token");
            match fs::symlink_metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(_) => return Err("cannot inspect canary credential".into()),
                Ok(_) => {
                    directory(&home.join("canary-auth"))?;
                    let file = regular(&path).map_err(|_| "invalid canary credential file")?;
                    if file
                        .metadata()
                        .map_err(|_| "cannot inspect canary credential")?
                        .mode()
                        & 0o077
                        != 0
                    {
                        return Err("canary credential must be private".into());
                    }
                    let mut token = String::new();
                    file.take(16385)
                        .read_to_string(&mut token)
                        .map_err(|_| "cannot read canary credential")?;
                    if token.is_empty()
                        || token.len() > 16384
                        || !token.bytes().all(|b| b.is_ascii_graphic())
                    {
                        return Err("invalid canary OAuth token".into());
                    }
                    env.insert("CLAUDE_CODE_OAUTH_TOKEN".into(), token);
                }
            }
        }
        _ => (),
    }
    Ok(())
}

/// Access-only snapshot for an explicitly bound Codex canary. No shared-home
/// discovery and no refresh credentials are supported by this interface.
pub(crate) fn codex_external_auth(
    home: &Path,
    instance: &Instance,
) -> Result<Option<serde_json::Value>, String> {
    let path = home.join("canary-auth/codex-external.json");
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("cannot inspect external Codex credential".into()),
        Ok(_) => (),
    }
    if instance.backend.as_str() != "codex" || expected(home, instance)?.is_none() {
        return Err("external Codex credential requires a bound canary".into());
    }
    directory(&home.join("canary-auth"))?;
    let file = regular(&path).map_err(|_| "invalid external Codex credential file")?;
    let meta = file
        .metadata()
        .map_err(|_| "cannot inspect external Codex credential")?;
    if meta.mode() & 0o077 != 0 || meta.nlink() != 1 {
        return Err("external Codex credential must be private and singly linked".into());
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read external Codex credential")?;
    if bytes.len() > 1024 * 1024 {
        return Err("external Codex credential exceeds limit".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "invalid external Codex credential")?;
    let valid = value.as_object().is_some_and(|v| v.len() == 3)
        && value["type"] == "chatgptAuthTokens"
        && ["accessToken", "chatgptAccountId"].iter().all(|key| {
            value[key]
                .as_str()
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_graphic()))
        });
    if !valid {
        return Err("invalid external Codex credential fields".into());
    }
    if home.join("probe-home/.codex/auth.json").exists() {
        return Err("external Codex credentials cannot coexist with stored auth".into());
    }
    Ok(Some(value))
}
