//! Developer tasks for the AgEnD workspace. Run as `cargo xtask <command>`.
//!
//! Commands:
//! - `check-deps [--allow-skip]`: enforce the crate-boundary rules (see
//!   `check_deps` and `check_core`).
//! - `accept <gate>`: run the acceptance checks of one build gate
//!   (docs/ROADMAP.md). For now it runs fmt, clippy, tests and check-deps for
//!   the gate's crates; each gate adds its human-readable demo when it is built.
//!
//! - `record <backend> [scenario...] --sandbox <script>`: record the real
//!   backend CLI into `crates/agend-testkit/transcripts/` (see `record`).
//!
//! Planned, not implemented: protocol JSON schema generation, release
//! packaging.

mod accept;
mod check_core;
mod check_deps;
mod core_demo;
mod record;

use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <command>

Commands:
  check-deps [--allow-skip]
                   Check crate-boundary rules (--allow-skip: do not fail if the
                   no-std target is not installed; still prints SKIPPED)
  accept <gate>    Run the acceptance checks of a build gate (1-13 or its name);
                   each gate is described in docs/gates/gate-NN-<name>.md
  record <backend> [scenario...] --sandbox <script>
                   Record the REAL backend CLI (codex, opencode, claude) under a
                   write sandbox into crates/agend-testkit/transcripts/
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check-deps") => check_deps::run(args.iter().any(|a| a == "--allow-skip")),
        Some("accept") => accept::run(args.get(1).map(String::as_str)),
        Some("record") => record::run(&args[1..]),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("xtask: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// Path to the cargo that invoked us (falls back to `cargo` on PATH).
fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

/// Workspace root of the checkout we are run from, resolved at runtime with
/// `cargo locate-project --workspace` from the current directory. Not taken
/// from `CARGO_MANIFEST_DIR`: a stale xtask binary built in another checkout
/// would otherwise inspect that checkout instead of this one.
fn workspace_root() -> std::path::PathBuf {
    static ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| match locate_workspace() {
        Ok(root) => root,
        Err(msg) => {
            eprintln!("xtask: cannot find the workspace root: {msg}");
            std::process::exit(2);
        }
    })
    .clone()
}

fn locate_workspace() -> Result<std::path::PathBuf, String> {
    let out = std::process::Command::new(cargo())
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let manifest = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    manifest
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| format!("unexpected manifest path {}", manifest.display()))
}
