//! The gate 9 CLI with the real `agend` binary: the CLI-n table against the
//! testkit fake daemon and a real `agend daemon` (P10), resends after a
//! restart (P5), `agend daemon restart` and its preflight (P7), `init` and
//! `doctor` (P3, P8, P9), start-up time, and the milestone: two fake codex
//! agents messaging each other across a daemon restart, every message once
//! and `confirmed`. The sections live in `tests/common/`, shared with the
//! `cli_demo` example.
//!
//! Safety: see `tests/common/cli_process.rs` (homes under
//! `/tmp/g9-<pid>-<n>`, only our own children are signalled, holders get
//! `Shutdown`).
#![cfg(unix)]

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

#[path = "../../agend-daemon/tests/common/codex_process.rs"]
mod codex;

#[path = "common/cli.rs"]
mod cli;

#[path = "common/cli_table.rs"]
mod table;

#[path = "common/cli_process.rs"]
mod sections;

use std::path::Path;
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_agend");

fn lab() -> lab::Lab {
    lab::Lab::with_prefix(Path::new(BIN), "g9")
}

fn show(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

/// Runs `section` in its own lab; no holder may be left running.
fn run(section: fn(&lab::Lab) -> Result<Vec<String>, String>) {
    let lab = lab();
    let result = section(&lab);
    lab.stop_all_holders();
    show(&result.unwrap());
    assert_eq!(lab.running_holders(), vec![], "holders left running");
}

#[test]
fn every_cli_row_holds_against_the_fake_and_the_real_daemon() {
    let lab = lab();
    let results = table::run_table(&lab, Path::new(BIN));
    lab.stop_all_holders();
    let (lines, failed) = table::lines(&results.unwrap());
    show(&lines);
    assert_eq!(failed, 0, "CLI rows failed");
    assert_eq!(lab.running_holders(), vec![], "holders left running");
}

#[test]
fn a_lost_send_is_resent_with_its_id_and_nothing_else_is() {
    run(sections::resend);
}

#[test]
fn restart_keeps_the_pid_and_the_holders() {
    run(sections::restart);
}

#[test]
fn a_failed_preflight_changes_nothing() {
    run(sections::preflight_failures);
}

#[test]
fn one_restart_at_a_time() {
    run(sections::one_restart_at_a_time);
}

#[test]
fn restart_waits_for_eof_and_a_new_boot_id() {
    run(sections::restart_waits);
}

#[test]
fn inherited_holders_are_reaped_and_nothing_else_is() {
    run(sections::after_exec);
}

#[test]
fn init_and_doctor() {
    run(sections::init_and_doctor);
}

#[test]
fn milestone_two_codex_agents_across_a_restart() {
    run(sections::milestone);
}

#[test]
fn ctrl_c_during_a_preflight_leaves_nothing() {
    run(sections::stop_during_preflight);
}

#[test]
fn the_preflight_deadline_holds() {
    run(sections::preflight_deadline);
}

#[test]
fn oversized_messages_and_lines_are_refused() {
    run(sections::limits);
}

#[test]
fn large_messages_to_codex_are_delivered() {
    run(sections::large_messages);
}

#[test]
fn a_stuck_codex_app_server_never_wedges_the_daemon() {
    run(sections::stuck_peer);
}

#[test]
fn a_binary_swapped_during_its_preflight_is_refused() {
    run(sections::swapped_binary);
}

#[test]
fn ctrl_c_during_a_restart_stops_the_daemon() {
    run(sections::ctrl_c_during_restart);
}

/// P3: the daemon has no default home either.
#[test]
fn unreachable_daemon_after_ten_seconds() {
    let lab = lab();
    let cli = cli::Cli::new(Path::new(BIN), &lab.home(1));
    let human = cli.run(None, &["status"]);
    let json = cli.run(None, &["status", "--json"]);
    show(&human.shown());
    show(&json.shown());
    assert_eq!(human.code, Some(1));
    assert!(
        human
            .stderr
            .starts_with("agend: cannot reach the AgEnD daemon at ")
    );
    assert!(
        human
            .stderr
            .ends_with("Is it running? Start it with: agend daemon\n")
    );
    // It waited the retry window (the message names it); no upper bound on
    // the wall clock: under load that measures the host, not agend (a hang
    // is still caught by the 90 s limit of `cli::finish`).
    assert!(human.took >= Duration::from_secs(10), "{:?}", human.took);
    assert!(human.stderr.contains(" after 10 s ("), "{}", human.stderr);
    assert!(
        json.stdout
            .starts_with(r#"{"error":{"code":"daemon_unreachable","message":"cannot reach"#)
    );
    assert!(json.stderr.is_empty());
}

/// An older daemon fails at once with what to do (the fake speaking 1.1).
#[test]
fn an_older_daemon_is_refused_at_once() {
    let lab = lab();
    let home = lab.home(1);
    std::fs::create_dir_all(home.join("run")).unwrap();
    let fake =
        agend_testkit::fake_daemon::FakeDaemon::start_at(&home.join("run/daemon.sock")).unwrap();
    fake.set_supported_versions(&[agend_core::protocol::client::V1_1]);
    let cli = cli::Cli::new(Path::new(BIN), &home);
    let hellos = || {
        fake.requests()
            .iter()
            .filter(|r| matches!(r, agend_core::protocol::client::ClientRequest::Hello { .. }))
            .count()
    };
    let human = cli.run(None, &["status"]);
    // Not retried: one connection, one hello (a retry says hello again).
    // Counted at the daemon, not timed: a wall-clock bound measures the
    // host under load.
    assert_eq!(hellos(), 1, "not retried");
    let json = cli.run(None, &["instance", "list", "--json"]);
    assert_eq!(hellos(), 2, "not retried");
    show(&human.shown());
    assert_eq!(human.code, Some(1));
    assert_eq!(
        human.stderr,
        "agend: the daemon speaks client protocol 1.1; this agend needs 1.2 — stop the daemon (Ctrl-C) and start this binary: agend daemon\n"
    );
    assert!(
        json.stdout
            .starts_with(r#"{"error":{"code":"version_mismatch""#)
    );
}

/// P10: `agend --version` still starts in under 10 ms (median of 50).
#[test]
fn version_starts_fast() {
    let p50 = sections::startup(Path::new(BIN), 50).unwrap();
    println!(
        "startup: agend --version p50 {:.1} ms",
        p50.as_secs_f64() * 1000.0
    );
    assert!(p50 < Duration::from_millis(10), "{p50:?}");
}
