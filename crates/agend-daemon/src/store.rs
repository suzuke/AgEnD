//! SQLite store, the only persistent state (D8). Gate 5 builds three tables:
//! `tasks`, `workflows`, `task_events`; every later gate that needs
//! persistence adds its own tables in its own migration.
//!
//! - One std thread, `agend-db`, owns the only [`rusqlite::Connection`]. The
//!   async methods send a closure over a bounded channel and await the
//!   answer on a oneshot, so there is exactly one writer and no pool.
//!   After that thread panics every call returns [`StoreError::Stopped`]
//!   (`store thread stopped`); nothing retries, the service manager restarts
//!   the daemon.
//! - The database is `<home>/agend.db` (created 0600; home, any missing
//!   parent of it, and `backups/` created 0700). The caller passes the home;
//!   the store reads no environment variable.
//! - A new database is built as `<home>/.agend.db.new` and hard-linked (or,
//!   without hard links, renamed) to `agend.db` only after every migration
//!   has committed, so an existing `agend.db` is always a database this
//!   store finished creating. A build file left by a failed or killed
//!   creation is removed and built again. An `agend.db` that is shorter
//!   than a SQLite header, has schema version 0 or below, or lacks a table of its
//!   version was damaged or replaced; the store refuses it
//!   ([`StoreError::Empty`], [`StoreError::NoSchema`], [`StoreError::BadVersion`],
//!   [`StoreError::MissingTables`]) without changing a byte, instead of
//!   starting over with an empty database whose daily snapshots would push
//!   the good ones out. A dangling symlink or a non-file is refused too
//!   ([`StoreError::Refused`]). `locking_mode=EXCLUSIVE` is taken when the
//!   store opens, so a second process fails with [`StoreError::InUse`].
//! - Forward-only migrations ([`migrate`]) tracked in `PRAGMA user_version`;
//!   a database newer than this binary is refused without a single byte
//!   changed, and an upgrade first writes a DB snapshot.
//! - `journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON`.
//! - Retention ([`retention::RETENTION`], [`SqliteStore::prune`]) and the
//!   daily DB snapshot ([`SqliteStore::snapshot`], 7 kept). The daemon runs
//!   both at boot and every hour (`crate::housekeeping`, gate 6 P5).
//! - `instances` (gate 6 P2): the agents the daemon keeps running;
//!   `session_started` (gate 8 P5, migration 0003) says whether the
//!   backend session was ever created.
//! - Pipeline progress (`PipelineState`) is not stored yet: gate 10 adds it as
//!   a column of `tasks`, written together with the task by the same
//!   compare-and-swap, and never rebuilt by replaying events (gate 5 P4).
//!
//! Must NOT: be called from async tasks directly (go through the DB thread);
//! open a second connection to `agend.db`; delete a file in `backups/` that
//! does not match the DB snapshot name pattern.

pub mod instances;
mod migrate;
pub mod retention;
pub mod snapshot;
pub mod task_row;

use std::fmt;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use agend_core::pipeline::task::Task;
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{CasResult, Store, StoredEvent, VersionedTask};
use rusqlite::{Connection, ErrorCode, OpenFlags};
use tokio::sync::{mpsc, oneshot};

pub use instances::{Instance, InstanceStatus};
pub use migrate::{LATEST_VERSION, MIGRATIONS, Migration};
pub use retention::PruneReport;
pub use snapshot::SnapshotReport;

/// The database file inside the AgEnD home.
pub const DB_FILE: &str = "agend.db";
/// Where a new database is built before it is linked to [`DB_FILE`].
const NEW_DB_FILE: &str = ".agend.db.new";
/// The DB snapshot directory inside the AgEnD home.
pub const BACKUPS_DIR: &str = "backups";
/// Name of the thread that owns the connection.
pub const THREAD_NAME: &str = "agend-db";
/// Size of the SQLite file header; a shorter file is not a database.
const SQLITE_HEADER_LEN: u64 = 100;
/// Requests that may wait for the DB thread before senders wait too.
const QUEUE_CAPACITY: usize = 256;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

