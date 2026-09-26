//! Restarts across real processes (gate 5 P6, P7; A25): this test binary
//! re-executes itself (`child_process_entry`) once per daemon boot, so data
//! that only lives in a process cannot pass. The negative check runs the
//! same four boots with a new database path per boot and must fail.
//!
//! Every child gets a fresh temporary home, an absolute cwd and no `AGEND_*`
//! variables; the crash test kills only its own `Child`; every wait has a
//! deadline (`common::store_process::CHILD_DEADLINE`).
#![cfg(unix)]

#[path = "common/store_process.rs"]
mod common;

use std::process::Command;

use agend_core::pipeline::task::Task;
use agend_core::traits::{CasResult, Store};
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;
use agend_testkit::tempdir::TempDir;

use common::{ACKS_BEFORE_KILL, NOW};

/// Re-executes this test binary running only [`child_process_entry`].
fn base() -> Command {
    let mut cmd = Command::new(std::env::current_exe().expect("current_exe"));
    cmd.args([
        "--exact",
        "child_process_entry",
        "--nocapture",
        "--test-threads=1",
    ]);
    cmd
}

/// The child side. In a normal test run no role is set and this does
/// nothing.
#[test]
fn child_process_entry() {
    common::run_child_role();
}

#[test]
fn four_boots_in_four_processes_keep_everything_and_block_the_stale_writer() {
    let dir = TempDir::new("store-boots").unwrap();
    let home = dir.path().join("home");
    let lines = common::four_boots(&base, &|_| home.clone()).unwrap();
    for line in &lines {
        println!("{line}");
    }
    assert!(lines[1].ends_with("(open only)"), "{lines:?}");
    assert!(
        lines[2].contains("stale v=3 -> conflict current=4, task unchanged; v=5"),
        "{lines:?}"
    );
    assert!(lines[3].contains("v=5 ok"), "{lines:?}");
    assert!(lines[3].contains("next cas v=6"), "{lines:?}");
}

#[test]
fn four_boots_with_a_new_database_path_each_boot_fail() {
    let dir = TempDir::new("store-boots-neg").unwrap();
    let root = dir.path().to_path_buf();
    let error = common::four_boots(&base, &|boot| root.join(format!("home-{boot}")))
        .expect_err("a new database each boot must lose the data");
    println!("{error}");
    assert!(error.starts_with("boot 3 failed"), "{error}");
    assert!(error.contains("task T-1 is missing"), "{error}");
}

#[test]
fn a_hard_killed_writer_loses_no_acknowledged_write() {
    let dir = TempDir::new("store-crash").unwrap();
    let home = dir.path().join("home");
    let crash = common::crash(&base, &home).unwrap();
    println!(
        "killed own child pid={}: acked={} found={} integrity={}",
        crash.pid, crash.last_acked, crash.found, crash.integrity
    );
    assert!(!crash.status.success(), "the child was killed");
    assert!(crash.acked_before_kill >= ACKS_BEFORE_KILL);
    assert!(
        crash.found >= crash.last_acked,
        "acked {} but found {}",
        crash.last_acked,
        crash.found
    );
    // At most the one write in flight when the kill landed is extra.
    assert!(crash.found <= crash.last_acked + 1, "found {}", crash.found);
    assert_eq!(crash.integrity, "ok");
}

#[test]
fn a_second_process_cannot_open_the_database_and_the_first_keeps_writing() {
    let dir = TempDir::new("store-second-process").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    let task = Task::new("T-1", "first", "general", "code", 1);
    block_on(store.create_task(&task)).unwrap();

    let line = common::second_open(&base, &home).unwrap();
    assert!(
        line.ends_with(
            "error=agend.db is in use by another process (is another agend daemon running?)"
        ),
        "{line}"
    );
    assert_eq!(
        block_on(store.compare_and_swap_task(&task, 1)).unwrap(),
        CasResult::Written { new_version: 2 }
    );
}
