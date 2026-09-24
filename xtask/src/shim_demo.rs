//! Run the gate 3 walkthrough: build the `agend` binary, then run the
//! agend-shim example against it (it invokes the binary as `git`, `kill` and
//! `pkill` in a temporary repo).

use crate::{cargo, workspace_root};
use std::path::PathBuf;
use std::process::Command;

pub fn run() -> Result<(), String> {
    let status = Command::new(cargo())
        .current_dir(workspace_root())
        .args(["build", "--quiet", "-p", "agend"])
        .status()
        .map_err(|e| format!("cannot build agend: {e}"))?;
    if !status.success() {
        return Err("`cargo build -p agend` failed".into());
    }
    let agend = target_dir()?.join("debug").join("agend");
    let status = Command::new(cargo())
        .current_dir(workspace_root())
        .args([
            "run",
            "--quiet",
            "-p",
            "agend-shim",
            "--example",
            "shim_demo",
            "--",
        ])
        .arg(&agend)
        .status()
        .map_err(|e| format!("cannot run shim demo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("shim demo failed".into())
    }
}

fn target_dir() -> Result<PathBuf, String> {
    let out = Command::new(cargo())
        .current_dir(workspace_root())
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let meta: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    meta["target_directory"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| "cargo metadata has no target_directory".into())
}