#[derive(Debug)]
pub enum StoreError {
    /// Another process holds `agend.db` (P2).
    InUse,
    /// The database schema is newer than this binary (P5).
    TooNew {
        found: i64,
        supported: i64,
        home: PathBuf,
    },
    /// `agend.db` exists but is 0 bytes, or shorter than a SQLite header
    /// (truncated, or replaced).
    Empty {
        len: u64,
        home: PathBuf,
    },
    /// `agend.db` exists but has schema version 0: it is not a database
    /// this store created.
    NoSchema {
        home: PathBuf,
    },
    /// `agend.db` has a negative schema version, which no agend writes.
    BadVersion {
        found: i64,
        home: PathBuf,
    },
    /// `agend.db` has schema version `found` but lacks tables that version
    /// has.
    MissingTables {
        found: i64,
        missing: Vec<String>,
        home: PathBuf,
    },
    /// The DB thread is gone (it panicked); the daemon must restart.
    Stopped,
    /// A migration failed; it was rolled back and `user_version` is unchanged.
    Migration {
        name: &'static str,
        source: rusqlite::Error,
    },
    /// A task or workflow version with this key already exists.
    Exists(String),
    /// Events can only be appended to a stored task.
    UnknownTask(String),
    /// A value cannot be stored or read back (out of range, bad encoding).
    Invalid(String),
    /// A snapshot failed its `quick_check`; it was not kept.
    SnapshotCheck(String),
    /// A file in the home is not what the store expects; nothing was changed.
    Refused {
        path: PathBuf,
        reason: &'static str,
    },
    /// Creating `agend.db` failed at `step`.
    Create {
        step: String,
        source: io::Error,
    },
    Sqlite(rusqlite::Error),
    Io(io::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InUse => write!(
                f,
                "{DB_FILE} is in use by another process (is another agend daemon running?)"
            ),
            Self::TooNew {
                found,
                supported,
                home,
            } => write!(
                f,
                "{DB_FILE} schema version {found} is newer than this agend supports ({supported}); \
                 install a newer agend or restore a snapshot from {}",
                home.join(BACKUPS_DIR).display()
            ),
            Self::Empty { len: 0, home } => write!(
                f,
                "{DB_FILE} exists but is empty (0 bytes); refusing to start with an empty \
                 database — restore a snapshot from {} (see README)",
                home.join(BACKUPS_DIR).display()
            ),
            Self::Empty { len, home } => write!(
                f,
                "{DB_FILE} exists but is only {len} bytes, shorter than a SQLite header \
                 ({SQLITE_HEADER_LEN}); refusing to start with it — restore a snapshot from {} \
                 (see README)",
                home.join(BACKUPS_DIR).display()
            ),
            Self::NoSchema { home } => write!(
                f,
                "{DB_FILE} exists but has no agend schema (schema version 0); refusing to \
                 start with it — restore a snapshot from {} (see README)",
                home.join(BACKUPS_DIR).display()
            ),
            Self::BadVersion { found, home } => write!(
                f,
                "{DB_FILE} has an invalid schema version {found}; refusing to start with it — \
                 restore a snapshot from {} (see README)",
                home.join(BACKUPS_DIR).display()
            ),
            Self::MissingTables {
                found,
                missing,
                home,
            } => write!(
                f,
                "{DB_FILE} has schema version {found} but lacks its table(s) {}; refusing to \
                 start with it — restore a snapshot from {} (see README)",
                missing.join(", "),
                home.join(BACKUPS_DIR).display()
            ),
            Self::Stopped => f.write_str("store thread stopped"),
            Self::Migration { name, source } => {
                write!(f, "migration {name} failed and was rolled back: {source}")
            }
            Self::Exists(what) => write!(f, "{what} already exists"),
            Self::UnknownTask(id) => write!(f, "no task {id}"),
            Self::Invalid(what) => write!(f, "invalid stored value: {what}"),
            Self::SnapshotCheck(what) => write!(f, "DB snapshot failed quick_check: {what}"),
            Self::Refused { path, reason } => {
                write!(f, "refusing to use {}: {reason}", path.display())
            }
            Self::Create { step, source } => {
                write!(f, "creating {DB_FILE} failed: {step}: {source}")
            }
            Self::Sqlite(e) => write!(f, "sqlite: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// The SQLite [`Store`]. Dropping it stops the DB thread and waits for it,
/// so the file lock is released when `drop` returns.
pub struct SqliteStore {
    home: PathBuf,
    jobs: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl SqliteStore {
    /// Opens `<home>/agend.db`, creating it only when no such file exists,
    /// and migrates it to [`LATEST_VERSION`]. `now_unix_ms` (from the daemon's `Clock`) dates
    /// the pre-upgrade DB snapshot.
    pub fn open(home: &Path, now_unix_ms: u64) -> Result<Self, StoreError> {
        Self::open_with(home, now_unix_ms, MIGRATIONS)
    }

    /// [`SqliteStore::open`] with an explicit migration list; the list is a
    /// parameter so tests can run a broken one through the real path.
    pub fn open_with(
        home: &Path,
        now_unix_ms: u64,
        migrations: &[Migration],
    ) -> Result<Self, StoreError> {
        let conn = open_connection(home, now_unix_ms, migrations)?;
        let (jobs, mut queue) = mpsc::channel::<Job>(QUEUE_CAPACITY);
        let thread = thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || {
                let mut conn = conn;
                while let Some(job) = queue.blocking_recv() {
                    job(&mut conn);
                }
            })?;
        Ok(Self {
            home: home.to_path_buf(),
            jobs: Some(jobs),
            thread: Some(thread),
        })
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn db_path(&self) -> PathBuf {
        self.home.join(DB_FILE)
    }

    /// Saves one workflow version (D19 TOML). A saved version never changes:
    /// saving the same (id, version) again fails with [`StoreError::Exists`].
    pub async fn save_workflow(&self, workflow: &Workflow) -> Result<(), StoreError> {
        let text = toml::to_string(workflow)
            .map_err(|e| StoreError::Invalid(format!("workflow {}: {e}", workflow.id)))?;
        let id = workflow.id.clone();
        let version = task_row::to_i64(workflow.version, "workflow version")?;
        self.call(move |conn| {
            let inserted = conn.execute(
                "INSERT INTO workflows (id, version, toml) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, version, text],
            );
            match inserted {
                Ok(_) => Ok(()),
                Err(e) if is_constraint(&e, PRIMARY_KEY) => Err(StoreError::Exists(format!(
                    "workflow {id} version {version}"
                ))),
                Err(e) => Err(e.into()),
            }
        })
        .await
    }

    /// The events of `task_id`, in append order.
    pub async fn load_events(&self, task_id: &str) -> Result<Vec<StoredEvent>, StoreError> {
        let task_id = task_id.to_owned();
        self.call(move |conn| task_row::load_events(conn, &task_id))
            .await
    }

    /// Every task, by id.
    pub async fn tasks(&self) -> Result<Vec<Task>, StoreError> {
        self.call(|conn| task_row::list(conn)).await
    }

    /// Every instance, by id.
    pub async fn instances(&self) -> Result<Vec<Instance>, StoreError> {
        self.call(|conn| instances::list(conn)).await
    }

    /// One instance, if it exists.
    pub async fn instance(&self, id: &str) -> Result<Option<Instance>, StoreError> {
        let id = id.to_owned();
        self.call(move |conn| instances::get(conn, &id)).await
    }

    /// Adds an instance; [`StoreError::Exists`] if the id is taken.
    pub async fn add_instance(&self, instance: &Instance) -> Result<(), StoreError> {
        let instance = instance.clone();
        self.call(move |conn| instances::insert(conn, &instance))
            .await
    }

    /// Removes an instance; false when there was none.
    pub async fn remove_instance(&self, id: &str) -> Result<bool, StoreError> {
        let id = id.to_owned();
        self.call(move |conn| instances::remove(conn, &id)).await
    }

    pub async fn set_instance_status(
        &self,
        id: &str,
        status: InstanceStatus,
    ) -> Result<(), StoreError> {
        let id = id.to_owned();
        self.call(move |conn| instances::set_status(conn, &id, status))
            .await
    }

    /// Row counts of every table in the retention table, in its order.
    pub async fn counts(&self) -> Result<Vec<(&'static str, u64)>, StoreError> {
        self.call(|conn| retention::counts(conn)).await
    }

    /// Deletes what is past its retention period at `now_unix_ms`, in one
    /// transaction, without VACUUM (P8).
    pub async fn prune(&self, now_unix_ms: u64) -> Result<PruneReport, StoreError> {
        self.call(move |conn| retention::prune(conn, now_unix_ms))
            .await
    }

    /// Writes today's (UTC) DB snapshot unless it exists, then keeps the 7
    /// newest (P9).
    pub async fn snapshot(&self, now_unix_ms: u64) -> Result<SnapshotReport, StoreError> {
        let home = self.home.clone();
        self.call(move |conn| snapshot::daily(conn, &home, now_unix_ms))
            .await
    }

    /// Runs `job` on the DB thread and returns its result.
    async fn call<T, F>(&self, job: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let jobs = self.jobs.as_ref().ok_or(StoreError::Stopped)?;
        let (reply, answer) = oneshot::channel();
        let job: Job = Box::new(move |conn| {
            let _ = reply.send(job(conn));
        });
        jobs.send(job).await.map_err(|_| StoreError::Stopped)?;
        answer.await.map_err(|_| StoreError::Stopped)?
    }
}

impl Drop for SqliteStore {
    fn drop(&mut self) {
        // Closing the channel ends the thread's loop; joining it closes the
        // connection, which releases the exclusive lock.
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Store for SqliteStore {
    type Error = StoreError;

    async fn load_task(&self, task_id: &str) -> Result<Option<VersionedTask>, StoreError> {
        let task_id = task_id.to_owned();
        self.call(move |conn| task_row::load(conn, &task_id)).await
    }

    async fn create_task(&self, task: &Task) -> Result<(), StoreError> {
        let task = task.clone();
        self.call(move |conn| task_row::insert(conn, &task)).await
    }

    async fn compare_and_swap_task(
        &self,
        task: &Task,
        expected_version: u64,
    ) -> Result<CasResult, StoreError> {
        let task = task.clone();
        self.call(move |conn| task_row::compare_and_swap(conn, &task, expected_version))
            .await
    }

    async fn load_workflow(
        &self,
        workflow_id: &str,
        version: u64,
    ) -> Result<Option<Workflow>, StoreError> {
        let workflow_id = workflow_id.to_owned();
        let Ok(version) = i64::try_from(version) else {
            return Ok(None);
        };
        self.call(move |conn| {
            let text: Option<String> = conn
                .query_row(
                    "SELECT toml FROM workflows WHERE id = ?1 AND version = ?2",
                    rusqlite::params![workflow_id, version],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    e => Err(e),
                })?;
            text.map(|text| {
                toml::from_str(&text).map_err(|e| {
                    StoreError::Invalid(format!("workflow {workflow_id} version {version}: {e}"))
                })
            })
            .transpose()
        })
        .await
    }

    async fn append_event(&self, task_id: &str, event: &StoredEvent) -> Result<(), StoreError> {
        let task_id = task_id.to_owned();
        let event = event.clone();
        self.call(move |conn| task_row::append_event(conn, &task_id, &event))
            .await
    }
}

/// `SQLITE_CONSTRAINT_PRIMARYKEY`.
const PRIMARY_KEY: i32 = 1555;
/// `SQLITE_CONSTRAINT_FOREIGNKEY`.
const FOREIGN_KEY: i32 = 787;

fn is_constraint(e: &rusqlite::Error, extended_code: i32) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.extended_code == extended_code)
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

/// Everything before the DB thread starts: directories and file modes,
/// creating a missing database, the exclusive lock, the version check, the
/// durability pragmas, the pre-upgrade snapshot and the migrations. Nothing
/// is written to an existing `agend.db` before the checks pass.
fn open_connection(
    home: &Path,
    now_unix_ms: u64,
    migrations: &[Migration],
) -> Result<Connection, StoreError> {
    create_private_dir(home)?;
    let db = home.join(DB_FILE);
    match fs::symlink_metadata(&db) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => create_database(home, &db, migrations)?,
        Err(e) => return Err(e.into()),
        Ok(_) => check_existing(home, &db)?,
    }
    let mut conn = lock(&db)?;
    let found = user_version(&conn)?;
    let supported = migrate::version_of(migrations);
    if found > supported {
        return Err(StoreError::TooNew {
            found,
            supported,
            home: home.to_path_buf(),
        });
    }
    if found == 0 {
        return Err(StoreError::NoSchema {
            home: home.to_path_buf(),
        });
    }
    let Ok(applied) = usize::try_from(found) else {
        return Err(StoreError::BadVersion {
            found,
            home: home.to_path_buf(),
        });
    };
    let missing = missing_tables(&conn, &migrations[..applied])?;
    if !missing.is_empty() {
        return Err(StoreError::MissingTables {
            found,
            missing,
            home: home.to_path_buf(),
        });
    }
    // A creation killed between linking and removing the build file's name
    // leaves a second name for this file; the lock on it is held here. A
    // build file that is another file belongs to a creator still running.
    let new = home.join(NEW_DB_FILE);
    if fs::symlink_metadata(&new).is_ok_and(|m| fs::metadata(&db).is_ok_and(|d| same_file(&m, &d)))
    {
        remove_if_present(&new)?;
        remove_if_present(&home.join(format!("{NEW_DB_FILE}-journal")))?;
    }
    let journal: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
    if journal != "wal" {
        return Err(StoreError::Invalid(format!("journal_mode is {journal}")));
    }
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    if found < supported {
        snapshot::pre_upgrade(&conn, home, now_unix_ms, supported)?;
    }
    migrate::apply(&mut conn, found, migrations)?;
    Ok(conn)
}

/// What can be refused about an existing `agend.db` before SQLite opens it
/// (opening a file shorter than the header makes SQLite write one).
fn check_existing(home: &Path, db: &Path) -> Result<(), StoreError> {
    let meta = match fs::metadata(db) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(StoreError::Refused {
                path: db.to_path_buf(),
                reason: "it is a symlink to a missing file; restore the link's target or \
                         remove the link",
            });
        }
        found => found?,
    };
    if !meta.is_file() {
        return Err(StoreError::Refused {
            path: db.to_path_buf(),
            reason: "it is not a regular file",
        });
    }
    if meta.len() < SQLITE_HEADER_LEN {
        return Err(StoreError::Empty {
            len: meta.len(),
            home: home.to_path_buf(),
        });
    }
    Ok(())
}

/// Builds a new database in [`NEW_DB_FILE`] (rollback journal, every
/// migration committed) and publishes it as `db`.
///
/// Creators are serialised by the build file's exclusive lock: only the
/// process holding the lock on the file currently at `.agend.db.new` changes
/// that path (removes a leftover one, or publishes its own). A leftover is
/// locked before it is removed and that lock is kept until the new build
/// file is created and locked, so a creator that opened the old file gets
/// [`StoreError::InUse`]; one whose file was replaced between its create and
/// its lock finds a different file at the path and stops with `InUse` too.
fn create_database(home: &Path, db: &Path, migrations: &[Migration]) -> Result<(), StoreError> {
    let new = home.join(NEW_DB_FILE);
    let leftover = remove_leftover_build(home, &new)?;
    // SQLite would create the file with the umask's mode; create it 0600
    // first (an empty file is an empty database). SQLite gives the -wal
    // file the database file's mode.
    let file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&new)
    {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(StoreError::InUse),
        opened => opened?,
    };
    let mut conn = lock_build(&new)?;
    drop(leftover);
    let at_path = match fs::symlink_metadata(&new) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(StoreError::InUse),
        found => found?,
    };
    if !same_file(&file.metadata()?, &at_path) {
        return Err(StoreError::InUse);
    }
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate::apply(&mut conn, 0, migrations)?;
    publish(&new, db)?;
    conn.close().map_err(|(_, e)| e)?;
    File::open(home)?.sync_all()?;
    Ok(())
}

