//! The SQLite store against real database files in fresh temporary
//! directories (never an in-memory database): the STO-1..12 contract, schema
//! and migrations (P5), permissions (P2), retention (P8) and DB snapshots
//! (P9). Cross-process restarts and the crash test are in `store_process.rs`.
#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agend_core::model::Backend;
use agend_core::pipeline::task::{Task, TaskStatus};
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{CasResult, Clock, Store, StoredEvent};
use agend_daemon::store::retention::{DAY_MS, Keep, RETENTION, Target};
use agend_daemon::store::snapshot::{self, KEEP};
use agend_daemon::store::{
    BACKUPS_DIR, DB_FILE, Instance, InstanceStatus, LATEST_VERSION, MIGRATIONS, Migration,
    SqliteStore, StoreError,
};
use agend_testkit::block_on;
use agend_testkit::contract::store::{self as contract, StoreFixture};
use agend_testkit::fakes::FakeClock;
use agend_testkit::tempdir::TempDir;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

const NOW: u64 = FakeClock::DEFAULT_START_UNIX_MS;
/// The build file of a new database.
const NEW_DB: &str = ".agend.db.new";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn user_version(db: &Path) -> i64 {
    Connection::open(db)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn listing(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

/// The schema as `golden/schema.sql` records it: the version, then every
/// schema object's SQL, sorted.
fn schema_dump(db: &Path) -> String {
    let conn = Connection::open(db).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    let mut out = format!("-- user_version = {version}\n");
    let mut stmt = conn
        .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type, name")
        .unwrap();
    for sql in stmt.query_map([], |r| r.get::<_, String>(0)).unwrap() {
        out.push('\n');
        out.push_str(&sql.unwrap());
        out.push_str(";\n");
    }
    out
}

fn golden() -> (PathBuf, String) {
    let path = manifest_dir().join("src/store/golden/schema.sql");
    let text = fs::read_to_string(&path).unwrap_or_default();
    (path, text)
}

fn task(id: &str) -> Task {
    let mut task = Task::new(id, "store test", "team-a", "code", 2).set_requires_repo(true);
    task.depends_on = vec!["T-dep".into()];
    task.assignee = Some("dev-1".into());
    task.status = TaskStatus::Running;
    task
}

fn event(n: u64, at: u64) -> StoredEvent {
    StoredEvent {
        id: format!("e-{n}"),
        occurred_at_unix_ms: at,
        kind: "stage_completed".into(),
        detail: format!("step {n}"),
    }
}

// ---- contract (P7, in-process part) ----

/// The real store as a contract fixture: `Persisted` is the database file;
/// `boot` opens a new store (a new connection on a new DB thread) on it.
struct SqliteStoreFixture {
    store: SqliteStore,
}

impl StoreFixture for SqliteStoreFixture {
    type Store = SqliteStore;
    type Error = StoreError;
    type Persisted = PathBuf;

    fn store(&self) -> &SqliteStore {
        &self.store
    }

    fn insert_workflow(&self, workflow: &Workflow) {
        block_on(self.store.save_workflow(workflow)).unwrap();
    }

    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        block_on(self.store.load_events(task_id)).unwrap()
    }

    fn persisted(&self) -> PathBuf {
        self.store.db_path()
    }

    fn boot(db: &PathBuf) -> Self {
        let home = db.parent().unwrap();
        Self {
            store: SqliteStore::open(home, NOW).unwrap(),
        }
    }
}

#[test]
fn contract_sto_1_to_12_passes_against_the_real_store() {
    let root = TempDir::new("store-contract").unwrap();
    let mut n = 0;
    let report = contract::run("sqlite", || {
        n += 1;
        let home = root.path().join(format!("case-{n}"));
        SqliteStoreFixture {
            store: SqliteStore::open(&home, NOW).unwrap(),
        }
    });
    println!("{}", report.summary());
    report.assert_passed();
    assert_eq!(
        report.total(),
        contract::cases::<SqliteStoreFixture>().len()
    );
}

/// Negative control: a fixture whose every boot opens a new, empty
/// database must fail the restart rules. If it passed, the suite could not
/// tell persisted data from data that only lives in the process.
struct NewDatabaseEachBoot {
    store: SqliteStore,
    root: PathBuf,
}

impl StoreFixture for NewDatabaseEachBoot {
    type Store = SqliteStore;
    type Error = StoreError;
    type Persisted = PathBuf;

    fn store(&self) -> &SqliteStore {
        &self.store
    }

    fn insert_workflow(&self, workflow: &Workflow) {
        block_on(self.store.save_workflow(workflow)).unwrap();
    }

    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        block_on(self.store.load_events(task_id)).unwrap()
    }

    fn persisted(&self) -> PathBuf {
        self.root.clone()
    }

    fn boot(root: &PathBuf) -> Self {
        let home = root.join(format!("boot-{}", listing(root).len()));
        Self {
            store: SqliteStore::open(&home, NOW).unwrap(),
            root: root.clone(),
        }
    }
}

#[test]
fn contract_rejects_a_store_that_opens_a_new_database_each_boot() {
    let root = TempDir::new("store-contract-neg").unwrap();
    let mut n = 0;
    let report = contract::run("new-db-each-boot", || {
        n += 1;
        let case = root.path().join(format!("case-{n}"));
        fs::create_dir(&case).unwrap();
        NewDatabaseEachBoot {
            store: SqliteStore::open(&case.join("boot-0"), NOW).unwrap(),
            root: case,
        }
    });
    println!("{report}");
    let failing = report.failing_rules();
    assert!(
        report.results.iter().all(|o| o
            .result
            .as_ref()
            .err()
            .is_none_or(|e| !e.contains("panicked"))),
        "the fixture itself broke: {report}"
    );
    assert!(
        failing.contains(&"STO-4") && failing.contains(&"STO-12"),
        "{report}"
    );
}

// ---- P1 ----

