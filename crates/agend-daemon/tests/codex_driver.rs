//! The codex driver (gate 7) against the fake app-server, with the real
//! store: DRV-1..9, the busy levels, idempotency, crash reconciliation,
//! approvals, and DRV-6 / DRV-9 as four daemon boots in four processes (this
//! binary re-executed, `boot_child`). The sections live in
//! `tests/common/codex_driver.rs`, shared with `examples/codex_demo.rs`.
//!
//! Safety: homes under `/tmp/g7-<pid>-<n>`; the fake app-server runs in this
//! process; boot children are this binary, waited with a deadline.
#![cfg(unix)]

#[path = "common/daemon_process.rs"]
mod lab;

#[path = "common/codex_driver.rs"]
mod codex;

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use agend_core::model::{Backend as Kind, DeliveryState};
use agend_core::policy::busy::BusyLevel;
use agend_core::traits::Driver;
use agend_daemon::driver::codex::CodexDriver;
use agend_daemon::store::{InstanceStatus, SqliteStore};
use agend_testkit::block_on;
use agend_testkit::contract::driver as contract;

fn lab() -> lab::Lab {
    lab::Lab::with_prefix(Path::new("/nonexistent/agend"), "g7")
}

fn tag() -> String {
    format!("{}", std::process::id() % 10_000)
}

fn show(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

#[test]
fn contract_drv_1_to_9_passes_against_the_codex_driver_and_the_fake_app_server() {
    let lab = lab();
    let mut n = 0;
    let report = contract::run("codex", || {
        n += 1;
        let backend = codex::Backend::new(
            &lab.home(n),
            &format!("g7-{}k{n}", tag()),
            Duration::from_millis(100),
        )
        .unwrap();
        codex::Fixture::boot(&backend).unwrap()
    });
    println!("{}", report.summary());
    report.assert_passed();
    assert_eq!(report.total(), contract::cases::<codex::Fixture>().len());
}

#[test]
fn the_three_busy_levels_map_to_their_codex_methods() {
    let lab = lab();
    show(&codex::busy(&lab, &tag()).unwrap());
}

#[test]
fn one_id_is_one_delivery_across_restarts_and_other_content_is_refused() {
    let lab = lab();
    show(&codex::idempotent(&lab, &tag()).unwrap());
}

#[test]
fn a_crash_between_send_and_sent_is_reconciled_from_history_and_queue() {
    let lab = lab();
    show(&codex::crash_window(&lab, &tag()).unwrap());
}

#[test]
fn a_lost_reply_is_not_sent_again() {
    let lab = lab();
    show(&codex::reply_lost(&lab, &tag()).unwrap());
}

#[test]
fn approvals_are_declined() {
    let lab = lab();
    show(&codex::approval(&lab, &tag()).unwrap());
}

/// Not a real test: the body of a boot process.
#[test]
fn boot_child() {
    let Ok(n) = std::env::var(codex::BOOT_ENV) else {
        return;
    };
    match codex::boot_main(n.parse().unwrap()) {
        Ok(line) => println!("\n@{line}"),
        Err(e) => panic!("{e}"),
    }
}

fn boot_command() -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "boot_child", "--nocapture", "--test-threads=1"]);
    command
}

#[test]
fn four_boots_in_four_processes_backfill_and_never_send_twice() {
    let lab = lab();
    let lines = codex::restart(&lab, &tag(), false, &boot_command).unwrap();
    show(&lines);
    assert!(lines[2].contains("m-q confirmed"), "{lines:?}");
}

#[test]
fn four_boots_with_a_new_home_each_boot_fail() {
    let lab = lab();
    let error = codex::restart(&lab, &format!("{}n", tag()), true, &boot_command)
        .expect_err("a new home each boot must fail");
    println!("{}", error.lines().next().unwrap());
    assert!(error.starts_with("boot 2 failed"), "{error}");
}

