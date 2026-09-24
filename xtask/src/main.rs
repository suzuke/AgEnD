//! Developer tasks for the AgEnD workspace. Run as `cargo xtask <command>`.
//!
//! Commands:
//! - `check-deps`: enforce the crate-boundary rules (see `check_deps`).
//! - `accept <gate>`: run the acceptance checks of one build gate
//!   (docs/ROADMAP.md). For now it runs fmt, clippy, tests and check-deps for
//!   the gate's crates; each gate adds its human-readable demo when it is built.
//!
//! Planned, not implemented: protocol JSON schema generation, release
//! packaging, backend screen fixture recording.

mod accept;
mod check_deps;

use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <command>

Commands:
  check-deps       Check crate-boundary dependency rules
  accept <gate>    Run the acceptance checks of a build gate (1-12 or its name)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check-deps") => check_deps::run(),
        Some("accept") => accept::run(args.get(1).map(String::as_str)),
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

/// Workspace root (the parent of this crate's directory).
fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}