#[test]
fn a_hundred_concurrent_callers_are_served_by_the_one_db_thread() {
    let dir = TempDir::new("store-concurrent").unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("home"), NOW).unwrap());
    block_on(store.create_task(&task("T-1"))).unwrap();
    let handles: Vec<_> = (0..100)
        .map(|_| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || block_on(store.load_task("T-1")))
        })
        .collect();
    for handle in handles {
        let loaded = handle.join().unwrap().unwrap().unwrap();
        assert_eq!(loaded.version, 1);
    }
}

// ---- P2, P3: files, modes, FakeStore behaviour ----

#[test]
fn a_new_home_is_private_and_the_database_is_created_0600() {
    let dir = TempDir::new("store-modes").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    block_on(store.snapshot(NOW)).unwrap();
    assert_eq!(mode(&home), 0o700, "home");
    assert_eq!(mode(&home.join(DB_FILE)), 0o600, "agend.db");
    assert_eq!(mode(&home.join(BACKUPS_DIR)), 0o700, "backups/");
    let snap = home.join(BACKUPS_DIR).join(snapshot::daily_name(NOW));
    assert_eq!(mode(&snap), 0o600, "snapshot");
    let wal = home.join(format!("{DB_FILE}-wal"));
    assert_eq!(mode(&wal), 0o600, "agend.db-wal");
    assert!(
        !home.join(format!("{DB_FILE}-shm")).exists(),
        "EXCLUSIVE locking keeps the WAL index in memory; no -shm file"
    );
}

#[test]
fn a_second_store_on_the_same_database_is_refused_with_the_in_use_message() {
    let dir = TempDir::new("store-second").unwrap();
    let home = dir.path().join("home");
    let first = SqliteStore::open(&home, NOW).unwrap();
    let error = SqliteStore::open(&home, NOW).err().unwrap();
    assert_eq!(
        error.to_string(),
        "agend.db is in use by another process (is another agend daemon running?)"
    );
    block_on(first.create_task(&task("T-1"))).unwrap();
}

