//! Unit tests that need the store's private parts: the DB thread's panic
//! path (P1) and creating a database without hard links. Everything else is
//! tested through the public API in `tests/`.

use std::cell::Cell;
use std::os::unix::fs::PermissionsExt;

use super::*;
use agend_testkit::block_on;
use agend_testkit::tempdir::TempDir;

thread_local! {
    /// Makes [`super::hard_link`] fail on this thread (the store is opened
    /// on the caller's thread).
    pub(super) static NO_HARD_LINKS: Cell<bool> = const { Cell::new(false) };
}

/// Verifier finding (gate 5 round 2): on a filesystem without hard links
/// (ExFAT) a new home could never be created and `.agend.db.new` was left
/// behind. The build file is renamed instead.
#[test]
fn without_hard_links_a_new_database_is_renamed_into_place() {
    let dir = TempDir::new("store-no-hard-links").unwrap();
    let home = dir.path().join("home");
    NO_HARD_LINKS.with(|f| f.set(true));
    let opened = SqliteStore::open(&home, 0);
    NO_HARD_LINKS.with(|f| f.set(false));
    let store = opened.unwrap();
    let task = Task::new("T-1", "no hard links", "general", "code", 1);
    block_on(store.create_task(&task)).unwrap();
    drop(store);

    let db = home.join(DB_FILE);
    let meta = fs::metadata(&db).unwrap();
    assert_eq!(
        (meta.permissions().mode() & 0o777, meta.nlink()),
        (0o600, 1)
    );
    let names: Vec<String> = fs::read_dir(&home)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![DB_FILE.to_owned()], "no build file left");
    let reopened = SqliteStore::open(&home, 0).unwrap();
    assert!(block_on(reopened.load_task("T-1")).unwrap().is_some());
}

/// The fallback never replaces an `agend.db` that appeared meanwhile, and a
/// failed rename says which step failed.
#[test]
fn the_rename_fallback_keeps_an_existing_database_and_names_the_failed_step() {
    let dir = TempDir::new("store-no-hard-links-steps").unwrap();
    let home = dir.path().join("home");
    create_private_dir(&home).unwrap();
    let new = home.join(NEW_DB_FILE);
    let db = home.join(DB_FILE);
    NO_HARD_LINKS.with(|f| f.set(true));

    fs::write(&new, b"built").unwrap();
    fs::write(&db, b"already there").unwrap();
    publish(&new, &db).unwrap();
    assert_eq!(fs::read(&db).unwrap(), b"already there");

    fs::remove_file(&db).unwrap();
    fs::create_dir(home.join("sub")).unwrap();
    let error = publish(&home.join("sub/missing"), &db).unwrap_err();
    NO_HARD_LINKS.with(|f| f.set(false));
    assert_eq!(
        error.to_string(),
        "creating agend.db failed: renaming .agend.db.new to agend.db (hard-linking it \
         failed first: Operation not supported (test: no hard links)): No such file or \
         directory (os error 2)"
    );
}

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
