//! Gate 5 (store) demo, run by `cargo xtask accept store`. Every section
//! works in one fresh temporary directory (deleted at the end unless
//! `AGEND_STORE_DEMO_KEEP=1`) with a fake clock; restarts, the crash and the
//! second open are real child processes of this binary.
//!
//! Sections: `== migrate`, `== restart` (with the `stale` writer),
//! `== crash`, `== retention` (with the v1-scale calibration),
//! `== snapshot`, `== too-new`, `== second-open`.
#![cfg_attr(not(unix), allow(unused))]

#[cfg(unix)]
#[path = "../tests/common/store_process.rs"]
mod common;

#[cfg(not(unix))]
fn main() {
    eprintln!("the store demo needs unix");
    std::process::exit(1);
}

#[cfg(unix)]
fn main() {
    if common::run_child_role() {
        return;
    }
    if let Err(e) = demo::run() {
        eprintln!("store demo FAILED: {e}");
        std::process::exit(1);
    }
}

#[cfg(unix)]
mod demo {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Instant;

    use agend_core::pipeline::task::{Task, TaskStatus};
    use agend_core::pipeline::workflow::Workflow;
    use agend_core::traits::{CasResult, Clock, Store, StoredEvent};
    use agend_daemon::store::retention::DAY_MS;
    use agend_daemon::store::snapshot;
    use agend_daemon::store::{BACKUPS_DIR, DB_FILE, LATEST_VERSION, SqliteStore};
    use agend_testkit::block_on;
    use agend_testkit::fakes::FakeClock;
    use agend_testkit::tempdir::TempDir;
    use rusqlite::{Connection, OpenFlags};
    use sha2::{Digest, Sha256};

    use super::common;

    type R<T = ()> = Result<T, String>;

    /// v1's task count (docs/V1-LESSONS.md: 8,347 tasks).
    const V1_TASKS: u64 = 8_347;
    /// Events per task in the calibration: an upper bound with every task's
    /// events still inside the 14-day window.
    const EVENTS_PER_TASK: u64 = 20;
    const DB_LIMIT: u64 = 1 << 30;
    const SNAPSHOTS_LIMIT: u64 = 5 << 30;

    fn base() -> Command {
        Command::new(std::env::current_exe().expect("current_exe"))
    }

    fn e(err: impl std::fmt::Display) -> String {
        err.to_string()
    }

    fn check(ok: bool, what: impl FnOnce() -> String) -> R {
        if ok { Ok(()) } else { Err(what()) }
    }

    fn mode_string(path: &Path) -> R<String> {
        let meta = fs::metadata(path).map_err(e)?;
        let mode = meta.permissions().mode();
        let mut s = String::from(if meta.is_dir() { "d" } else { "-" });
        for shift in [6, 3, 0] {
            let bits = (mode >> shift) & 7;
            s.push(if bits & 4 != 0 { 'r' } else { '-' });
            s.push(if bits & 2 != 0 { 'w' } else { '-' });
            s.push(if bits & 1 != 0 { 'x' } else { '-' });
        }
        Ok(s)
    }

    fn sha256(path: &Path) -> R<String> {
        Ok(format!("{:x}", Sha256::digest(fs::read(path).map_err(e)?)))
    }

    fn mb(bytes: u64) -> String {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    }

    fn counts(store: &SqliteStore) -> R<String> {
        let counts = block_on(store.counts()).map_err(e)?;
        Ok(counts
            .iter()
            .map(|(t, n)| format!("{t} {n}"))
            .collect::<Vec<_>>()
            .join(", "))
    }

    pub fn run() -> R {
        let keep = std::env::var_os("AGEND_STORE_DEMO_KEEP").is_some_and(|v| v == "1");
        let dir = TempDir::new("store-demo").map_err(e)?;
        let root = dir.path().to_path_buf();
        println!("demo directory {}", root.display());
        let clock = FakeClock::default();

        let home = root.join("home");
        let store = migrate(&home, &clock)?;
        restart(&root)?;
        crash(&root)?;
        retention(&store, &clock, &root)?;
        let snap = snapshots(&store, &clock)?;
        too_new(&root, &snap)?;
        second_open(&store, &home)?;
        drop(store);

        if keep {
            std::mem::forget(dir);
            println!("kept {} (delete it when done)", root.display());
            println!("export SNAP={}", snap.display());
        }
        Ok(())
    }

