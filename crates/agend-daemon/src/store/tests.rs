//! Unit tests that need the private job channel: the DB thread's panic path
//! (P1). Everything else is tested through the public API in `tests/`.

use super::*;
use agend_testkit::block_on;
use agend_testkit::tempdir::TempDir;

#[test]
fn after_the_db_thread_panics_every_call_reports_store_thread_stopped() {
    let dir = TempDir::new("store-panic").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, 0).unwrap();
    let task = Task::new("T-1", "survives", "general", "code", 1);
    block_on(store.create_task(&task)).unwrap();

    let panicked = block_on(
        store.call(|_| -> Result<(), StoreError> { panic!("injected panic on the DB thread") }),
    );
    assert!(matches!(panicked, Err(StoreError::Stopped)), "{panicked:?}");
    for _ in 0..2 {
        let error = block_on(store.load_task("T-1")).unwrap_err();
        assert_eq!(error.to_string(), "store thread stopped");
    }
    drop(store);

    // The panic closed the connection and released the lock; what was
    // committed before it is still there.
    let reopened = SqliteStore::open(&home, 0).unwrap();
    let loaded = block_on(reopened.load_task("T-1")).unwrap().unwrap();
    assert_eq!(loaded.task, task);
}
