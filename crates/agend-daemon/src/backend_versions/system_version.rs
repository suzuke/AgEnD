//! Observe an external program with daemon launch resolution and bounded I/O.
use super::{ExecutableBinding, executable, inspect_launch, version_probe};
use agend_core::{model::Backend, setup::backend::observation::SystemBackendVersion};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn select(program: &str, cwd: &Path, path: &str) -> Result<PathBuf, String> {
    if !cwd.is_absolute() || program.is_empty() || program.contains('\0') {
        return Err("version probe requires an absolute cwd and a program".into());
    }
    let requested = Path::new(program);
    let selected = if requested.is_absolute() {
        requested.to_owned()
    } else if program.starts_with("./") || program.starts_with("../") {
        cwd.join(requested)
    } else if program.contains('/') {
        return Err("ambiguous relative backend program; use an absolute path or ./ prefix".into());
    } else {
        path.split(':')
            .map(|dir| {
                let dir = Path::new(dir);
                if dir.is_absolute() {
                    dir.join(requested)
                } else {
                    cwd.join(dir).join(requested)
                }
            })
            .find(|candidate| executable(candidate))
            .ok_or("configured backend is absent from daemon PATH")?
    };
    if !executable(&selected) {
        return Err("configured backend is not an executable regular file".into());
    }
    Ok(selected)
}

/// None means managed bytes: their admission and drift checks belong to the
/// existing artifact verifier. Never execute modified managed bytes as a probe.
pub fn observe(
    home: &Path,
    backend: Backend,
    program: &str,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<Option<SystemBackendVersion>, String> {
    let path = environment
        .get("PATH")
        .ok_or("version probe requires daemon PATH")?;
    if inspect_launch(home, backend.as_str(), program, cwd, path)?.is_some() {
        return Ok(None);
    }
    let selected = select(program, cwd, path)?;
    let resolved = selected
        .canonicalize()
        .map_err(|_| "cannot resolve configured backend")?;
    let binding = ExecutableBinding::capture(&resolved)?;
    let output = version_probe::version_line_in(&selected, cwd, environment)?;
    // Re-resolve PATH and symlinks after the process finishes. A changed
    // selection or file identity cannot become a successful observation.
    let after = select(program, cwd, path)?
        .canonicalize()
        .map_err(|_| "cannot resolve backend after version probe")?;
    if after != resolved {
        return Err("backend path changed during version probe".into());
    }
    let digest = binding.digest_if_unchanged(&resolved)?;
    Ok(Some(SystemBackendVersion {
        backend: backend.as_str().into(),
        configured_program: program.into(),
        resolved_program: resolved.to_str().ok_or("non-UTF8 backend path")?.into(),
        version_output: output,
        sha256: digest.into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    fn script(path: &Path, body: &str) {
        fs::write(
            path,
            format!("#!/bin/sh\n[ \"$1\" = --version ] || exit 2\n{body}\n"),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[test]
    fn native_daemon_resolution_observes_path_cwd_and_replacement_versions() {
        let dir = TempDir::new("system-version").unwrap();
        let home = dir.path().canonicalize().unwrap();
        let bin = home.join("custom");
        fs::create_dir(&bin).unwrap();
        let exe = bin.join("backend");
        script(
            &exe,
            "[ \"$PWD\" = \"$EXPECTED_CWD\" ] || exit 3\n[ -z \"${TELEGRAM_BOT_TOKEN+x}\" ] || exit 4\nprintf 'backend 1.0\\n'",
        );
        let env = BTreeMap::from([
            ("PATH".into(), "custom:/usr/bin:/bin".into()),
            ("EXPECTED_CWD".into(), home.to_string_lossy().into_owned()),
        ]);
        let first = observe(&home, Backend::Codex, "backend", &home, &env)
            .unwrap()
            .unwrap();
        assert_eq!(first.version_output, "backend 1.0");
        assert_eq!(first.resolved_program, exe.to_str().unwrap());
        script(&exe, "printf 'backend 2.0\\n'");
        let second = observe(&home, Backend::Codex, "./custom/backend", &home, &env)
            .unwrap()
            .unwrap();
        assert_ne!(first.sha256, second.sha256);
        assert_eq!(second.version_output, "backend 2.0");
        assert!(observe(&home, Backend::Codex, "custom/backend", &home, &env).is_err());
        assert!(observe(&home, Backend::Codex, "absent", &home, &env).is_err());
    }
    #[test]
    fn native_probe_cannot_publish_a_symlink_retargeted_during_execution() {
        let dir = TempDir::new("system-retarget").unwrap();
        let home = dir.path().canonicalize().unwrap();
        let first = home.join("first");
        let second = home.join("second");
        let link = home.join("selected");
        script(&second, "printf 'backend 2.0\\n'");
        script(
            &first,
            "rm selected\nln -s second selected\nprintf 'backend 1.0\\n'",
        );
        symlink(&first, &link).unwrap();
        let env = BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]);
        let error =
            observe(&home, Backend::Codex, link.to_str().unwrap(), &home, &env).unwrap_err();
        assert!(error.contains("path changed"), "{error}");
    }
    #[test]
    fn invalid_managed_artifact_is_never_executed_as_an_external_probe() {
        let dir = TempDir::new("system-managed-refusal").unwrap();
        let home = dir.path().canonicalize().unwrap();
        let managed = home.join("backends/codex/1.0");
        fs::create_dir_all(&managed).unwrap();
        let program = managed.join("program");
        script(&program, "touch executed-marker\nprintf 'unsafe 1.0\n'");
        let env = BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]);
        assert!(
            observe(
                &home,
                Backend::Codex,
                program.to_str().unwrap(),
                &home,
                &env
            )
            .is_err()
        );
        assert!(!home.join("executed-marker").exists());
    }
    #[tokio::test]
    async fn runtime_probe_uses_captured_daemon_path_and_never_passes_unlisted_secrets() {
        let dir = TempDir::new("system-runtime-probe").unwrap();
        let home = dir.path().canonicalize().unwrap();
        let bin = home.join("selected-bin");
        fs::create_dir(&bin).unwrap();
        script(
            &bin.join("custom-cli"),
            "[ -z \"${TELEGRAM_BOT_TOKEN+x}\" ] || exit 7\n[ \"$AGEND_INSTANCE\" = version-test ] || exit 8\n[ \"$OPENCODE_DISABLE_AUTOUPDATE\" = 1 ] || exit 9\nprintf 'custom 1.2\n'",
        );
        let runtime = crate::runtime::HolderRuntime::new(
            &home,
            Path::new("/nonexistent/agend"),
            vec![
                ("PATH".into(), bin.to_string_lossy().into_owned()),
                ("TELEGRAM_BOT_TOKEN".into(), "literal-test-secret".into()),
            ],
            std::sync::Arc::new(|_| {}),
        );
        let result = runtime
            .observe_backend_version(
                "version-test",
                Backend::Opencode,
                "custom-cli",
                home.to_str().unwrap(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.version_output, "custom 1.2");
        assert_eq!(
            result.resolved_program,
            bin.join("custom-cli").to_str().unwrap()
        );
    }
}
