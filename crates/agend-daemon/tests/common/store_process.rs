//! Real-process restarts of the SQLite store (gate 5 P6, P7), shared by
//! `tests/store_process.rs` and `examples/store_demo.rs` (`#[path]`).
//!
//! The parent re-executes its own binary once per daemon boot; an
//! environment variable picks the child's role. The parent waits for each
//! child to exit before starting the next, so no store handle and no child
//! is alive between boots. Children print result lines starting with `@`.
//!
//! Safety: every child is this binary, started with a fresh temporary
//! directory as its home and cwd, with every `AGEND_*` variable removed. The
//! only signal sent is `Child::kill()` on a child this module spawned; every
//! wait has a deadline.
//!
//! Must NOT: signal any process it did not spawn, or use an in-memory
//! database.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use agend_core::pipeline::task::{Task, TaskStatus};
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{CasResult, Store, StoredEvent};
use agend_daemon::store::{DB_FILE, SqliteStore, StoreError};
use agend_testkit::block_on;

pub const ROLE_ENV: &str = "STORE_PROCESS_ROLE";
pub const HOME_ENV: &str = "STORE_PROCESS_HOME";
pub const STALE_ENV: &str = "STORE_PROCESS_STALE";

/// Fixed "now" for children (2026-09-21T13:46:40Z); no child depends on it.
pub const NOW: u64 = 1_790_000_000_000;
/// Longest a child may run before the parent kills it and fails.
pub const CHILD_DEADLINE: Duration = Duration::from_secs(60);
/// Acks the crash test waits for before killing its child.
pub const ACKS_BEFORE_KILL: u64 = 50;

const TASK: &str = "T-1";
const CRASH_TASK: &str = "T-crash";

/// Builds the command that re-executes this binary as a child (the test
/// harness adds its filter arguments; the demo needs none).
pub type Base<'a> = &'a dyn Fn() -> Command;

// ---- child side ----

/// Runs the role named by [`ROLE_ENV`], if any. Returns false when this
/// process is not a child. A failing role panics (nonzero exit).
pub fn run_child_role() -> bool {
    let Some(role) = std::env::var_os(ROLE_ENV) else {
        return false;
    };
    let home = PathBuf::from(std::env::var_os(HOME_ENV).expect("child needs STORE_PROCESS_HOME"));
    assert!(home.is_absolute(), "child home must be absolute");
    let pid = std::process::id();
    match role.to_str().expect("role is UTF-8") {
        "boot1" => boot1(&home, pid),
        "boot2" => {
            let store = open(&home);
            drop(store);
            say(&format!("boot 2 pid={pid} (open only)"));
        }
        "boot3" => boot3(&home, pid),
        "boot4" => boot4(&home, pid),
        "crash" => crash_writer(&home),
        "second-open" => match SqliteStore::open(&home, NOW) {
            Err(error @ StoreError::InUse) => say(&format!("second-open pid={pid} error={error}")),
            Err(error) => panic!("second open failed with the wrong error: {error}"),
            Ok(_) => panic!("second open succeeded while another process holds agend.db"),
        },
        other => panic!("unknown role {other}"),
    }
    true
}

/// Prints a result line. It starts on a line of its own: the test harness
/// may have printed `test child_process_entry ... ` without a newline.
fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "\n@{line}").expect("parent went away");
    out.flush().expect("parent went away");
}

fn open(home: &Path) -> SqliteStore {
    SqliteStore::open(home, NOW).unwrap_or_else(|e| panic!("open {}: {e}", home.display()))
}

fn titled(title: &str) -> Task {
    let mut task = Task::new(TASK, title, "general", "code", 1);
    task.depends_on = vec!["T-0".into()];
    task.status = TaskStatus::Running;
    task
}

fn event(n: u64) -> StoredEvent {
    StoredEvent {
        id: format!("e-{n}"),
        occurred_at_unix_ms: NOW + n,
        kind: "stage_completed".into(),
        detail: format!("event {n}"),
    }
}

fn cas(store: &SqliteStore, task: &Task, version: u64) -> CasResult {
    block_on(store.compare_and_swap_task(task, version)).expect("compare_and_swap_task")
}

fn written(result: CasResult) -> u64 {
    match result {
        CasResult::Written { new_version } => new_version,
        other => panic!("expected Written, got {other:?}"),
    }
}

/// Current version and task of T-1, which must exist.
fn load(store: &SqliteStore) -> (u64, Task) {
    let loaded = block_on(store.load_task(TASK))
        .expect("load_task")
        .unwrap_or_else(|| panic!("task {TASK} is missing"));
    (loaded.version, loaded.task)
}

/// Checks the workflow and the events every boot must find.
fn check_common(store: &SqliteStore, events: u64) {
    let workflow = block_on(store.load_workflow("code", 1)).expect("load_workflow");
    assert_eq!(workflow, Some(Workflow::builtin_code()), "workflow code v1");
    let stored = block_on(store.load_events(TASK)).expect("load_events");
    let expected: Vec<StoredEvent> = (1..=events).map(event).collect();
    assert_eq!(stored, expected, "events of {TASK}");
}

