//! Run the gate 1 walkthrough in the core example target.

use crate::{cargo, workspace_root};
use std::process::Command;

pub fn run() -> Result<(), String> {
    let status = Command::new(cargo())
        .current_dir(workspace_root())
        .args([
            "run",
            "--quiet",
            "-p",
            "agend-core",
            "--example",
            "core_demo",
        ])
        .status()
        .map_err(|error| format!("cannot run core demo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("core demo failed".into())
    }
}
