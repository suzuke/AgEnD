//! `client_demo`: the gate 8 acceptance demo (`cargo xtask accept client`).
//! Runs the CLP contract against the testkit fake daemon and the real `agend
//! daemon`, then the version, slow-client, socket, retry, terminal and
//! restart sections. Finds `agend` through `AGEND_BIN` (default:
//! `target/<profile>/agend` next to this example).
//!
//! Safety: the sections only signal daemons they started (SIGINT or
//! `Child::kill`); holders get `Shutdown` (see
//! `tests/common/client_process.rs`); homes are under `/tmp/g8-<pid>-<n>`.

#[path = "../tests/common/daemon_process.rs"]
mod lab;

#[path = "../tests/common/client_process.rs"]
mod clp;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agend_testkit::contract::Report;
use agend_testkit::contract::client::{self, FakeDaemonFixture, fake_with_ids_from_one};

const RULES: usize = 12;

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
        // A stale export (e.g. from an earlier gate's steps) must not show up
        // later as a bare "No such file or directory".
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

/// `ok` or `FAIL: <reason>` for `rule` in `report`.
fn verdict(report: &Report, rule: &str) -> Option<String> {
    let outcome = report.results.iter().find(|o| o.rule == rule)?;
    Some(match &outcome.result {
        Ok(()) => "ok".into(),
        Err(reason) => format!("FAIL: {reason}"),
    })
}

fn contract(agend: &Path) -> Result<Vec<String>, String> {
    let real_rules: Vec<String> = (1..=RULES)
        .filter(|&n| n != 8)
        .map(|n| format!("CLP-{n}"))
        .collect();
    let real_rules: Vec<&str> = real_rules.iter().map(String::as_str).collect();
    let fake = client::run("fake", FakeDaemonFixture::new);
    let real = client::run_rules("real", &real_rules, || clp::RealDaemon::fixture(agend));
    let in_process = client::run_rules("real", &["CLP-8"], clp::InProcess::start);
    let mut out = Vec::new();
    for n in 1..=RULES {
        let rule = format!("CLP-{n}");
        let (report, note) = if n == 8 {
            (
                &in_process,
                " (the daemon's server code in this process: 2000 events)",
            )
        } else {
            (&real, "")
        };
        out.push(format!(
            "{rule} fake {}",
            verdict(&fake, &rule).ok_or("no fake result")?
        ));
        out.push(format!(
            "{rule} real {}{note}",
            verdict(report, &rule).ok_or("no real result")?
        ));
    }
    let negative = client::run_rules("ids-from-one", &["CLP-4"], fake_with_ids_from_one);
    let reason = negative
        .results
        .first()
        .and_then(|o| o.result.clone().err())
        .ok_or("negative check passed: the contract cannot tell event ids from 1")?;
    out.push(format!(
        "negative check (fake event ids from 1 again): CLP-4 FAIL: {reason}"
    ));
    for report in [&fake, &real, &in_process] {
        if !report.all_passed() {
            return Err(format!("{out:#?}\n{report}"));
        }
    }
    Ok(out)
}

fn demo() -> Result<(), String> {
    let agend = agend_bin()?;
    section("contract", contract(&agend))?;
    let lab = lab::Lab::with_prefix(&agend, "g8");
    println!("\ndemo directory {}", lab.root.display());
    section("version", clp::version(&lab))?;
    section(
        "slow-client",
        client::slow_clients(&mut clp::InProcess::start()).map(|lines| {
            std::iter::once("the daemon's server, three clients, 2000 events:".to_owned())
                .chain(lines.into_iter().map(|l| format!("  {l}")))
                .collect()
        }),
    )?;
    section("socket", clp::socket(&lab))?;
    section("retry", clp::retry(&lab))?;
    section("terminal", clp::terminal(&lab))?;
    section("restart", clp::restart(&lab))?;
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
    println!("client demo: all sections passed");
    Ok(())
}