/// Removes a build file left by a creation that failed or was killed. It
/// holds no user data (it was never published), so the database is built
/// again from scratch instead of finishing it: its mode, version and content
/// are whatever the crash left. Returns the lock taken on it, which the
/// caller keeps until its own build file is locked; [`StoreError::InUse`]
/// when a creator is building it right now.
fn remove_leftover_build(home: &Path, new: &Path) -> Result<Option<Connection>, StoreError> {
    let journal = home.join(format!("{NEW_DB_FILE}-journal"));
    let meta = match fs::symlink_metadata(new) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // A journal without its database would be rolled back into the
            // new build file.
            remove_if_present(&journal)?;
            return Ok(None);
        }
        found => found?,
    };
    if !meta.file_type().is_file() || meta.uid() != fs::metadata(home)?.uid() {
        return Err(StoreError::Refused {
            path: new.to_path_buf(),
            reason: "a leftover build file that is not a regular file owned by the home's \
                     owner; remove it by hand",
        });
    }
    let held = match lock_build(new) {
        Ok(conn) => Some(conn),
        // Not a database, so no creator holds a lock on it.
        Err(StoreError::Sqlite(e)) if e.sqlite_error_code() == Some(ErrorCode::NotADatabase) => {
            None
        }
        Err(e) => return Err(e),
    };
    remove_if_present(new)?;
    remove_if_present(&journal)?;
    Ok(held)
}