/// P5: a message to an instance without a driver (claude until gate 12)
/// stays `queued`; one to a `failed` instance is `failed`; one to a codex
/// instance whose app-server is not connected stays `queued` and goes out
/// when the link connects.
#[test]
fn queued_without_a_driver_or_a_link_failed_for_a_failed_instance() {
    let lab = lab();
    let home = lab.home(1);
    let id = format!("g7-{}q", tag());
    let backend = codex::Backend::new(&home, &id, Duration::from_millis(100)).unwrap();
    {
        let store = SqliteStore::open(&home, 0).unwrap();
        let mut claude = block_on(store.instance(&id)).unwrap().unwrap();
        claude.id = format!("g7-{}c", tag());
        claude.backend = Kind::Claude;
        block_on(store.add_instance(&claude)).unwrap();
        let mut failed = claude.clone();
        failed.id = format!("g7-{}f", tag());
        failed.backend = Kind::Codex;
        failed.status = InstanceStatus::Failed;
        block_on(store.add_instance(&failed)).unwrap();
    }
    let store = Arc::new(SqliteStore::open(&home, 0).unwrap());
    let driver = CodexDriver::new(&home, Arc::clone(&store), Arc::new(|_| {}));
    let send = |to: &str, m: &str| {
        block_on(driver.deliver(to, &codex::message(m, "hi"), BusyLevel::Queue))
            .unwrap()
            .state
    };
    assert_eq!(
        send(&format!("g7-{}c", tag()), "m-c"),
        DeliveryState::Queued
    );
    assert_eq!(
        send(&format!("g7-{}f", tag()), "m-f"),
        DeliveryState::Failed
    );
    assert_eq!(send(&id, "m-early"), DeliveryState::Queued, "no link yet");
    let lines = block_on(driver.connect(&id, 1)).unwrap().unwrap();
    println!("{lines:?}");
    let state = block_on(store.message("m-early")).unwrap().unwrap().state;
    assert!(
        matches!(state, DeliveryState::Sent | DeliveryState::Confirmed),
        "{state:?}"
    );
    drop(driver);
    drop(store);
    drop(backend);
}

/// P3: `thread/resume` cannot find the thread: replaced only when nothing
/// was ever sent to it; otherwise the connect fails (`failed`, a human).
#[test]
fn a_lost_thread_is_replaced_only_when_it_was_never_used() {
    let lab = lab();
    let home = lab.home(2);
    let id = format!("g7-{}l", tag());
    let backend = codex::Backend::new(&home, &id, Duration::from_millis(100)).unwrap();
    let set_thread = |thread: &str| {
        let store = SqliteStore::open(&home, 0).unwrap();
        block_on(store.set_session_id(&id, thread)).unwrap();
    };
    set_thread("thread-lost-1");
    let store = Arc::new(SqliteStore::open(&home, 0).unwrap());
    let driver = CodexDriver::new(&home, Arc::clone(&store), Arc::new(|_| {}));
    let lines = block_on(driver.connect(&id, 1)).unwrap().unwrap();
    println!("{lines:?}");
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("thread thread-lost-1 not found and never used; new thread")),
        "{lines:?}"
    );
    let new = block_on(store.instance(&id))
        .unwrap()
        .unwrap()
        .session_id
        .unwrap();
    assert_ne!(new, "thread-lost-1");
    block_on(driver.deliver(&id, &codex::message("m-1", "hi"), BusyLevel::Queue)).unwrap();
    drop(driver);
    drop(store);
    set_thread("thread-lost-2");
    let store = Arc::new(SqliteStore::open(&home, 0).unwrap());
    let driver = CodexDriver::new(&home, Arc::clone(&store), Arc::new(|_| {}));
    let error = block_on(driver.connect(&id, 1)).unwrap_err();
    println!("{error}");
    assert!(
        error.starts_with("thread thread-lost-2 not found and messages were sent to it"),
        "{error}"
    );
    drop(driver);
    drop(store);
    drop(backend);
}
