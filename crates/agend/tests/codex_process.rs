//! The real `agend daemon` with real holders running the gate 7 wrapper and
//! `fake_codex` (this crate's example) as codex: resume after `kill -9` of
//! the holder, the sweep (daemon running and at boot), give-up, an
//! app-server that dies, an interrupted first start, gate 6 codex rows. The
//! sections live in `agend-daemon/tests/common/codex_process.rs`, shared
//! with the `codex_demo`.
//!
//! Safety: see that module. Each test has its own `Lab` (homes under
//! `/tmp/g7-<pid>-<n>`, instance ids unique to this test process); dropping
//! it stops every holder it started.
#![cfg(unix)]

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

#[path = "../../agend-daemon/tests/common/codex_process.rs"]
mod codex;

use std::path::Path;

const BIN: &str = env!("CARGO_BIN_EXE_agend");

fn lab() -> lab::Lab {
    lab::Lab::with_prefix(Path::new(BIN), "g7")
}

fn tag(t: &str) -> String {
    format!("{t}{}", std::process::id() % 10_000)
}

fn run(section: fn(&lab::Lab, &str) -> Result<Vec<String>, String>, t: &str) {
    let lab = lab();
    let lines = section(&lab, &tag(t)).unwrap();
    for line in &lines {
        println!("{line}");
    }
    lab.stop_all_holders();
    assert_eq!(lab.running_holders(), vec![], "holders left running");
}

#[test]
fn a_killed_holder_leaves_no_codex_and_the_restart_resumes_the_thread() {
    run(codex::resume, "r");
}

#[test]
fn the_sweep_kills_codex_that_ignores_sighup_while_running_and_at_boot() {
    run(codex::sweep_left_behind, "w");
}

#[test]
fn a_tui_that_always_dies_is_failed_after_three_resumes_with_no_codex_left() {
    run(codex::give_up, "x");
}

#[test]
fn an_app_server_gone_for_20_s_is_a_death_and_the_thread_resumes() {
    run(codex::app_server_dies, "g");
}

#[test]
fn a_first_start_interrupted_before_the_thread_creates_it_on_the_next_boot() {
    run(codex::first_start_interrupted, "s");
}

#[test]
fn a_gate_6_codex_row_is_failed_and_left_alone() {
    run(codex::legacy, "l");
}
