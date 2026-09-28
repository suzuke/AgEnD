//! `tui_real`: the gate 11 acceptance demo against the real daemon (`cargo
//! xtask accept tui` runs it after `agend-tui`'s `tui_accept`). `agend
//! daemon` in a temp home with a counting agent and one that dies at once;
//! the TUI (`App` through `agend-client`) sees the needs-you item appear,
//! retries it, watches the live terminal, types, is refused as an agent,
//! and reconnects across a daemon restart; then `agend app`'s exits. The
//! sections live in `tests/common/tui_process.rs`, shared with
//! `tests/tui_daemon.rs`. Finds `agend` through `AGEND_BIN` (default:
//! `target/<profile>/agend` next to this example).
//!
//! Safety: see that module (homes under `/tmp/g11.t-<pid>-<n>`, only our
//! own daemons are signalled, holders get `Shutdown`).

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

#[path = "../../agend-daemon/tests/common/client_process.rs"]
mod clp;

#[path = "../tests/common/tui_process.rs"]
mod tui;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn agend_bin() -> Result<PathBuf, String> {
    if let Some(bin) = std::env::var_os("AGEND_BIN") {
        let bin = PathBuf::from(bin);
        return if bin.is_file() {
            Ok(bin)
        } else {
            Err(format!(
                "AGEND_BIN={} does not exist; `unset AGEND_BIN` or point it at a built agend",
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

fn main() -> ExitCode {
    let result = agend_bin().and_then(|bin| {
        let lab = tui::lab(&bin);
        let lines = tui::scenario(&lab)?;
        lab.stop_all_holders();
        if !lab.running_holders().is_empty() {
            return Err("holders left running".into());
        }
        Ok(lines)
    });
    match result {
        Ok(lines) => {
            for line in lines {
                println!("{line}");
            }
            println!("tui real-daemon demo: all sections passed");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("tui real-daemon demo: FAILED: {message}");
            ExitCode::FAILURE
        }
    }
}
