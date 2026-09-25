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

use agend_core::pipeline::task::{Task, TaskStatus};
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{CasResult, Clock, Store, StoredEvent};
use agend_daemon::store::retention::{DAY_MS, Keep, RETENTION, Target};
use agend_daemon::store::snapshot::{self, KEEP};
use agend_daemon::store::{
    BACKUPS_DIR, DB_FILE, LATEST_VERSION, MIGRATIONS, Migration, SqliteStore, StoreError,
};
use agend_testkit::block_on;
use agend_testkit::contract::store::{self as contract, StoreFixture};
use agend_testkit::fakes::FakeClock;
use agend_testkit::tempdir::TempDir;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

const NOW: u64 = FakeClock::DEFAULT_START_UNIX_MS;

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

    let two = [MIGRATIONS[0], ADD_TABLE];
    drop(SqliteStore::open_with(&home, NOW, &two).unwrap());
    assert_eq!(user_version(&home.join(DB_FILE)), 2);
    let pre = home
        .join(BACKUPS_DIR)
        .join(snapshot::pre_upgrade_name(NOW, 2));
    assert_eq!(user_version(&pre), 1, "the snapshot is the v1 database");
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
    let list = [MIGRATIONS[0], BROKEN];

    // From an empty database: 0001 commits, the broken one rolls back.
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
    assert_eq!(user_version(&db), 1);
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