/// Boot 1: create the task, a workflow and 3 events, then 3 CAS writes.
/// The version held before the last write is handed to boot 3 as a stale
/// writer's version.
fn boot1(home: &Path, pid: u32) {
    let store = open(home);
    block_on(store.create_task(&titled("created"))).expect("create_task");
    block_on(store.save_workflow(&Workflow::builtin_code())).expect("save_workflow");
    for n in 1..=3 {
        block_on(store.append_event(TASK, &event(n))).expect("append_event");
    }
    let (mut version, _) = load(&store);
    let mut stale = version;
    for n in 1..=3 {
        stale = version;
        version = written(cas(&store, &titled(&format!("boot 1 write {n}")), version));
    }
    say(&format!("boot 1 pid={pid} v={version} stale={stale}"));
}

/// Boot 3: everything from boot 1 is there; the stale writer from before
/// the restart conflicts and changes nothing; the current version writes.
fn boot3(home: &Path, pid: u32) {
    let stale: u64 = std::env::var(STALE_ENV)
        .expect("boot 3 needs STORE_PROCESS_STALE")
        .parse()
        .expect("stale version");
    let store = open(home);
    let (current, task) = load(&store);
    assert_eq!(task, titled("boot 1 write 3"), "task after boot 1");
    check_common(&store, 3);
    let result = cas(&store, &titled("stale writer"), stale);
    assert_eq!(
        result,
        CasResult::Conflict {
            current_version: Some(current)
        },
        "stale writer"
    );
    assert_eq!(
        load(&store),
        (current, task),
        "stale write changed the task"
    );
    let version = written(cas(&store, &titled("boot 3"), current));
    assert!(version > current, "version must grow");
    block_on(store.append_event(TASK, &event(4))).expect("append_event");
    say(&format!(
        "boot 3 pid={pid} stale v={stale} -> conflict current={current}, task unchanged; v={version}"
    ));
}

/// Boot 4: checks everything, then one CAS to see the next version.
fn boot4(home: &Path, pid: u32) {
    let store = open(home);
    let (version, task) = load(&store);
    assert_eq!(task, titled("boot 3"), "task after boot 3");
    check_common(&store, 4);
    let next = written(cas(&store, &titled("boot 4"), version));
    assert!(next > version, "version must grow");
    say(&format!(
        "boot 4 pid={pid} v={version} ok (task, workflow, 4 events; next cas v={next})"
    ));
}

/// Writes T-crash by CAS forever, printing `acked <n>` after the n-th
/// write committed. It stops only when killed (or when its parent is gone:
/// the next print fails).
fn crash_writer(home: &Path) {
    let store = open(home);
    let task = Task::new(CRASH_TASK, "crash writer", "general", "code", 1);
    block_on(store.create_task(&task)).expect("create_task");
    let mut version = 1;
    for n in 1..=1_000_000u64 {
        let mut next = task.clone();
        next.title = format!("write {n}");
        version = written(cas(&store, &next, version));
        say(&format!("acked {n}"));
    }
}

// ---- parent side ----

/// The child command: this binary, the role, a clean `AGEND_*`-free
/// environment, cwd = the home's parent (an absolute temporary directory).
fn command(base: Base<'_>, role: &str, home: &Path, extra: &[(&str, String)]) -> Command {
    assert!(home.is_absolute(), "home must be absolute");
    let mut cmd = base();
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("AGEND_") {
            cmd.env_remove(key);
        }
    }
    cmd.env(ROLE_ENV, role)
        .env(HOME_ENV, home)
        .current_dir(home.parent().expect("home has a parent"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in extra {
        cmd.env(key, value);
    }
    cmd
}

/// Spawns `cmd` with a thread forwarding its stdout lines.
fn spawn(mut cmd: Command) -> Result<(Child, Receiver<String>, Receiver<String>), String> {
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot spawn child: {e}"))?;
    let forward = |pipe: Box<dyn std::io::Read + Send>| {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(pipe).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        rx
    };
    let stdout = forward(Box::new(child.stdout.take().expect("piped stdout")));
    let stderr = forward(Box::new(child.stderr.take().expect("piped stderr")));
    Ok((child, stdout, stderr))
}

/// Waits for `child` until `deadline`; kills it (our own child) on timeout.
fn wait(child: &mut Child, deadline: Instant) -> Result<ExitStatus, String> {
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("child pid={} timed out and was killed", child.id()));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Every line still coming from `rx` until the child's pipe closes.
fn drain(rx: &Receiver<String>, deadline: Instant) -> Vec<String> {
    let mut lines = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) => lines.push(line),
            Err(RecvTimeoutError::Disconnected | RecvTimeoutError::Timeout) => return lines,
        }
    }
}

