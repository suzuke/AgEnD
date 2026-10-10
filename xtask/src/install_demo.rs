//! Isolated installation scenarios; never register a host service or call a live backend.
use crate::{accept::step, cargo, workspace_root};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::{Command, Stdio},
};

pub fn run() -> Result<(), String> {
    println!("== Native installation demo ==");
    println!("Isolated homes, fake backends and local Telegram producer only.");
    // Resolve the executables from Cargo's own output, including configured targets.
    let output = Command::new(cargo())
        .current_dir(workspace_root())
        .args([
            "build",
            "--locked",
            "--message-format=json",
            "-p",
            "agend",
            "-p",
            "agend-testkit",
            "-p",
            "agend-daemon",
            "--bin",
            "agend",
            "--bin",
            "fake-worker",
            "--bin",
            "fake-claude-cli",
            "--bin",
            "fake-opencode-cli",
            "--example",
            "fake_codex",
            "--example",
            "fake_codex_next",
            "--example",
            "fake_claude_next",
            "--example",
            "fake_opencode_next",
            "--example",
            "pipeline_probe",
        ])
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot build installation producers: {e}"))?;
    if !output.status.success() {
        return Err("installation producer build failed".into());
    }
    let mut executables = BTreeMap::new();
    for line in output
        .stdout
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: serde_json::Value =
            serde_json::from_slice(line).map_err(|e| format!("invalid Cargo build output: {e}"))?;
        if value["reason"] == "compiler-artifact"
            && let (Some(name), Some(path)) = (
                value["target"]["name"].as_str(),
                value["executable"].as_str(),
            )
        {
            executables.insert(name.to_owned(), PathBuf::from(path));
        }
    }
    let executable = |name: &str| {
        executables
            .get(name)
            .ok_or_else(|| format!("Cargo did not produce {name}"))
    };
    // Check every required artifact before starting any scenario.
    let agend = executable("agend")?;
    let worker = executable("fake-worker")?;
    let probe = executable("pipeline_probe")?;
    for suite in [
        "install_home",
        "install_service",
        "backend_import",
        "backend_switch",
        "backend_version_monitor",
        "backend_canary",
        "telegram_pairing",
    ] {
        println!("== {suite} ==");
        step(&[
            "test",
            "-p",
            "agend",
            "--test",
            suite,
            "--",
            "--test-threads=1",
            "--nocapture",
        ])?;
    }
    step(&[
        "test",
        "-p",
        "agend",
        "--bin",
        "agend",
        "service::tests::",
        "--",
        "--nocapture",
    ])?;
    println!("== Fresh HOME: init, first task, one merge and cleanup ==");
    let status = Command::new(probe)
        .arg("install")
        .current_dir(workspace_root())
        .env("TMPDIR", "/tmp")
        .env("AGEND_BIN", agend)
        .env("AGEND_WORKER_BIN", worker)
        .status()
        .map_err(|e| format!("cannot run installation probe: {e}"))?;
    if !status.success() {
        return Err("fresh HOME installation probe failed".into());
    }
    println!(
        "Native installation demo passed. Archive/Brew installation, host service lifecycle and live backend/Telegram acceptance remain separate."
    );
    Ok(())
}
