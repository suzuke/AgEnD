//! `codex_demo`: the gate 7 acceptance demo (`cargo xtask accept codex`).
//! Every section runs against fakes only, never the real `codex`:
//!
//! - the codex driver with the real store against the fake app-server
//!   (`tests/common/codex_driver.rs`): `busy`, `idempotent`,
//!   `crash-window`, `approval`, `restart` (four boots in four processes:
//!   this example re-executed);
//! - the real `agend daemon` with real holders running the `sh` wrapper
//!   and `fake_codex` (`tests/common/codex_process.rs`): `resume`, `sweep`,
//!   `give-up`, `app-server-dies`, `first-start-interrupted`, `legacy`.
//!
//! Needs `target/<profile>/agend` (or `AGEND_BIN`) and
//! `target/<profile>/examples/fake_codex`: `cargo build -p agend --bin agend
//! --example fake_codex` (xtask does it).
//!
//! Safety: see the two modules; homes under `/tmp/g7-<pid>-<n>`.

#[path = "../tests/common/daemon_process.rs"]
mod lab;

#[path = "../tests/common/codex_driver.rs"]
mod driver;

#[path = "../tests/common/codex_process.rs"]
mod process;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    if let Ok(n) = std::env::var(driver::BOOT_ENV) {
        return match driver::boot_main(n.parse().unwrap_or(0)) {
            Ok(line) => {
                println!("@{line}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                println!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    match demo() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            println!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn agend_bin() -> Result<PathBuf, String> {
    if let Some(bin) = std::env::var_os("AGEND_BIN") {
        let bin = PathBuf::from(bin);
        return if bin.is_file() {
            Ok(bin)
        } else {
            Err(format!(
                "AGEND_BIN={} does not exist; unset AGEND_BIN or build agend",
                bin.display()
            ))
        };
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bin = exe
        .parent()
        .and_then(Path::parent)
        .map(|dir| dir.join("agend"))
        .ok_or("cannot locate agend; set AGEND_BIN")?;
    if bin.is_file() {
        Ok(bin)
    } else {
        Err(format!(
            "{} not found; run `cargo build -p agend` or set AGEND_BIN",
            bin.display()
        ))
    }
}

fn section(name: &str, lines: Result<Vec<String>, String>) -> Result<(), String> {
    println!("\n== {name}");
    for line in lines.map_err(|e| format!("{name} failed: {e}"))? {
        println!("{line}");
    }
    Ok(())
}

fn boot_command() -> Command {
    Command::new(std::env::current_exe().expect("current_exe"))
}

fn demo() -> Result<(), String> {
    process::fake_codex()?;
    let lab = lab::Lab::with_prefix(&agend_bin()?, "g7");
    println!("demo directory {}", lab.root.display());
    let tag = "d";
    section("busy", driver::busy(&lab, tag))?;
    section("idempotent", driver::idempotent(&lab, tag))?;
    section("crash-window", driver::crash_window(&lab, tag))?;
    section("approval", driver::approval(&lab, tag))?;
    section("restart", driver::restart(&lab, tag, false, &boot_command))?;
    match driver::restart(&lab, "n", true, &boot_command) {
        Ok(_) => return Err("negative check passed: it cannot tell restarts apart".into()),
        Err(e) => println!(
            "negative check (new AGEND_HOME each boot): {}",
            e.lines().next().unwrap_or_default()
        ),
    }
    section("resume", process::resume(&lab, tag))?;
    section("sweep", process::sweep_left_behind(&lab, tag))?;
    section("give-up", process::give_up(&lab, tag))?;
    section("app-server-dies", process::app_server_dies(&lab, tag))?;
    section(
        "first-start-interrupted",
        process::first_start_interrupted(&lab, tag),
    )?;
    section("legacy", process::legacy(&lab, tag))?;
    println!("\n== cleanup");
    let stopped = lab.stop_all_holders();
    let left = lab.running_holders();
    println!(
        "stopped {stopped} holder(s) with Shutdown; running now: {}",
        left.len()
    );
    if !left.is_empty() {
        return Err(format!("holders left: {left:?}"));
    }
    println!("codex demo: all sections passed");
    Ok(())
}
