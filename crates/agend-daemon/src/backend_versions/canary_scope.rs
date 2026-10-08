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
        || instance.id != "canary"
        || instance.backend.as_str() != scope.artifact.backend
        || Path::new(&instance.program) != Path::new(&scope.program)
        || Path::new(&instance.working_directory)
            .canonicalize()
            .map_err(|e| e.to_string())?
            != home.join("workspace")
        || instance.args != scope.args
        || instance.delivery != "push"
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
    Ok(Some(scope.artifact.version))
}
