//! `agend daemon preflight <dir>` (gate 9 P7, D2): what a new binary must
//! get through before a running daemon `exec`s it. `<dir>` is a temporary
//! home the old daemon made (`/tmp/agend-pf-XXXXXX`) with a fresh copy of
//! its `agend.db`; the real home is never touched.
//!
//! 1. The copy with this binary's migrations: schema version before and
//!    after, `PRAGMA quick_check`, one read of `instances` (a database newer
//!    than this binary fails here, like any open).
//! 2. A holder of this binary in `<dir>`: `agend holder pf-check`, hello,
//!    `Spawn` of `/bin/sh -c 'sleep 60'`, `Shutdown`, and its lock released.
//!
//! Prints one line per step on stdout (the old daemon returns them to the
//! CLI); the first is `agend <version>`. Exit 0 when every step passed,
//! otherwise 1 with the reason on stderr.
//!
//! Must NOT: open or connect to anything outside `<dir>`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use agend_core::model::Backend;
use agend_core::traits::HolderLaunch;

use crate::log;
use crate::runtime::{HolderRuntime, SpawnOutcome};
use crate::store::{DB_FILE, LATEST_VERSION, SqliteStore};

/// The preflight holder's instance id.
pub const HOLDER_ID: &str = "pf-check";

pub fn run(args: Vec<OsString>) -> ExitCode {
    let [dir] = args.as_slice() else {
        eprintln!("usage: agend daemon preflight <dir>   (run by agend daemon restart)");
        return ExitCode::from(2);
    };
    let dir = PathBuf::from(dir);
    println!("agend {}", env!("CARGO_PKG_VERSION"));
    let steps = [db_copy(&dir), holder(&dir)];
    for step in steps {
        match step {
            Ok(line) => println!("{line}"),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}

/// Step 1: the database copy with this binary's migrations.
fn db_copy(dir: &Path) -> Result<String, String> {
    let from: i64 = rusqlite::Connection::open_with_flags(
        dir.join(DB_FILE),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .and_then(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0)))
    .map_err(|e| format!("db copy: cannot read its schema version: {e}"))?;
    let store = SqliteStore::open(dir, log::now_unix_ms()).map_err(|e| format!("db copy: {e}"))?;
    let runtime = current_thread()?;
    let check = runtime
        .block_on(store.quick_check())
        .map_err(|e| format!("db copy: quick_check: {e}"))?;
    if check != "ok" {
        return Err(format!("db copy: quick_check: {check}"));
    }
    let instances = runtime
        .block_on(store.instances())
        .map_err(|e| format!("db copy: read instances: {e}"))?;
    Ok(format!(
        "db copy: migrations {from} -> {LATEST_VERSION}, quick_check ok, instances {}",
        instances.len()
    ))
}

/// Step 2: one holder of this binary in `dir`.
fn holder(dir: &Path) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("holder: own binary: {e}"))?;
    let runtime = current_thread()?;
    let holders = HolderRuntime::new(dir, &exe, Vec::new(), Arc::new(|_| {}));
    let launch = HolderLaunch {
        instance_id: HOLDER_ID.into(),
        backend: Backend::Claude,
        executable: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 60".into()],
        working_directory: dir.display().to_string(),
    };
    let started = runtime
        .block_on(holders.start(&launch))
        .map_err(|e| format!("holder: {e}"))?;
    let spawned = matches!(started.attached.spawn, Some(SpawnOutcome::Spawned { .. }));
    let stopped = runtime.block_on(holders.stop(HOLDER_ID));
    if !spawned {
        return Err(format!(
            "holder: hello ok, spawn answered {:?}",
            started.attached.spawn
        ));
    }
    stopped.map_err(|e| format!("holder: hello ok, spawn ok, shutdown: {e}"))?;
    Ok("holder: hello ok, spawn ok, shutdown ok".into())
}

fn current_thread() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot build a runtime: {e}"))
}
