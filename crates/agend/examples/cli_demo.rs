//! `cli_demo`: the gate 9 acceptance demo (`cargo xtask accept cli`). Runs
//! the new CLP rules (CLP-13..17) against the fake and the real daemon, the
//! CLI-n table (each command's input, output and exit code; `fake ok` /
//! `real ok`), the resend, restart, preflight, `init`/`doctor` sections, the
//! milestone (two fake codex agents across a restart) and the start-up time.
//! Finds `agend` through `AGEND_BIN` (default: `target/<profile>/agend` next
//! to this example) and `fake_codex` next to this example.
//!
//! Safety: the sections only signal daemons they started (SIGINT or
//! `Child::kill`); holders get `Shutdown`; homes are under
//! `/tmp/g9-<pid>-<n>`; the real codex, claude and opencode are never run.

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

#[path = "../../agend-daemon/tests/common/codex_process.rs"]
mod codex;

#[path = "../../agend-daemon/tests/common/client_process.rs"]
mod clp;

#[path = "../tests/common/cli.rs"]
mod cli;

#[path = "../tests/common/cli_table.rs"]
mod table;

#[path = "../tests/common/cli_process.rs"]
mod sections;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agend_testkit::contract::Report;
use agend_testkit::contract::client::{self, FakeDaemonFixture};

const GATE_9_RULES: [&str; 5] = ["CLP-13", "CLP-14", "CLP-15", "CLP-16", "CLP-17"];

fn main() -> ExitCode {
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

fn section(name: &str, lines: Result<Vec<String>, String>) -> Result<(), String> {
    println!("\n== {name}");
    for line in lines.map_err(|e| format!("{name} failed: {e}"))? {
        println!("{line}");
    }
    Ok(())
}

fn verdict(report: &Report, rule: &str) -> String {
    match report.results.iter().find(|o| o.rule == rule) {
        Some(outcome) => match &outcome.result {
            Ok(()) => "ok".into(),
            Err(reason) => format!("FAIL: {reason}"),
        },
        None => "not run".into(),
    }
}

fn contract(agend: &Path) -> Result<Vec<String>, String> {
    let fake = client::run_rules("fake", &GATE_9_RULES, FakeDaemonFixture::new);
    let real = client::run_rules("real", &GATE_9_RULES, || clp::RealDaemon::fixture(agend));
    let mut out = Vec::new();
    for rule in GATE_9_RULES {
        out.push(format!(
            "{rule} fake {} · real {}",
            verdict(&fake, rule),
            verdict(&real, rule)
        ));
    }
    if !fake.all_passed() || !real.all_passed() {
        return Err(format!("{out:#?}\n{fake}\n{real}"));
    }
    Ok(out)
}

fn cli_table(lab: &lab::Lab) -> Result<Vec<String>, String> {
    let results = table::run_table(lab, &lab.agend)?;
    let (lines, failed) = table::lines(&results);
    if failed > 0 {
        return Err(format!(
            "{}\n{failed} CLI row run(s) failed",
            lines.join("\n")
        ));
    }
    Ok(lines)
}

fn demo() -> Result<(), String> {
    let agend = agend_bin()?;
    section("contract (gate 9 rules)", contract(&agend))?;
    let lab = lab::Lab::with_prefix(&agend, "g9");
    println!("\ndemo directory {}", lab.root.display());
    section("CLI-n", cli_table(&lab))?;
    section("resend", sections::resend(&lab))?;
    section("restart", sections::restart(&lab))?;
    section("preflight", sections::preflight_failures(&lab))?;
    section(
        "one restart at a time",
        sections::one_restart_at_a_time(&lab),
    )?;
    section("restart waits", sections::restart_waits(&lab))?;
    section("after exec", sections::after_exec(&lab))?;
    section("init and doctor", sections::init_and_doctor(&lab))?;
    section("milestone", sections::milestone(&lab))?;
    let p50 = sections::startup(&agend, 50)?;
    println!(
        "\nstartup: agend --version p50 {:.1} ms (limit 10 ms)",
        p50.as_secs_f64() * 1000.0
    );
    if p50.as_millis() >= 10 {
        return Err("agend --version is too slow".into());
    }
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
    let pf = sections::preflight_homes();
    println!("/tmp/agend-pf-*: {}", pf.len());
    println!("cli demo: all sections passed");
    Ok(())
}