/// Makes the build file `db`, never replacing an existing `db`: a hard
/// link, then the build file's name is removed. On a filesystem without
/// hard links (ExFAT, some network filesystems) it renames the build file
/// instead, after checking again that `db` is absent. rename replaces an
/// existing `db`, so the check and the rename must not race another
/// creator: the caller holds the build file's lock, and only the holder of
/// that lock publishes (see [`create_database`]), so no other creator can
/// create `db` in between. What is left is a process outside agend creating
/// `agend.db` in that window, which the hard link would have refused.
fn publish(new: &Path, db: &Path) -> Result<(), StoreError> {
    let link_error = match hard_link(new, db) {
        Ok(()) => {
            return fs::remove_file(new).map_err(|source| StoreError::Create {
                step: format!("removing {NEW_DB_FILE} after linking it"),
                source,
            });
        }
        // A creator that finished before this one took the lock.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(e) => e,
    };
    match fs::symlink_metadata(db) {
        Ok(_) => return Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(StoreError::Create {
                step: format!("checking {DB_FILE} is still absent"),
                source,
            });
        }
    }
    fs::rename(new, db).map_err(|source| StoreError::Create {
        step: format!(
            "renaming {NEW_DB_FILE} to {DB_FILE} (hard-linking it failed first: {link_error})"
        ),
        source,
    })
}