#[test]
fn versions_start_at_one_and_events_need_a_stored_task_like_the_fake() {
    let dir = TempDir::new("store-fake-parity").unwrap();
    let store = SqliteStore::open(&dir.path().join("home"), NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    let loaded = block_on(store.load_task("T-1")).unwrap().unwrap();
    assert_eq!(
        loaded.version,
        agend_testkit::fakes::FakeStore::FIRST_VERSION
    );
    let error = block_on(store.append_event("T-missing", &event(1, NOW))).unwrap_err();
    assert_eq!(error.to_string(), "no task T-missing");
    let error = block_on(store.create_task(&task("T-1"))).unwrap_err();
    assert_eq!(error.to_string(), "task T-1 already exists");
}

#[test]
fn a_saved_workflow_version_never_changes() {
    let dir = TempDir::new("store-workflow").unwrap();
    let store = SqliteStore::open(&dir.path().join("home"), NOW).unwrap();
    let workflow = Workflow::builtin_code();
    block_on(store.save_workflow(&workflow)).unwrap();
    let mut changed = workflow.clone();
    changed.allow_unreviewed = !changed.allow_unreviewed;
    let error = block_on(store.save_workflow(&changed)).unwrap_err();
    assert!(matches!(error, StoreError::Exists(_)), "{error}");
    let loaded = block_on(store.load_workflow(&workflow.id, workflow.version)).unwrap();
    assert_eq!(loaded, Some(workflow));
}

#[test]
fn every_task_field_is_its_own_column_with_readable_values() {
    let dir = TempDir::new("store-columns").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    drop(store);
    let conn = Connection::open(home.join(DB_FILE)).unwrap();
    let (status, depends_on, requires_repo): (String, String, i64) = conn
        .query_row(
            "SELECT status, depends_on, requires_repo FROM tasks WHERE id = 'T-1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (status.as_str(), depends_on.as_str(), requires_repo),
        ("running", r#"["T-dep"]"#, 1)
    );
}

/// The home may be several directories deep in a directory that does not
/// exist yet: the missing ones are created 0700, the existing parent keeps
/// its mode.
#[test]
fn missing_parents_of_the_home_are_created_private() {
    let dir = TempDir::new("store-parents").unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let home = dir.path().join("a").join("b").join("home");
    drop(SqliteStore::open(&home, NOW).unwrap());
    assert_eq!(mode(dir.path()), 0o755, "existing parent unchanged");
    for created in [dir.path().join("a"), dir.path().join("a/b"), home.clone()] {
        assert_eq!(mode(&created), 0o700, "{}", created.display());
    }
    assert_eq!(mode(&home.join(DB_FILE)), 0o600);
}

// ---- P5: schema and migrations ----

#[test]
fn a_fresh_database_migrates_to_the_golden_schema() {
    let dir = TempDir::new("store-golden").unwrap();
    let home = dir.path().join("home");
    drop(SqliteStore::open(&home, NOW).unwrap());
    let dump = schema_dump(&home.join(DB_FILE));
    let (path, expected) = golden();
    if std::env::var_os("AGEND_BLESS_GOLDEN").is_some() {
        fs::write(&path, &dump).unwrap();
        return;
    }
    assert_eq!(
        dump,
        expected,
        "schema differs from {}; if intended, re-bless with \
         AGEND_BLESS_GOLDEN=1 cargo test -p agend-daemon --test store",
        path.display()
    );
    assert!(dump.starts_with(&format!("-- user_version = {LATEST_VERSION}\n")));
}

/// Every schema version this binary ever shipped has a fixture: its schema
/// as it was, plus sample rows. Opening it must upgrade to the golden
/// schema and read the samples back. This also catches an edited old
/// migration: the fixture keeps the schema as shipped.
#[test]
fn every_schema_version_fixture_upgrades_to_golden_and_keeps_its_samples() {
    let (_, expected) = golden();
    let dir = TempDir::new("store-fixtures").unwrap();
    for version in 1..=LATEST_VERSION {
        let fixture = manifest_dir().join(format!("src/store/fixtures/schema-v{version}.sql"));
        let sql = fs::read_to_string(&fixture)
            .unwrap_or_else(|e| panic!("missing fixture {}: {e}", fixture.display()));
        let home = dir.path().join(format!("v{version}"));
        fs::create_dir(&home).unwrap();
        let db = home.join(DB_FILE);
        Connection::open(&db).unwrap().execute_batch(&sql).unwrap();
        assert_eq!(user_version(&db), version, "fixture sets its version");

        let store = SqliteStore::open(&home, NOW).unwrap();
        let sample = block_on(store.load_task("T-fixture")).unwrap().unwrap();
        assert_eq!(sample.version, 3);
        assert_eq!(sample.task.title, "fixture task");
        assert_eq!(
            sample.task.depends_on,
            vec!["T-a".to_string(), "T-b".into()]
        );
        assert_eq!(sample.task.status, TaskStatus::Blocked);
        let events = block_on(store.load_events("T-fixture")).unwrap();
        assert_eq!(events.len(), 2);
        let workflow = block_on(store.load_workflow("code", 1)).unwrap();
        assert_eq!(workflow, Some(Workflow::builtin_code()));
        drop(store);
        assert_eq!(schema_dump(&db), expected, "fixture v{version} upgraded");
        let pre = home
            .join(BACKUPS_DIR)
            .join(snapshot::pre_upgrade_name(NOW, LATEST_VERSION));
        assert_eq!(pre.exists(), version < LATEST_VERSION);
    }
}

#[test]
fn an_upgrade_takes_a_pre_upgrade_snapshot_first() {
    const ADD_TABLE: Migration = Migration {
        name: "9002_test_add_table",
        sql: "CREATE TABLE later (id TEXT NOT NULL PRIMARY KEY) STRICT;",
    };
    let dir = TempDir::new("store-upgrade").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    drop(store);
    let first = SqliteStore::open_with(&home, NOW, &[]).err().unwrap();
    assert!(matches!(first, StoreError::TooNew { .. }), "{first}");

    let next = [MIGRATIONS, &[ADD_TABLE]].concat();
    drop(SqliteStore::open_with(&home, NOW, &next).unwrap());
    assert_eq!(user_version(&home.join(DB_FILE)), LATEST_VERSION + 1);
    let pre = home
        .join(BACKUPS_DIR)
        .join(snapshot::pre_upgrade_name(NOW, LATEST_VERSION + 1));
    assert_eq!(
        user_version(&pre),
        LATEST_VERSION,
        "the snapshot is the database before the upgrade"
    );
    let tasks: i64 = Connection::open(&pre)
        .unwrap()
        .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tasks, 1);
}

#[test]
fn a_failing_migration_rolls_back_and_leaves_user_version_unchanged() {
    const BROKEN: Migration = Migration {
        name: "9002_test_broken",
        sql: "CREATE TABLE half_done (id TEXT) STRICT; INSERT INTO no_such_table VALUES (1);",
    };
    let dir = TempDir::new("store-broken").unwrap();
    let home = dir.path().join("home");
    let list = [MIGRATIONS, &[BROKEN]].concat();
    drop(SqliteStore::open(&home, NOW).unwrap());

    // From a database at the latest version: the broken one rolls back.
    let error = SqliteStore::open_with(&home, NOW, &list).err().unwrap();
    assert!(
        matches!(
            error,
            StoreError::Migration {
                name: "9002_test_broken",
                ..
            }
        ),
        "{error}"
    );
    let db = home.join(DB_FILE);
    assert_eq!(user_version(&db), LATEST_VERSION);
    let half: i64 = Connection::open(&db)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'half_done'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(half, 0, "the failed migration's table was rolled back");
    // The database still opens with the real list.
    drop(SqliteStore::open(&home, NOW).unwrap());
}

/// A new database is built beside `agend.db` and only linked into place
/// once every migration has committed: a failed creation leaves no
/// `agend.db` (which the next boot would refuse), and the next open builds
/// the database again.
#[test]
fn a_failed_creation_leaves_no_database_and_the_next_open_builds_it_again() {
    const BROKEN: Migration = Migration {
        name: "9002_test_broken",
        sql: "INSERT INTO no_such_table VALUES (1);",
    };
    let dir = TempDir::new("store-create-fail").unwrap();
    let home = dir.path().join("home");
    let error = SqliteStore::open_with(&home, NOW, &[MIGRATIONS[0], BROKEN])
        .err()
        .unwrap();
    assert!(matches!(error, StoreError::Migration { .. }), "{error}");
    assert!(!home.join(DB_FILE).exists(), "no half-made agend.db");
    assert_eq!(user_version(&home.join(NEW_DB)), 1);

    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    drop(store);
    assert_eq!(user_version(&home.join(DB_FILE)), LATEST_VERSION);
    assert_eq!(mode(&home.join(DB_FILE)), 0o600);
    assert_eq!(
        listing(&home),
        BTreeSet::from([DB_FILE.to_owned()]),
        "the build file is gone; closing checkpointed the WAL"
    );
}

/// A kill between creating the build file and SQLite's first write leaves
/// a 0-byte `.agend.db.new`; unlike a 0-byte `agend.db` it is only a build
/// file, and the next open builds the database again.
#[test]
fn an_empty_build_file_from_a_killed_creation_is_rebuilt() {
    let dir = TempDir::new("store-create-killed").unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::write(home.join(NEW_DB), b"").unwrap();
    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    assert!(!home.join(NEW_DB).exists());
}

/// Verifier finding (gate 5 round 2): a leftover build file was linked in
/// as it was, keeping its mode and its version. It is removed and the
/// database built from scratch.
#[test]
fn a_leftover_build_file_is_replaced_by_a_fresh_0600_v1_database() {
    let dir = TempDir::new("store-leftover-build").unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    let new = home.join(NEW_DB);
    let leftover = Connection::open(&new).unwrap();
    leftover
        .execute_batch("CREATE TABLE leftover (a); PRAGMA user_version = 5;")
        .unwrap();
    drop(leftover);
    fs::set_permissions(&new, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(home.join(format!("{NEW_DB}-journal")), b"stale journal").unwrap();

    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    drop(store);
    let db = home.join(DB_FILE);
    assert_eq!(mode(&db), 0o600);
    assert_eq!(user_version(&db), LATEST_VERSION);
    assert_eq!(schema_dump(&db), golden().1, "no table of the leftover");
    assert_eq!(listing(&home), BTreeSet::from([DB_FILE.to_owned()]));
}

/// A build file that is a symlink (or not a regular file) is not the
/// store's; it is refused with the path named, and nothing is created.
#[test]
fn a_leftover_build_file_that_is_a_symlink_is_refused() {
    let dir = TempDir::new("store-leftover-symlink").unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    let outside = dir.path().join("outside.db");
    fs::write(&outside, b"").unwrap();
    let new = home.join(NEW_DB);
    std::os::unix::fs::symlink(&outside, &new).unwrap();

    let error = SqliteStore::open(&home, NOW).err().unwrap();
    assert_eq!(
        error.to_string(),
        format!(
            "refusing to use {}: a leftover build file that is not a regular file owned by \
             the home's owner; remove it by hand",
            new.display()
        )
    );
    assert_eq!(listing(&home), BTreeSet::from([NEW_DB.to_owned()]));
    assert_eq!(
        fs::metadata(&outside).unwrap().len(),
        0,
        "the target is untouched"
    );
}

/// Verifier finding (gate 5 round 1): an `agend.db` truncated to 0 bytes,
/// its WAL gone, used to open as a new, empty database, and every daily
/// snapshot of it then pushed a good snapshot out. It must be refused, and
/// the file left exactly as it is.
#[test]
fn a_zero_byte_database_is_refused_and_left_untouched() {
    let dir = TempDir::new("store-zero-byte").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    for i in 0..400 {
        block_on(store.create_task(&task(&format!("T-{i}")))).unwrap();
    }
    block_on(store.snapshot(NOW)).unwrap();
    drop(store);
    let db = home.join(DB_FILE);
    let _ = fs::remove_file(home.join(format!("{DB_FILE}-wal")));
    fs::OpenOptions::new()
        .write(true)
        .open(&db)
        .unwrap()
        .set_len(0)
        .unwrap();
    let before = (listing(&home), listing(&home.join(BACKUPS_DIR)));

    let error = SqliteStore::open(&home, NOW).err().unwrap();
    assert_eq!(
        error.to_string(),
        format!(
            "agend.db exists but is empty (0 bytes); refusing to start with an empty database \
             — restore a snapshot from {} (see README)",
            home.join("backups").display()
        )
    );
    assert_eq!(fs::metadata(&db).unwrap().len(), 0);
    assert_eq!((listing(&home), listing(&home.join(BACKUPS_DIR))), before);
}

/// Verifier finding (gate 5 round 2): a 1-byte `agend.db` was refused, but
/// SQLite had already written a 4096-byte header into it. A file shorter
/// than the header is refused before SQLite opens it.
#[test]
fn a_database_shorter_than_a_sqlite_header_is_refused_and_left_untouched() {
    let dir = TempDir::new("store-one-byte").unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    let db = home.join(DB_FILE);
    for content in [&b"x"[..], &[0u8; 99][..]] {
        fs::write(&db, content).unwrap();
        let before = (sha256(&db), listing(&home));
        let error = SqliteStore::open(&home, NOW).err().unwrap();
        assert_eq!(
            error.to_string(),
            format!(
                "agend.db exists but is only {} bytes, shorter than a SQLite header (100); \
                 refusing to start with it — restore a snapshot from {} (see README)",
                content.len(),
                home.join("backups").display()
            )
        );
        assert_eq!((sha256(&db), listing(&home)), before);
    }
}

/// Verifier finding (gate 5 round 2): a dangling-symlink `agend.db` failed
/// with `File exists (os error 17)` and left a built `.agend.db.new`.
#[test]
fn a_dangling_symlink_database_is_refused_with_a_clear_message() {
    let dir = TempDir::new("store-dangling").unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    let db = home.join(DB_FILE);
    std::os::unix::fs::symlink(dir.path().join("gone.db"), &db).unwrap();

    let error = SqliteStore::open(&home, NOW).err().unwrap();
    assert_eq!(
        error.to_string(),
        format!(
            "refusing to use {}: it is a symlink to a missing file; restore the link's \
             target or remove the link",
            db.display()
        )
    );
    assert_eq!(listing(&home), BTreeSet::from([DB_FILE.to_owned()]));
    assert!(!dir.path().join("gone.db").exists());
}

/// The other shape that looks new: a valid SQLite file with schema version
/// 0. This store never links such a file to `agend.db`, so it is refused
/// too, byte for byte unchanged.
#[test]
fn a_database_with_schema_version_zero_is_refused_and_left_untouched() {
    let dir = TempDir::new("store-version-zero").unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    let db = home.join(DB_FILE);
    Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TABLE scratch (a); DROP TABLE scratch;")
        .unwrap();
    assert_eq!(user_version(&db), 0);
    let before = (sha256(&db), listing(&home));

    let error = SqliteStore::open(&home, NOW).err().unwrap();
    assert_eq!(
        error.to_string(),
        format!(
            "agend.db exists but has no agend schema (schema version 0); refusing to start \
             with it — restore a snapshot from {} (see README)",
            home.join("backups").display()
        )
    );
    assert_eq!((sha256(&db), listing(&home)), before);
}

/// Verifier finding (gate 5 round 3): a negative schema version made `open`
/// panic. It is refused like version 0, byte for byte unchanged.
#[test]
fn a_database_with_a_negative_schema_version_is_refused_and_left_untouched() {
    let dir = TempDir::new("store-version-negative").unwrap();
    let home = dir.path().join("home");
    drop(SqliteStore::open(&home, NOW).unwrap());
    let db = home.join(DB_FILE);
    let conn = Connection::open(&db).unwrap();
    conn.pragma_update(None, "user_version", -1).unwrap();
    drop(conn);
    assert_eq!(user_version(&db), -1);
    let before = (sha256(&db), listing(&home));

    let error = SqliteStore::open(&home, NOW).err().unwrap();
    assert_eq!(
        error.to_string(),
        format!(
            "agend.db has an invalid schema version -1; refusing to start with it — \
             restore a snapshot from {} (see README)",
            home.join("backups").display()
        )
    );
    assert_eq!((sha256(&db), listing(&home)), before);
}

/// Verifier finding (gate 5 round 2): a valid header with schema version 1
/// and no tables opened, every task call failed, and its daily snapshots
/// pushed the good ones out. The tables of the recorded version must be
/// there.
#[test]
fn a_database_missing_a_table_of_its_version_is_refused_and_left_untouched() {
    let dir = TempDir::new("store-missing-tables").unwrap();
    let home = dir.path().join("home");
    drop(SqliteStore::open(&home, NOW).unwrap());
    let db = home.join(DB_FILE);
    Connection::open(&db)
        .unwrap()
        .execute_batch("DROP TABLE task_events; DROP TABLE workflows;")
        .unwrap();
    assert_eq!(user_version(&db), LATEST_VERSION);
    let before = (sha256(&db), listing(&home));

    let error = SqliteStore::open(&home, NOW).err().unwrap();
    assert_eq!(
        error.to_string(),
        format!(
            "agend.db has schema version {LATEST_VERSION} but lacks its table(s) task_events, \
             workflows; refusing to start with it — restore a snapshot from {} (see README)",
            home.join("backups").display()
        )
    );
    assert_eq!((sha256(&db), listing(&home)), before);

    // A version 1 header on a database with no tables at all.
    fs::remove_file(&db).unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    drop(conn);
    let error = SqliteStore::open(&home, NOW).err().unwrap();
    assert!(
        matches!(&error, StoreError::MissingTables { found: 1, missing, .. } if missing.len() == 3),
        "{error}"
    );
}

#[test]
fn a_too_new_database_is_refused_and_not_a_byte_changes() {
    let dir = TempDir::new("store-too-new").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    drop(store);
    let db = home.join(DB_FILE);
    Connection::open(&db)
        .unwrap()
        .pragma_update(None, "user_version", LATEST_VERSION + 1)
        .unwrap();
    let before = (sha256(&db), listing(&home));

    let error = SqliteStore::open(&home, NOW).err().unwrap();
    assert_eq!(
        error.to_string(),
        format!(
            "agend.db schema version {} is newer than this agend supports ({LATEST_VERSION}); \
             install a newer agend or restore a snapshot from {}",
            LATEST_VERSION + 1,
            home.join("backups").display()
        )
    );
    assert_eq!((sha256(&db), listing(&home)), before);
}

// ---- P8: retention ----

#[test]
fn every_table_in_the_database_has_a_retention_rule() {
    let dir = TempDir::new("store-retention-rules").unwrap();
    let home = dir.path().join("home");
    drop(SqliteStore::open(&home, NOW).unwrap());
    let conn = Connection::open(home.join(DB_FILE)).unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(!tables.is_empty());
    for table in &tables {
        let rule = RETENTION
            .iter()
            .find(|r| matches!(r.target, Target::Table { name, .. } if name == table))
            .unwrap_or_else(|| panic!("table {table} has no retention rule in RETENTION"));
        if let (Keep::Days(_), Target::Table { time_column, .. }) = (rule.keep, rule.target) {
            let column = time_column.unwrap_or_else(|| panic!("{table}: no time column"));
            let found: i64 = conn
                .query_row(
                    &format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name = ?1"),
                    [column],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "{table}.{column} does not exist");
        }
    }
    for rule in RETENTION {
        if let Target::Table { name, .. } = rule.target {
            assert!(
                tables.iter().any(|t| t == name),
                "rule for missing table {name}"
            );
        }
    }
    assert!(
        RETENTION.iter().any(|r| r.target
            == Target::DailyRotatedFile {
                path: "audit/shim.jsonl"
            }
            && r.keep == Keep::Days(14)),
        "the shim audit log rotation rule is listed"
    );
}

#[test]
fn prune_deletes_only_events_past_fourteen_days() {
    let dir = TempDir::new("store-prune").unwrap();
    let store = SqliteStore::open(&dir.path().join("home"), NOW).unwrap();
    let clock = FakeClock::new(NOW);
    for id in ["T-1", "T-2", "T-3"] {
        block_on(store.create_task(&task(id))).unwrap();
    }
    let mut workflow = Workflow::builtin_code();
    block_on(store.save_workflow(&workflow)).unwrap();
    workflow.version += 1;
    block_on(store.save_workflow(&workflow)).unwrap();
    for n in 0..120 {
        let id = ["T-1", "T-2", "T-3"][n as usize % 3];
        block_on(store.append_event(id, &event(n, clock.peek()))).unwrap();
    }
    let recent = NOW + 2 * DAY_MS;
    block_on(store.append_event("T-1", &event(999, recent))).unwrap();

    clock.advance(14 * DAY_MS);
    let report = block_on(store.prune(clock.now_unix_ms())).unwrap();
    assert_eq!(
        report.table("task_events").unwrap().after,
        121,
        "exactly 14 days is kept"
    );

    clock.advance(DAY_MS);
    let report = block_on(store.prune(clock.now_unix_ms())).unwrap();
    let rows = |t: &str| {
        let t = report.table(t).unwrap();
        (t.before, t.after)
    };
    assert_eq!(rows("task_events"), (121, 1));
    assert_eq!(rows("tasks"), (3, 3));
    assert_eq!(rows("workflows"), (2, 2));
    assert_eq!(
        block_on(store.load_events("T-1")).unwrap(),
        vec![event(999, recent)]
    );
}

// ---- P9: DB snapshots ----

#[test]
fn utc_dates_and_snapshot_names() {
    assert_eq!(snapshot::utc_date(0), "1970-01-01");
    assert_eq!(snapshot::utc_date(951_782_400_000), "2000-02-29");
    assert_eq!(snapshot::utc_date(1_790_812_799_999), "2026-09-30");
    assert_eq!(snapshot::utc_date(1_790_812_800_000), "2026-10-01");
    assert_eq!(snapshot::daily_name(NOW), "agend-2026-09-21.db");
    assert_eq!(
        snapshot::pre_upgrade_name(NOW, 2),
        "agend-2026-09-21-pre-v2.db"
    );
    for good in ["agend-2026-09-21.db", "agend-2026-09-21-pre-v12.db"] {
        assert!(snapshot::is_snapshot_name(good), "{good}");
    }
    for bad in [
        "notes.txt",
        "agend.db",
        "agend-notes.db",
        "agend-2026-9-21.db",
        "agend-2026-09-21.db.bak",
        "agend-2026-09-21-pre-v.db",
        "agend-2026-09-21-copy.db",
        ".agend-2026-09-21.db.tmp",
    ] {
        assert!(!snapshot::is_snapshot_name(bad), "{bad}");
    }
}

#[test]
fn daily_snapshots_keep_the_seven_newest_and_never_touch_other_files() {
    let dir = TempDir::new("store-snapshots").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    let backups = home.join(BACKUPS_DIR);
    fs::create_dir_all(&backups).unwrap();
    let foreign = ["notes.txt", "agend-notes.db", "agend-2026-09-21.db.bak"];
    for name in foreign {
        fs::write(backups.join(name), b"mine").unwrap();
    }
    let stale = format!(".{}.tmp", snapshot::daily_name(NOW));
    fs::write(backups.join(&stale), b"half a snapshot").unwrap();

    let clock = FakeClock::new(NOW);
    let first = block_on(store.snapshot(clock.now_unix_ms())).unwrap();
    assert!(first.taken);
    assert_eq!(first.stale_tmp_removed, vec![stale]);
    let again = block_on(store.snapshot(clock.now_unix_ms())).unwrap();
    assert!(!again.taken, "one snapshot per UTC day");
    for _ in 1..9 {
        clock.advance(DAY_MS);
        let report = block_on(store.snapshot(clock.now_unix_ms())).unwrap();
        assert!(report.taken);
    }
    let files = listing(&backups);
    let snapshots: Vec<&String> = files
        .iter()
        .filter(|n| snapshot::is_snapshot_name(n))
        .collect();
    assert_eq!(snapshots.len(), KEEP);
    assert_eq!(snapshots[0], &snapshot::daily_name(NOW + 2 * DAY_MS));
    assert_eq!(snapshots[KEEP - 1], &snapshot::daily_name(clock.peek()));
    for name in foreign {
        assert_eq!(fs::read(backups.join(name)).unwrap(), b"mine", "{name}");
    }
    assert_eq!(files.len(), KEEP + foreign.len());

    // A snapshot is an ordinary SQLite file: read-only open, quick_check,
    // the data is there.
    let newest = backups.join(snapshots[KEEP - 1]);
    let conn =
        Connection::open_with_flags(&newest, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .unwrap();
    let tasks: i64 = conn
        .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!((check.as_str(), tasks), ("ok", 1));
}

/// Verifier finding (gate 5 round 2): an empty database (a deleted
/// `agend.db` starts one) took a daily snapshot every day and pushed all
/// seven good ones out. No snapshot is taken while it is empty.
#[test]
fn an_empty_database_takes_no_daily_snapshot_and_evicts_none() {
    let dir = TempDir::new("store-snapshot-empty").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    for day in 0..KEEP as u64 {
        assert!(block_on(store.snapshot(NOW + day * DAY_MS)).unwrap().taken);
    }
    drop(store);
    let backups = home.join(BACKUPS_DIR);
    let good = listing(&backups);

    fs::remove_file(home.join(DB_FILE)).unwrap();
    let _ = fs::remove_file(home.join(format!("{DB_FILE}-wal")));
    let store = SqliteStore::open(&home, NOW).unwrap();
    for day in KEEP as u64..2 * KEEP as u64 {
        let report = block_on(store.snapshot(NOW + day * DAY_MS)).unwrap();
        assert!(report.empty && !report.taken, "{report:?}");
        assert!(report.rotated_out.is_empty());
    }
    assert_eq!(
        listing(&backups),
        good,
        "every good snapshot is still there"
    );

    block_on(store.create_task(&task("T-2"))).unwrap();
    let report = block_on(store.snapshot(NOW + 2 * KEEP as u64 * DAY_MS)).unwrap();
    assert!(report.taken && !report.empty);
    assert_eq!(report.rotated_out.len(), 1);
}

/// Verifier finding (gate 6 round 1, F2): instances are data too. A DB
/// whose only rows are instances (every home right after gate 6's first
/// `daemon_probe add`) gets its daily snapshot.
#[test]
fn a_database_with_only_instances_takes_its_daily_snapshot() {
    let dir = TempDir::new("store-snapshot-instances").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    assert!(block_on(store.snapshot(NOW)).unwrap().empty);
    let instance = Instance {
        id: "g6-1".into(),
        backend: Backend::Claude,
        program: "/bin/bash".into(),
        args: vec!["-c".into(), "exit 0".into()],
        working_directory: "/tmp".into(),
        session_id: Some("s-1".into()),
        status: InstanceStatus::New,
        session_started: false,
        agent_pid: None,
        legacy_no_thread: false,
    };
    block_on(store.add_instance(&instance)).unwrap();
    let report = block_on(store.snapshot(NOW)).unwrap();
    assert!(report.taken && !report.empty, "{report:?}");
    let copied: i64 = Connection::open(&report.path)
        .unwrap()
        .query_row("SELECT count(*) FROM instances", [], |r| r.get(0))
        .unwrap();
    assert_eq!(copied, 1);
}

/// Verifier finding (gate 5 round 1): a snapshot killed mid-write can
/// leave SQLite's `-journal` (or `-wal`, `-shm`) next to the temporary
/// file. The next snapshot removes them with it; files that only look
/// similar are kept.
#[test]
fn a_killed_snapshots_temporary_file_and_its_sidecars_are_removed() {
    let dir = TempDir::new("store-snapshot-sidecars").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    let backups = home.join(BACKUPS_DIR);
    fs::create_dir_all(&backups).unwrap();
    let tmp = format!(".{}.tmp", snapshot::daily_name(NOW - DAY_MS));
    let stale: Vec<String> = ["", "-journal", "-wal", "-shm"]
        .iter()
        .map(|s| format!("{tmp}{s}"))
        .collect();
    let foreign = [
        format!("{tmp}-journal.bak"),
        format!("{tmp}-other"),
        ".agend-notes.db.tmp-journal".to_owned(),
        format!("{}-journal", snapshot::daily_name(NOW - DAY_MS)),
        "notes.tmp-journal".to_owned(),
    ];
    for name in stale.iter().chain(&foreign) {
        fs::write(backups.join(name), b"x").unwrap();
    }

    let report = block_on(store.snapshot(NOW)).unwrap();
    let mut removed = report.stale_tmp_removed.clone();
    removed.sort();
    let mut expected = stale.clone();
    expected.sort();
    assert_eq!(removed, expected);
    let mut left: BTreeSet<String> = foreign.iter().cloned().collect();
    left.insert(snapshot::daily_name(NOW));
    assert_eq!(listing(&backups), left);
}

#[test]
fn a_restored_snapshot_opens_as_the_database() {
    let dir = TempDir::new("store-restore").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    block_on(store.create_task(&task("T-1"))).unwrap();
    let snap = block_on(store.snapshot(NOW)).unwrap().path;
    let loaded = block_on(store.load_task("T-1")).unwrap().unwrap();
    assert_eq!(
        block_on(store.compare_and_swap_task(&task("T-1"), loaded.version)).unwrap(),
        CasResult::Written { new_version: 2 }
    );
    drop(store);
    // README restore steps: daemon stopped; copy the snapshot over
    // agend.db; delete agend.db-wal.
    fs::copy(&snap, home.join(DB_FILE)).unwrap();
    let _ = fs::remove_file(home.join(format!("{DB_FILE}-wal")));
    let store = SqliteStore::open(&home, NOW).unwrap();
    let loaded = block_on(store.load_task("T-1")).unwrap().unwrap();
    assert_eq!(loaded.version, 1, "back to the snapshot's version");
}

// ---- gate 8 P5: migration 0003 `session_started` ----

/// Migration 0003's backfill: `running`, and `failed` codex/opencode, count
/// as started (they ran, so a retry must not start them fresh); `new` and
/// `failed` claude stay 0. Run on the shipped v2 schema (the fixture).
#[test]
fn migration_0003_marks_running_and_failed_codex_opencode_as_started() {
    let dir = TempDir::new("store-0003").unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    let db = home.join(DB_FILE);
    let v2 = fs::read_to_string(manifest_dir().join("src/store/fixtures/schema-v2.sql")).unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(&v2).unwrap();
    let rows = [
        ("claude-running", "claude", "running", true),
        ("claude-failed", "claude", "failed", false),
        ("claude-new", "claude", "new", false),
        ("codex-running", "codex", "running", true),
        ("codex-failed", "codex", "failed", true),
        ("codex-new", "codex", "new", false),
        ("opencode-failed", "opencode", "failed", true),
    ];
    for (id, backend, status, _) in rows {
        conn.execute(
            "INSERT INTO instances (id, backend, program, args, working_directory, session_id, status) \
             VALUES (?1, ?2, '/bin/bash', '[]', '/tmp', NULL, ?3)",
            [id, backend, status],
        )
        .unwrap();
    }
    drop(conn);
    let store = SqliteStore::open(&home, NOW).unwrap();
    let started: Vec<(String, bool)> = block_on(store.instances())
        .unwrap()
        .into_iter()
        .map(|i| (i.id, i.session_started))
        .collect();
    let mut expected: Vec<(String, bool)> = rows
        .iter()
        .map(|(id, _, _, s)| ((*id).to_owned(), *s))
        .chain([("fixture-1".to_owned(), true)])
        .collect();
    expected.sort();
    assert_eq!(started, expected);
}

/// `session_started` is only 0 or 1, and writing `running` sets it in the
/// same statement; it is never cleared afterwards.
#[test]
fn session_started_is_set_with_running_and_checked() {
    let dir = TempDir::new("store-session-started").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    let instance = Instance {
        id: "g8-1".into(),
        backend: Backend::Codex,
        program: "/bin/bash".into(),
        args: vec![],
        working_directory: "/tmp".into(),
        session_id: None,
        status: InstanceStatus::New,
        session_started: false,
        agent_pid: None,
        legacy_no_thread: false,
    };
    block_on(store.add_instance(&instance)).unwrap();
    let read = || block_on(store.instance("g8-1")).unwrap().unwrap();
    assert!(!read().session_started);
    block_on(store.set_instance_status("g8-1", InstanceStatus::Running)).unwrap();
    assert!(read().session_started);
    block_on(store.set_instance_status("g8-1", InstanceStatus::Failed)).unwrap();
    block_on(store.set_instance_status("g8-1", InstanceStatus::New)).unwrap();
    assert!(read().session_started, "never cleared");
    drop(store);
    let conn = Connection::open(home.join(DB_FILE)).unwrap();
    let error = conn
        .execute("UPDATE instances SET session_started = 2", [])
        .unwrap_err();
    assert!(
        error.to_string().contains("CHECK constraint failed"),
        "{error}"
    );
}

/// `tasks()` lists every stored task by id (the fleet view reads it).
#[test]
fn tasks_lists_every_task_by_id() {
    let dir = TempDir::new("store-tasks").unwrap();
    let store = SqliteStore::open(&dir.path().join("home"), NOW).unwrap();
    assert_eq!(block_on(store.tasks()).unwrap(), vec![]);
    for id in ["T-2", "T-1"] {
        block_on(store.create_task(&task(id))).unwrap();
    }
    let ids: Vec<String> = block_on(store.tasks())
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, ["T-1", "T-2"]);
}

// ---- gate 7: migration 0004, `messages` ----

/// Migration 0004 decides once which codex rows are from gate 6 with a
/// conversation (P3): `running`/`failed`, no thread, session started →
/// `failed` + `legacy_no_thread = 1`. `new`, a failure before the first
/// `Spawn` (`session_started = 0`), a codex row with a thread, and other
/// backends are left alone. Run on the shipped v3 schema (the fixture).
#[test]
fn migration_0004_marks_only_gate_6_codex_rows_with_a_conversation() {
    let dir = TempDir::new("store-0004").unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    let v3 = fs::read_to_string(manifest_dir().join("src/store/fixtures/schema-v3.sql")).unwrap();
    let conn = Connection::open(home.join(DB_FILE)).unwrap();
    conn.execute_batch(&v3).unwrap();
    // (id, backend, session, status, started) → (legacy, status after)
    let rows = [
        ("codex-running", "codex", None, "running", 1, true, "failed"),
        ("codex-failed", "codex", None, "failed", 1, true, "failed"),
        (
            "codex-failed-early",
            "codex",
            None,
            "failed",
            0,
            false,
            "failed",
        ),
        ("codex-new", "codex", None, "new", 0, false, "new"),
        (
            "codex-thread",
            "codex",
            Some("t-1"),
            "running",
            1,
            false,
            "running",
        ),
        (
            "opencode-running",
            "opencode",
            None,
            "running",
            1,
            false,
            "running",
        ),
    ];
    for (id, backend, session, status, started, _, _) in rows {
        conn.execute(
            "INSERT INTO instances (id, backend, program, args, working_directory, session_id, \
             status, session_started) VALUES (?1, ?2, 'codex', '[]', '/tmp', ?3, ?4, ?5)",
            rusqlite::params![id, backend, session, status, started],
        )
        .unwrap();
    }
    drop(conn);
    let store = SqliteStore::open(&home, NOW).unwrap();
    for (id, _, _, _, _, legacy, status) in rows {
        let i = block_on(store.instance(id)).unwrap().unwrap();
        assert_eq!(
            (i.legacy_no_thread, i.status.as_str(), i.agent_pid),
            (legacy, status, None),
            "{id}"
        );
    }
}

fn new_message(id: &str, body: &str) -> agend_daemon::store::NewMessage {
    agend_daemon::store::NewMessage {
        id: id.into(),
        from_instance: "operator".into(),
        to_instance: "g7-1".into(),
        task_id: None,
        body: body.into(),
        level: agend_core::policy::busy::BusyLevel::Queue,
    }
}

/// P5: one id is inserted once. Same content again → the stored row
/// unchanged; other content → `Different`, nothing changed; 50 claims of
/// one new id at once → one insert.
#[test]
fn a_message_id_is_claimed_once_and_other_content_is_refused() {
    use agend_daemon::store::Claim;
    let dir = TempDir::new("store-claim").unwrap();
    let store = Arc::new(SqliteStore::open(&dir.path().join("home"), NOW).unwrap());
    let Claim::Inserted(first) =
        block_on(store.claim_message(&new_message("m-1", "a"), NOW)).unwrap()
    else {
        panic!("not inserted")
    };
    assert_eq!(first.state, agend_core::model::DeliveryState::Queued);
    assert_eq!(
        block_on(store.claim_message(&new_message("m-1", "a"), NOW + 5)).unwrap(),
        Claim::Existing(first.clone())
    );
    assert_eq!(
        block_on(store.claim_message(&new_message("m-1", "b"), NOW + 5)).unwrap(),
        Claim::Different(first.clone())
    );
    let inserted = std::thread::scope(|s| {
        let runs: Vec<_> = (0..50)
            .map(|_| {
                let store = Arc::clone(&store);
                s.spawn(move || {
                    matches!(
                        block_on(store.claim_message(&new_message("m-2", "x"), NOW)).unwrap(),
                        Claim::Inserted(_)
                    )
                })
            })
            .collect();
        runs.into_iter()
            .map(|r| r.join().unwrap())
            .filter(|inserted| *inserted)
            .count()
    });
    assert_eq!(inserted, 1);
    let rows = block_on(store.messages_to("g7-1")).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["m-1", "m-2"]);
}

/// P5: `seq` is an explicit INTEGER PRIMARY KEY: after a 30-day prune
/// leaves a gap, a `VACUUM INTO` snapshot restored as `agend.db` keeps the
/// same order numbers. `seq` and `UNIQUE(id)` are in the golden schema.
#[test]
fn message_order_survives_prune_snapshot_and_restore() {
    let (_, golden) = golden();
    assert!(
        golden.contains("seq                INTEGER NOT NULL PRIMARY KEY")
            && golden.contains("id                 TEXT    NOT NULL UNIQUE"),
        "{golden}"
    );
    let dir = TempDir::new("store-seq").unwrap();
    let home = dir.path().join("home");
    let store = SqliteStore::open(&home, NOW).unwrap();
    // A DB without instances, tasks or events gets no daily snapshot.
    let instance = Instance {
        id: "g7-1".into(),
        backend: Backend::Codex,
        program: "codex".into(),
        args: vec![],
        working_directory: "/tmp".into(),
        session_id: None,
        status: InstanceStatus::New,
        session_started: false,
        agent_pid: None,
        legacy_no_thread: false,
    };
    block_on(store.add_instance(&instance)).unwrap();
    let old = NOW - 31 * DAY_MS;
    for (id, at) in [("m-old", old), ("m-a", NOW), ("m-b", NOW)] {
        block_on(store.claim_message(&new_message(id, id), at)).unwrap();
    }
    let report = block_on(store.prune(NOW)).unwrap();
    let pruned = report.table("messages").unwrap();
    assert_eq!((pruned.before, pruned.after), (3, 2));
    let seqs = |store: &SqliteStore| -> Vec<(String, i64)> {
        block_on(store.messages_to("g7-1"))
            .unwrap()
            .into_iter()
            .map(|m| (m.id, m.seq))
            .collect()
    };
    let before = seqs(&store);
    assert_eq!(before, [("m-a".to_owned(), 2), ("m-b".to_owned(), 3)]);
    block_on(store.snapshot(NOW)).unwrap();
    drop(store);
    let snap = home.join(BACKUPS_DIR).join(snapshot::daily_name(NOW));
    let restored = dir.path().join("restored");
    fs::create_dir(&restored).unwrap();
    fs::copy(&snap, restored.join(DB_FILE)).unwrap();
    let store = SqliteStore::open(&restored, NOW).unwrap();
    assert_eq!(seqs(&store), before);
    block_on(store.claim_message(&new_message("m-c", "c"), NOW)).unwrap();
    assert_eq!(seqs(&store).last().unwrap(), &("m-c".to_owned(), 4));
}