    fn migrate(home: &Path, clock: &FakeClock) -> R<SqliteStore> {
        println!("\n== migrate");
        let db = home.join(DB_FILE);
        let before = if db.exists() {
            Connection::open(&db)
                .and_then(|c| c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)))
                .map_err(e)?
        } else {
            0
        };
        let store = SqliteStore::open(home, clock.now_unix_ms()).map_err(e)?;
        println!("schema {before} -> {LATEST_VERSION}");
        for n in 1..=3 {
            let mut task = Task::new(
                format!("T-{n}"),
                format!("demo task {n}"),
                "general",
                "code",
                1,
            );
            task.depends_on = (1..n).map(|d| format!("T-{d}")).collect();
            block_on(store.create_task(&task)).map_err(e)?;
        }
        let mut workflow = Workflow::builtin_code();
        block_on(store.save_workflow(&workflow)).map_err(e)?;
        workflow.version += 1;
        block_on(store.save_workflow(&workflow)).map_err(e)?;
        for n in 0..120u64 {
            let event = StoredEvent {
                id: format!("e-{n}"),
                occurred_at_unix_ms: clock.peek() + n,
                kind: "stage_completed".into(),
                detail: format!("demo event {n}"),
            };
            block_on(store.append_event(&format!("T-{}", n % 3 + 1), &event)).map_err(e)?;
        }
        println!("rows: {}", counts(&store)?);
        println!("home {}", mode_string(home)?);
        println!("{DB_FILE} {}", mode_string(&db)?);
        Ok(store)
    }

    fn restart(root: &Path) -> R {
        println!("\n== restart");
        let home = root.join("restart");
        for line in common::four_boots(&base, &|_| home.clone())? {
            println!("{line}");
        }
        println!("4 boots, 4 different pids, each child exited before the next started");
        let fresh = root.join("restart-negative");
        fs::create_dir(&fresh).map_err(e)?;
        match common::four_boots(&base, &|boot| fresh.join(format!("home-{boot}"))) {
            Ok(_) => return Err("a new DB path each boot passed; the check has no teeth".into()),
            Err(error) => {
                let first = error.split(": thread").next().unwrap_or(&error);
                check(error.contains("task T-1 is missing"), || error.clone())?;
                println!("negative check (new DB path each boot): {first} (task T-1 is missing)");
            }
        }
        Ok(())
    }

    fn crash(root: &Path) -> R {
        println!("\n== crash");
        let crash = common::crash(&base, &root.join("crash"))?;
        println!(
            "child pid={} acked {} writes; killed own child pid={} ({})",
            crash.pid, crash.acked_before_kill, crash.pid, crash.status
        );
        println!(
            "acked={} found={} integrity={}",
            crash.last_acked, crash.found, crash.integrity
        );
        check(
            crash.found >= crash.last_acked && crash.integrity == "ok",
            || "an acknowledged write was lost or the DB is damaged".into(),
        )
    }

    fn retention(store: &SqliteStore, clock: &FakeClock, root: &Path) -> R {
        println!("\n== retention");
        clock.advance(15 * DAY_MS);
        let report = block_on(store.prune(clock.now_unix_ms())).map_err(e)?;
        let line = report
            .tables
            .iter()
            .map(|t| format!("{} {} -> {}", t.table, t.before, t.after))
            .collect::<Vec<_>>()
            .join(", ");
        println!("fake clock +15d: {line}");
        calibrate(&root.join("calibration"))
    }

    /// v1-scale data: one task and its events written through the store (the
    /// real producer), replicated by SQL to v1's size, then 7 daily snapshots.
    fn calibrate(home: &Path) -> R {
        let clock = FakeClock::default();
        let store = SqliteStore::open(home, clock.now_unix_ms()).map_err(e)?;
        let mut task = Task::new(
            "T-template",
            "Fix the lock ordering in the scheduler when two reviewers approve at once",
            "general",
            "code",
            1,
        )
        .set_requires_repo(true);
        task.depends_on = vec!["T-00001".into(), "T-00002".into()];
        task.assignee = Some("dev-claude-1".into());
        task.status = TaskStatus::Done;
        task.merge_commit = Some("9f2c4e1b7a3d5f6e8c0b1a2d3e4f5a6b7c8d9e0f".into());
        block_on(store.create_task(&task)).map_err(e)?;
        block_on(store.save_workflow(&Workflow::builtin_code())).map_err(e)?;
        for n in 0..EVENTS_PER_TASK {
            let event = StoredEvent {
                id: format!("evt-{n:02}"),
                occurred_at_unix_ms: clock.peek() + n,
                kind: "stage_completed".into(),
                detail: format!(
                    "stage review attempt {n}: reviewer codex-2 approved head \
                     9f2c4e1b7a3d5f6e8c0b1a2d3e4f5a6b7c8d9e0f after checks passed \
                     (cargo test 312 passed, clippy clean); summary: tightened the \
                     lock order and added a regression test for the double approval"
                ),
            };
            block_on(store.append_event("T-template", &event)).map_err(e)?;
        }
        drop(store);
        let started = Instant::now();
        let conn = Connection::open(home.join(DB_FILE)).map_err(e)?;
        conn.execute_batch(&format!(
            "BEGIN;
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {V1_TASKS} - 1)
             INSERT INTO tasks
               SELECT printf('T-%05d', i), title, team_id, workflow_id, workflow_version, parent,
                      depends_on, superseded_by, assignee, status, requires_repo, merge_commit,
                      version
               FROM n, tasks WHERE id = 'T-template';
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {V1_TASKS} - 1)
             INSERT INTO task_events (task_id, event_id, occurred_at_unix_ms, kind, detail)
               SELECT printf('T-%05d', i), event_id, occurred_at_unix_ms, kind, detail
               FROM n, task_events WHERE task_id = 'T-template';
             COMMIT;"
        ))
        .map_err(e)?;
        drop(conn);
        let fill = started.elapsed();
        let store = SqliteStore::open(home, clock.now_unix_ms()).map_err(e)?;
        println!(
            "calibration: v1 scale, {} (filled in {:.1} s)",
            counts(&store)?,
            fill.as_secs_f64()
        );
        let mut total = 0;
        let mut slowest = 0f64;
        for _ in 0..snapshot::KEEP {
            let report = block_on(store.snapshot(clock.now_unix_ms())).map_err(e)?;
            total += report.bytes;
            slowest = slowest.max(report.elapsed.as_secs_f64());
            clock.advance(DAY_MS);
        }
        drop(store);
        let db_bytes = fs::metadata(home.join(DB_FILE)).map_err(e)?.len()
            + fs::metadata(home.join(format!("{DB_FILE}-wal")))
                .map(|m| m.len())
                .unwrap_or(0);
        println!(
            "calibration: agend.db {} (limit 1 GB), {} snapshots {} (limit 5 GB), \
             slowest VACUUM INTO + quick_check {slowest:.2} s",
            mb(db_bytes),
            snapshot::KEEP,
            mb(total)
        );
        let verdict = if db_bytes <= DB_LIMIT && total <= SNAPSHOTS_LIMIT {
            "within limits: D31 retention periods and 7 snapshots stay"
        } else {
            "OVER the limits: D31 needs recalibrating"
        };
        println!("calibration: {verdict}");
        check(db_bytes <= DB_LIMIT && total <= SNAPSHOTS_LIMIT, || {
            verdict.into()
        })
    }

    fn snapshots(store: &SqliteStore, clock: &FakeClock) -> R<PathBuf> {
        println!("\n== snapshot");
        let first = block_on(store.snapshot(clock.now_unix_ms())).map_err(e)?;
        let quick: String =
            Connection::open_with_flags(&first.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .and_then(|c| c.query_row("PRAGMA quick_check", [], |r| r.get(0)))
                .map_err(e)?;
        println!(
            "{} {} bytes quick_check {quick} ({}, {:.3} s)",
            first.path.display(),
            first.bytes,
            mode_string(&first.path)?,
            first.elapsed.as_secs_f64()
        );
        let backups = first
            .path
            .parent()
            .ok_or("snapshot has no directory")?
            .to_path_buf();
        fs::write(backups.join("notes.txt"), "my notes\n").map_err(e)?;
        let mut last = first;
        for _ in 1..9 {
            clock.advance(DAY_MS);
            last = block_on(store.snapshot(clock.now_unix_ms())).map_err(e)?;
        }
        let kept = &last.kept;
        println!(
            "fake clock 9 days: backups: {} files ({} .. {}), notes.txt {}",
            kept.len(),
            kept.first().map_or("", String::as_str),
            kept.last().map_or("", String::as_str),
            if backups.join("notes.txt").exists() {
                "kept"
            } else {
                "DELETED"
            }
        );
        check(
            kept.len() == snapshot::KEEP && backups.join("notes.txt").exists(),
            || format!("rotation kept {kept:?}"),
        )?;
        println!("{} is {}", BACKUPS_DIR, mode_string(&backups)?);
        Ok(last.path)
    }

    fn too_new(root: &Path, snap: &Path) -> R {
        println!("\n== too-new");
        let home = root.join("too-new");
        fs::create_dir(&home).map_err(e)?;
        let db = home.join(DB_FILE);
        fs::copy(snap, &db).map_err(e)?;
        Connection::open(&db)
            .and_then(|c| c.pragma_update(None, "user_version", LATEST_VERSION + 1))
            .map_err(e)?;
        let before = sha256(&db)?;
        println!(
            "copy of a snapshot with user_version {}",
            LATEST_VERSION + 1
        );
        let error = match SqliteStore::open(&home, 0) {
            Ok(_) => return Err("a too-new database opened".into()),
            Err(error) => error.to_string(),
        };
        println!("open: {error}");
        let after = sha256(&db)?;
        println!("sha256 before {before}");
        println!("sha256 after  {after}");
        check(before == after, || "the too-new database changed".into())?;
        println!("unchanged");
        Ok(())
    }

    fn second_open(store: &SqliteStore, home: &Path) -> R {
        println!("\n== second-open");
        println!(
            "first store (pid={}) holds {}",
            std::process::id(),
            home.join(DB_FILE).display()
        );
        let line = common::second_open(&base, home)?;
        println!("{line}");
        let loaded = block_on(store.load_task("T-1"))
            .map_err(e)?
            .ok_or("T-1 is missing")?;
        let mut task = loaded.task.clone();
        task.title = "written after the second open".into();
        match block_on(store.compare_and_swap_task(&task, loaded.version)).map_err(e)? {
            CasResult::Written { new_version } => {
                println!(
                    "first store still writes: T-1 v={} -> v={new_version}",
                    loaded.version
                );
                Ok(())
            }
            other => Err(format!("first store could not write: {other:?}")),
        }
    }
}
