//! `cargo xtask record <backend> [scenario...] --sandbox <script>`: records
//! the REAL backend CLI with `agend-record` (agend-testkit) and copies the
//! transcripts into `crates/agend-testkit/transcripts/<backend>/`.
//!
//! The recorder runs under `<script>` (a write sandbox such as
//! `record-sandbox.sh`: writes only to /private/tmp, TMPDIR and the CLI's own
//! state dirs), with its output in a fresh `mktemp -d /private/tmp/agend-rec-out-XXXX`;
//! only this step, outside the sandbox, writes into the repo. Transcripts of
//! failed scenarios (`*.failed.jsonl`) stay in the output dir.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{cargo, workspace_root};

pub fn run(args: &[String]) -> Result<(), String> {
    let mut sandbox = None;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--sandbox" {
            sandbox = it.next().cloned();
        } else {
            rest.push(arg.clone());
        }
    }
    let sandbox = sandbox.ok_or("--sandbox <script> is required (a write sandbox wrapper)")?;
    let backend = rest
        .first()
        .ok_or("usage: cargo xtask record <backend> [scenario...] --sandbox <script>")?;
    let root = workspace_root();
    let status = Command::new(cargo())
        .args(["build", "-p", "agend-testkit", "--bin", "agend-record"])
        .current_dir(&root)
        .status()
        .map_err(|e| format!("cargo build: {e}"))?;
    if !status.success() {
        return Err("cargo build -p agend-testkit --bin agend-record failed".into());
    }
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let recorder = target.join("debug").join("agend-record");
    let out = mktemp_out()?;
    println!("record: output in {}", out.display());
    let status = Command::new(&sandbox)
        .arg(&recorder)
        .args(&rest)
        .arg("--out")
        .arg(&out)
        .current_dir(&out)
        .status()
        .map_err(|e| format!("{sandbox}: {e}"))?;
    let copied = copy_transcripts(
        &out.join(backend),
        &root.join("crates/agend-testkit/transcripts").join(backend),
    )?;
    println!(
        "record: copied {copied} transcript(s) into crates/agend-testkit/transcripts/{backend}/"
    );
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "agend-record failed ({status}); see {}",
            out.display()
        ))
    }
}

fn mktemp_out() -> Result<PathBuf, String> {
    let out = Command::new("mktemp")
        .args(["-d", "/private/tmp/agend-rec-out-XXXX"])
        .output()
        .map_err(|e| format!("mktemp: {e}"))?;
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    if out.status.success() && path.starts_with("/private/tmp/") && path.is_dir() {
        Ok(path)
    } else {
        Err(format!("mktemp gave {}", path.display()))
    }
}

fn copy_transcripts(from: &Path, to: &Path) -> Result<usize, String> {
    let Ok(entries) = std::fs::read_dir(from) else {
        return Ok(0);
    };
    let mut copied = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".jsonl") || name.ends_with(".failed.jsonl") {
            continue;
        }
        std::fs::create_dir_all(to).map_err(|e| format!("mkdir {}: {e}", to.display()))?;
        std::fs::copy(entry.path(), to.join(&name)).map_err(|e| format!("copy {name}: {e}"))?;
        copied += 1;
    }
    Ok(copied)
}