/// [`fs::hard_link`]; unit tests can make it fail as on a filesystem
/// without hard links.
fn hard_link(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(test)]
    if tests::NO_HARD_LINKS.with(std::cell::Cell::get) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Operation not supported (test: no hard links)",
        ));
    }
    fs::hard_link(src, dst)
}

fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    (a.dev(), a.ino()) == (b.dev(), b.ino())
}

/// Opens `path` read-write and takes the exclusive lock, held until the
/// connection closes.
fn lock(path: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // Fail at once instead of waiting: a busy file means another daemon.
    conn.busy_timeout(Duration::ZERO)?;
    let mode: String = conn.query_row("PRAGMA locking_mode = EXCLUSIVE", [], |r| r.get(0))?;
    if mode != "exclusive" {
        return Err(StoreError::Invalid(format!("locking_mode is {mode}")));
    }
    // An empty exclusive transaction takes the exclusive file lock; in
    // EXCLUSIVE locking mode it is kept until the connection closes. It
    // writes nothing.
    conn.execute_batch("BEGIN EXCLUSIVE; COMMIT;")
        .map_err(|e| {
            if is_busy(&e) {
                StoreError::InUse
            } else {
                e.into()
            }
        })?;
    Ok(conn)
}

/// [`lock`] on the build file. The file vanishing before SQLite opens it
/// means another creator published or removed it: [`StoreError::InUse`].
fn lock_build(new: &Path) -> Result<Connection, StoreError> {
    match lock(new) {
        Err(StoreError::Sqlite(e))
            if e.sqlite_error_code() == Some(ErrorCode::CannotOpen)
                && fs::symlink_metadata(new)
                    .is_err_and(|m| m.kind() == io::ErrorKind::NotFound) =>
        {
            Err(StoreError::InUse)
        }
        locked => locked,
    }
}

/// Tables that the `applied` migrations create but `conn` lacks. The
/// expected list comes from running those migrations on an in-memory
/// database, so it is right for every recorded version (the tests pin the
/// latest one to `golden/schema.sql`). Only reads `conn`.
fn missing_tables(conn: &Connection, applied: &[Migration]) -> Result<Vec<String>, StoreError> {
    let mut expected = Connection::open_in_memory()?;
    migrate::apply(&mut expected, 0, applied)?;
    let present = table_names(conn)?;
    Ok(table_names(&expected)?
        .into_iter()
        .filter(|name| !present.contains(name))
        .collect())
}

fn table_names(conn: &Connection) -> Result<Vec<String>, StoreError> {
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
    let names = stmt.query_map([], |row| row.get(0))?;
    Ok(names.collect::<Result<_, _>>()?)
}

fn user_version(conn: &Connection) -> Result<i64, StoreError> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

fn remove_if_present(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Creates `dir` and its missing parents with mode 0700; directories that
/// already exist are left as they are.
fn create_private_dir(dir: &Path) -> Result<(), StoreError> {
    Ok(DirBuilder::new().recursive(true).mode(0o700).create(dir)?)
}

#[cfg(test)]
mod tests;