fn results(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix('@'))
        .map(str::to_owned)
        .collect()
}

/// A child run to completion: its pid and its `@` lines.
pub struct Finished {
    pub pid: u32,
    pub results: Vec<String>,
}

fn run_to_end(cmd: Command) -> Result<Finished, String> {
    let deadline = Instant::now() + CHILD_DEADLINE;
    let (mut child, stdout, stderr) = spawn(cmd)?;
    let pid = child.id();
    let status = wait(&mut child, deadline)?;
    let out = drain(&stdout, deadline);
    if !status.success() {
        let err = drain(&stderr, deadline);
        let panic = err
            .iter()
            .skip_while(|l| !l.contains("panicked"))
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        return Err(format!("child pid={pid} {status}: {panic}"));
    }
    Ok(Finished {
        pid,
        results: results(&out),
    })
}

fn only_result(run: &Finished, prefix: &str) -> Result<String, String> {
    match run.results.as_slice() {
        [line] if line.starts_with(prefix) => Ok(line.clone()),
        other => Err(format!("expected one `{prefix}` line, got {other:?}")),
    }
}

/// Four daemon boots, each a new process on `home_of(boot)`. Returns the
/// four boot lines. Fails on any child failure, on a repeated pid, or when
/// a pid is this process.
pub fn four_boots(
    base: Base<'_>,
    home_of: &dyn Fn(usize) -> PathBuf,
) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let mut pids = vec![std::process::id()];
    let mut stale = String::new();
    for boot in 1..=4 {
        let extra = [(STALE_ENV, stale.clone())];
        let run = run_to_end(command(
            base,
            &format!("boot{boot}"),
            &home_of(boot),
            &extra,
        ))
        .map_err(|e| format!("boot {boot} failed: {e}"))?;
        let line = only_result(&run, &format!("boot {boot} pid={}", run.pid))?;
        if boot == 1 {
            stale = line
                .rsplit_once("stale=")
                .map(|(_, v)| v.to_owned())
                .ok_or_else(|| format!("boot 1 did not report a stale version: {line}"))?;
        }
        if pids.contains(&run.pid) {
            return Err(format!("boot {boot} reused pid {}", run.pid));
        }
        pids.push(run.pid);
        lines.push(line);
    }
    Ok(lines)
}

pub struct Crash {
    pub pid: u32,
    /// Acks read before the kill was sent.
    pub acked_before_kill: u64,
    /// The last ack the child printed at all (read after it died).
    pub last_acked: u64,
    /// Writes found in the database after the crash.
    pub found: u64,
    pub integrity: String,
    pub status: ExitStatus,
}

fn ack(line: &str) -> Option<u64> {
    line.strip_prefix("@acked ")?.parse().ok()
}

/// Starts the crash writer on `home`, kills it (our own child) after
/// [`ACKS_BEFORE_KILL`] acks, then reopens the database in this process.
pub fn crash(base: Base<'_>, home: &Path) -> Result<Crash, String> {
    let deadline = Instant::now() + CHILD_DEADLINE;
    let (mut child, stdout, stderr) = spawn(command(base, "crash", home, &[]))?;
    let pid = child.id();
    let mut acked_before_kill = 0;
    while acked_before_kill < ACKS_BEFORE_KILL {
        let left = deadline.saturating_duration_since(Instant::now());
        match stdout.recv_timeout(left) {
            Ok(line) => acked_before_kill = ack(&line).unwrap_or(acked_before_kill),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let err = drain(&stderr, Instant::now() + Duration::from_secs(1));
                return Err(format!(
                    "crash writer pid={pid} stopped after {acked_before_kill} acks: {}",
                    err.join(" | ")
                ));
            }
        }
    }
    child
        .kill()
        .map_err(|e| format!("kill own child {pid}: {e}"))?;
    let status = child.wait().map_err(|e| e.to_string())?;
    let last_acked = drain(&stdout, deadline)
        .iter()
        .filter_map(|l| ack(l))
        .fold(acked_before_kill, u64::max);

    let store = SqliteStore::open(home, NOW).map_err(|e| format!("reopen after crash: {e}"))?;
    let version = block_on(store.load_task(CRASH_TASK))
        .map_err(|e| e.to_string())?
        .ok_or("the crash task is missing")?
        .version;
    drop(store);
    let conn = rusqlite::Connection::open(home.join(DB_FILE)).map_err(|e| e.to_string())?;
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(Crash {
        pid,
        acked_before_kill,
        last_acked,
        found: version - 1,
        integrity,
        status,
    })
}

/// A second process opening `home` while the caller holds it open. Returns
/// the child's line (`second-open pid=N error=...`).
pub fn second_open(base: Base<'_>, home: &Path) -> Result<String, String> {
    let run = run_to_end(command(base, "second-open", home, &[]))?;
    only_result(&run, "second-open pid=")
}
