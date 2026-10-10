//! Real pipeline fixture, with fake inbox workers. No real LLM is started.
#[path = "../tests/common/pipeline_process.rs"]
mod lab;
fn main() -> std::process::ExitCode {
    let result = match std::env::args().nth(1).as_deref() {
        Some("setup") => lab::home().and_then(|h| lab::setup(&h, &[])),
        Some("teardown") => lab::home().and_then(|h| lab::teardown(&h)),
        Some("demo") => lab::demo(),
        Some("install") => install(),
        _ => Err("usage: pipeline_probe setup|teardown|demo|install".into()),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pipeline_probe: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Exercise an explicitly supplied installed binary, never the developer default.
fn install() -> Result<(), String> {
    use std::{fs, os::unix::fs::PermissionsExt, process::Command, time::Instant};
    if std::env::var_os("AGEND_BIN").is_none() {
        return Err("install requires AGEND_BIN pointing to the extracted release".into());
    }
    let start = Instant::now();
    let root = agend_testkit::tempdir::TempDir::new("g13-install").map_err(|e| e.to_string())?;
    let (agend, _) = lab::binaries()?;
    let output = Command::new(&agend)
        .env_clear()
        .env("HOME", root.path())
        .env("PATH", "/usr/bin:/bin")
        .arg("init")
        .output()
        .map_err(|e| e.to_string())?;
    // Doctor reports missing model CLIs in this deliberately empty HOME.
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(format!("init failed: {output:?}"));
    }
    let home = root.path().join(".agend");
    let config = home.join("config.toml");
    for (path, expected) in [(&home, 0o700), (&config, 0o600)] {
        if fs::metadata(path)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o777
            != expected
        {
            return Err("init did not create private home/config".into());
        }
    }
    let original = fs::read(&config).map_err(|e| e.to_string())?;
    let mut runtime = lab::Lab {
        home,
        daemon: None,
        agend,
        environment: vec![("HOME".into(), root.path().display().to_string())],
    };
    lab::setup(&runtime.home, &[])?;
    let result = (|| {
        runtime.boot(None)?;
        let task = runtime.create("g10", "demo", "release install")?;
        runtime.approve(&task)?;
        runtime.wait_stage(&task, "done")?;
        let log = lab::git(&runtime.repo(), &["log", "--format=%B", "main"])?;
        if log.matches(&format!("Agend-Task: {task}")).count() != 1
            || runtime.home.join("worktrees").join(&task).exists()
            || fs::read(&config).map_err(|e| e.to_string())? != original
        {
            return Err("merge, task cleanup or configuration preservation failed".into());
        }
        if start.elapsed().as_secs() >= 300 {
            return Err("installed first task exceeded five minutes".into());
        }
        println!(
            "installed first task: {task} done; one merge; elapsed_ms={}; fake workers, no model calls",
            start.elapsed().as_millis()
        );
        Ok(())
    })();
    if result.is_err() {
        eprintln!("{}", runtime.logs());
    }
    runtime.stop(false);
    lab::teardown(&runtime.home)?;
    if runtime.home.exists() || runtime.repo().exists() {
        return Err("installation fixture cleanup failed".into());
    }
    result
}
