//! The client protocol against the real `agend daemon` (gate 8): the CLP
//! contract (CLP-8 on the daemon's server in this process, the rest on the
//! real binary), the socket (P1), `retry` (P5), the terminal (P6), and
//! `agend debug ping|watch` across a restart (P4, P7). The sections live in
//! `agend-daemon/tests/common/client_process.rs`, shared with `client_demo`.
//!
//! Safety: see that module and `daemon_process.rs` (homes under
//! `/tmp/g8-<pid>-<n>`, only our own children are signalled, holders get
//! `Shutdown`).
#![cfg(unix)]

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

#[path = "../../agend-daemon/tests/common/client_process.rs"]
mod clp;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use agend_testkit::contract::client;

const BIN: &str = env!("CARGO_BIN_EXE_agend");

fn show(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

/// Stops the lab's holders with `Shutdown`; none may be left running.
fn assert_no_holders_left(lab: &lab::Lab) {
    lab.stop_all_holders();
    assert_eq!(lab.running_holders(), vec![], "holders left running");
}

const REAL_RULES: [&str; 11] = [
    "CLP-1", "CLP-2", "CLP-3", "CLP-4", "CLP-5", "CLP-6", "CLP-7", "CLP-9", "CLP-10", "CLP-11",
    "CLP-12",
];

#[test]
fn the_real_daemon_meets_the_client_protocol_contract() {
    let report = client::run_rules("real", &REAL_RULES, || {
        clp::RealDaemon::fixture(Path::new(BIN))
    });
    println!("{report}");
    report.assert_passed();
    assert_eq!(report.total(), REAL_RULES.len());
}

#[test]
fn the_daemon_server_closes_slow_clients_clp_8() {
    let report = client::run_rules(
        "real (in-process server)",
        &["CLP-8"],
        clp::InProcess::start,
    );
    println!("{report}");
    report.assert_passed();
}

#[test]
fn the_socket_is_private_replaced_after_a_crash_and_removed_on_stop() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g8");
    show(&clp::socket(&lab).unwrap());
    assert_no_holders_left(&lab);
}

#[test]
fn retry_resumes_or_starts_by_session_started() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g8");
    show(&clp::retry(&lab).unwrap());
    assert_no_holders_left(&lab);
}

#[test]
fn the_terminal_streams_after_the_screen() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g8");
    show(&clp::terminal(&lab).unwrap());
    assert_no_holders_left(&lab);
}

#[test]
fn ping_and_watch_survive_a_daemon_restart() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g8");
    show(&clp::restart(&lab).unwrap());
    assert_no_holders_left(&lab);
}

#[test]
fn a_1_0_daemon_fails_ping_at_once() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g8");
    show(&clp::version(&lab).unwrap());
}

/// P7: without a daemon, `agend debug ping` retries for 10 s, then says
/// what to do.
#[test]
fn ping_without_a_daemon_gives_up_after_ten_seconds() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g8");
    let home = lab.home(1);
    let (out, took) = clp::agend(
        &lab,
        &home,
        &["debug", "ping"],
        &[],
        Duration::from_secs(30),
    )
    .unwrap();
    let said = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{said}");
    assert!(
        said.starts_with(&format!(
            "cannot reach the AgEnD daemon at {}/run/daemon.sock after 10 s (",
            home.display()
        )) && said
            .trim_end()
            .ends_with("Is it running? Start it with: agend daemon"),
        "{said}"
    );
    assert!(
        took >= Duration::from_secs(10) && took < Duration::from_secs(13),
        "{took:?}"
    );
}

#[test]
fn debug_needs_agend_home_and_valid_arguments() {
    let out = Command::new(BIN)
        .args(["debug", "ping"])
        .env_remove("AGEND_HOME")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "agend: AGEND_HOME is not set\n"
    );
    for args in [
        &["debug"][..],
        &["debug", "ping", "--count"],
        &["debug", "ping", "--count", "0"],
        &["debug", "watch", "extra"],
    ] {
        let out = Command::new(BIN)
            .args(args)
            .env("AGEND_HOME", "/nonexistent-g8")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}
