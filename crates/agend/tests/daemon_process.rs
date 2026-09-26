//! The real `agend daemon` with real holders (gate 6, contract layer 3 and
//! P1, P3, P5, P6). The sections live in
//! `agend-daemon/tests/common/daemon_process.rs`, shared with the
//! `daemon_probe demo`, so the demo prints exactly what is checked here.
//!
//! Safety: see that module. Each test has its own `Lab` (homes under `/tmp`,
//! instance ids starting `g6-` and unique to this test process); dropping it
//! stops every holder it started.
#![cfg(unix)]

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_agend");

/// Instance id tag unique to this test process (ids stay within 24 bytes).
fn tag(t: &str) -> String {
    format!("{t}{}", std::process::id() % 100_000)
}

fn show(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

/// Fails if a holder of `lab` is still running, or any `agend holder` of
/// this test's ids shows in `ps`.
fn assert_no_leftovers(lab: &lab::Lab, tag: &str) {
    assert_eq!(lab.running_holders(), vec![], "holders left running");
    let ps = Command::new("/bin/ps")
        .args(["-A", "-o", "command="])
        .output()
        .unwrap();
    let needle = format!("agend holder g6-{tag}");
    let ps = String::from_utf8_lossy(&ps.stdout);
    assert!(!ps.contains(&needle), "{needle} still in ps");
}

#[test]
fn four_daemon_boots_one_killed_keep_the_holder_and_its_counter() {
    let lab = lab::Lab::new(Path::new(BIN));
    let tag = tag("r");
    let home = lab.home(1);
    let lines = lab::restart(&lab, &tag, &|_| home.clone()).unwrap();
    show(&lines);
    assert!(
        lines.iter().any(|l| l.contains("killed -9 by test")),
        "{lines:?}"
    );
    lab.stop_all_holders();
    assert_no_leftovers(&lab, &tag);
}

#[test]
fn four_daemon_boots_with_a_new_home_each_boot_fail() {
    let lab = lab::Lab::new(Path::new(BIN));
    let tag = tag("n");
    let error = lab::restart(&lab, &tag, &|boot| lab.home(boot)).expect_err("must fail");
    println!("{}", error.lines().next().unwrap());
    assert!(error.starts_with("boot 2 failed"), "{error}");
    lab.stop_all_holders();
    assert_no_leftovers(&lab, &tag);
}

#[test]
fn an_agent_that_keeps_dying_is_resumed_three_times_then_failed() {
    let lab = lab::Lab::new(Path::new(BIN));
    let tag = tag("g");
    show(&lab::give_up(&lab, &tag).unwrap());
    lab.stop_all_holders();
    assert_no_leftovers(&lab, &tag);
}

#[test]
fn the_agent_gets_the_whitelisted_environment_and_the_shims_first() {
    let lab = lab::Lab::new(Path::new(BIN));
    let tag = tag("e");
    show(&lab::env(&lab, &tag).unwrap());
    lab.stop_all_holders();
    assert_no_leftovers(&lab, &tag);
}

#[test]
fn a_second_daemon_is_refused_after_ten_seconds_and_the_first_is_untouched() {
    let lab = lab::Lab::new(Path::new(BIN));
    let tag = tag("s");
    show(&lab::second_daemon(&lab, &tag).unwrap());
    lab.stop_all_holders();
    assert_no_leftovers(&lab, &tag);
}

#[test]
fn an_orphan_holder_is_stopped_at_the_next_boot() {
    let lab = lab::Lab::new(Path::new(BIN));
    let tag = tag("o");
    show(&lab::orphan(&lab, &tag).unwrap());
    assert_no_leftovers(&lab, &tag);
}

#[test]
fn the_daemon_needs_agend_home_and_takes_no_arguments() {
    let out = Command::new(BIN)
        .arg("daemon")
        .env_remove("AGEND_HOME")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "AGEND_HOME is not set\n"
    );

    let out = Command::new(BIN)
        .args(["daemon", "--foreground"])
        .env("AGEND_HOME", "/nonexistent-g6")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!Path::new("/nonexistent-g6").exists());
}
