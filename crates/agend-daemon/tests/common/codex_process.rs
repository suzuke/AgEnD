//! The real `agend daemon` with real holders running the gate 7 `sh`
//! wrapper, with `fake_codex` (the `codex` CLI stand-in, an example of the
//! `agend` crate) as codex: shared by `crates/agend/tests/codex_process.rs`
//! and `examples/codex_demo.rs` (`#[path]`). Needs the gate 6 lab module as
//! `crate::lab`.
//!
//! Safety: homes under `/tmp/g7-<pid>-<n>`; every daemon is a child of
//! this process (SIGINT / `Child::kill`); a holder is killed with SIGKILL
//! only as a pid (> 1) read from a lock file in the lab's own home while
//! that lock is held; the sweep under test only signals process groups of
//! this lab's own instances (it checks their socket path). Every wait has a
//! deadline.
//!
//! Must NOT: run the real `codex`, or signal anything else.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agend_core::model::Backend;
use agend_daemon::driver::codex::{launch, sweep};
use agend_daemon::runtime::client::Conn;
use agend_daemon::runtime::files;
use agend_daemon::store::{Instance, InstanceStatus, SqliteStore};
use agend_testkit::block_on;
use agend_testkit::fake_agent::codex::Probe;
use serde_json::json;

use crate::lab::{Daemon, Lab, untimed};

fn ensure(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what()) }
}

/// `fake_codex` next to this executable (`target/<profile>/examples/`).
pub fn fake_codex() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no exe dir")?;
    [dir.join("fake_codex"), dir.join("../examples/fake_codex")]
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!(
                "fake_codex not found near {}; run `cargo build -p agend --example fake_codex`",
                exe.display()
            )
        })
}

/// A script in the lab that runs `fake_codex` with `prelude` first.
pub fn codex_script(lab: &Lab, name: &str, prelude: &str) -> Result<PathBuf, String> {
    let fake = fake_codex()?;
    let path = lab.root.join(name);
    let text = format!("#!/bin/sh\n{prelude}\nexec '{}' \"$@\"\n", fake.display());
    fs::write(&path, text).map_err(|e| e.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Removes, on drop, the short socket file a `fake_codex` app-server of
/// `id` binds (a killed one cannot remove it itself).
pub struct BoundSocket(PathBuf);

impl BoundSocket {
    pub fn of(home: &Path, id: &str) -> Self {
        Self(agend_testkit::fake_agent::codex::socket_path_for(
            &launch::socket_path(home, id),
        ))
    }
}

impl Drop for BoundSocket {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Adds codex instance `id` running `program` (turns of `turn_ms`).
pub fn add(home: &Path, id: &str, program: &Path, turn_ms: u64) -> Result<Instance, String> {
    let workdir = home.join("workspace").join(id);
    fs::create_dir_all(&workdir).map_err(|e| e.to_string())?;
    let instance = Instance {
        id: id.into(),
        backend: Backend::Codex,
        program: program.display().to_string(),
        args: vec!["--turn-ms".into(), turn_ms.to_string()],
        working_directory: workdir.display().to_string(),
        session_id: None,
        status: InstanceStatus::New,
        session_started: false,
        agent_pid: None,
        legacy_no_thread: false,
    };
    let store = SqliteStore::open(home, 0).map_err(|e| format!("open store: {e}"))?;
    block_on(store.add_instance(&instance)).map_err(|e| format!("add {id}: {e}"))?;
    Ok(instance)
}

/// The instance as the DB has it (no daemon may run).
pub fn read(home: &Path, id: &str) -> Result<Instance, String> {
    let store = SqliteStore::open(home, 0).map_err(|e| format!("open store: {e}"))?;
    block_on(store.instance(id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no instance {id}"))
}

/// The holder's screen, from one short connection (no daemon may run).
pub fn screen(home: &Path, id: &str) -> Result<String, String> {
    let (_, screen) =
        Conn::connect(&files::socket_path(home, id)).map_err(|e| format!("screen: {e}"))?;
    Ok(screen)
}

/// The line of `screen` starting with `prefix`.
pub fn screen_line(screen: &str, prefix: &str) -> Option<String> {
    screen
        .lines()
        .find(|l| l.trim_start().starts_with(prefix))
        .map(|l| l.trim().to_owned())
}

/// The TUI's `agent args:` line, once it is on the holder's screen (the
/// wrapper polls `$GO` every 0.1 s; no daemon may run).
pub fn tui_args(home: &Path, id: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let shown = screen(home, id)?;
        if let Some(line) = screen_line(&shown, "agent args:") {
            return Ok(line);
        }
        if Instant::now() > deadline {
            return Err(format!("no agent args on the screen of {id}:\n{shown}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The live processes of group `pgid` that belong to instance `id`.
pub fn codex_left(home: &Path, id: &str, pgid: u32, thread: Option<&str>) -> Vec<String> {
    let markers = sweep::Markers {
        socket: launch::socket_path(home, id).display().to_string(),
        thread: thread.map(str::to_owned),
    };
    sweep::group_members(pgid)
        .into_iter()
        .filter_map(|pid| Some((pid, sweep::argv(pid)?)))
        .filter(|(_, argv)| markers.matches(argv))
        .map(|(pid, argv)| format!("{pid} {}", argv.join(" ")))
        .collect()
}

/// Waits until no process of `id` is left in group `pgid`.
pub fn wait_no_codex(home: &Path, id: &str, pgid: u32, thread: Option<&str>) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = codex_left(home, id, pgid, thread);
        if left.is_empty() {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("codex processes left in group {pgid}: {left:?}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// SIGKILL to the holder of `id` (a pid > 1 from its lock, held).
pub fn kill9_holder(home: &Path, id: &str) -> Result<u32, String> {
    let pid = files::running(home, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no holder runs for {id}"))?;
    ensure(pid > 1, || format!("holder pid {pid}"))?;
    // SAFETY: kill(2) on one positive pid > 1 that this lab's own holder
    // wrote into its lock file, whose lock is held.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    let deadline = Instant::now() + Duration::from_secs(10);
    while files::running(home, id).ok().flatten() == Some(pid) {
        ensure(Instant::now() < deadline, || {
            format!("holder {pid} still runs")
        })?;
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(pid)
}

fn value<'a>(line: &'a str, after: &str) -> Option<&'a str> {
    line.split_once(after)
        .map(|(_, rest)| rest.split([' ', ')', ';']).next().unwrap_or_default())
}

/// The daemon's lines about `id`, untimed.
fn lines_of(d: &Daemon, id: &str) -> Vec<String> {
    d.log
        .iter()
        .filter(|l| l.contains(id))
        .map(|l| untimed(l).to_owned())
        .collect()
}

/// Starts a daemon and waits until `id` handed its thread to the TUI.
fn up(lab: &Lab, home: &Path, id: &str) -> Result<(Daemon, String), String> {
    let mut d = Daemon::start(lab, home, &[])?;
    d.ready()?;
    let line = d.expect(&format!("{id}: go (resume "))?;
    let thread = value(&line, "go (resume ").unwrap_or_default().to_owned();
    Ok((d, thread))
}

/// `== resume`: a codex instance's first start (thread created before the
/// TUI, app-server and TUI in one process group), then `kill -9` of its
/// holder: the sweep leaves no codex, the restart resumes the same thread.
pub fn resume(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(1);
    let id = format!("g7-{tag}a");
    add(&home, &id, &fake_codex()?, 300)?;
    let _bound = BoundSocket::of(&home, &id);
    let (mut d, thread) = up(lab, &home, &id)?;
    let mut out: Vec<String> = lines_of(&d, &id)
        .into_iter()
        .filter(|l| {
            [
                ": start",
                "holder pid=",
                "app-server ready",
                "created",
                "go (",
            ]
            .iter()
            .any(|k| l.contains(k))
        })
        .collect();
    d.interrupt()?;
    let instance = read(&home, &id)?;
    ensure(
        instance.session_id.as_deref() == Some(thread.as_str()),
        || format!("thread in the DB: {:?}", instance.session_id),
    )?;
    let pgid = instance.agent_pid.ok_or("no agent_pid recorded")?;
    // The wrapper polls $GO every 0.1 s: wait for the TUI first.
    let args = tui_args(&home, &id)?;
    let members: Vec<String> = sweep::group_members(pgid)
        .into_iter()
        .filter_map(|p| {
            let argv = sweep::argv(p)?;
            let role = if argv.iter().any(|a| a == "app-server") {
                "app-server"
            } else if argv.iter().any(|a| a == "resume") {
                "tui"
            } else {
                return None;
            };
            Some(format!("{role} pid {p}"))
        })
        .collect();
    ensure(members.len() == 2, || format!("group {pgid}: {members:?}"))?;
    out.push(format!(
        "process group {pgid}: {} (the same group)",
        members.join(", ")
    ));
    ensure(
        args.contains(&format!("resume {thread} --remote unix://")),
        || args.clone(),
    )?;
    out.push(format!("TUI screen: {}", args.replace(&thread, "<T>")));
    // kill -9 the holder while a daemon runs.
    let mut d = Daemon::start(lab, &home, &[])?;
    d.ready()?;
    d.expect(&format!("{id}: thread {thread} resumed"))?;
    let holder = kill9_holder(&home, &id)?;
    out.push(format!("holder {id} pid={holder} killed -9 by test"));
    let swept = d.expect(&format!("{id}: sweep of agent group {pgid}"))?;
    d.expect_within(&format!("{id}: restart 1/3"), Duration::from_secs(15))?;
    d.expect_within(
        &format!("{id}: go (resume {thread})"),
        Duration::from_secs(30),
    )?;
    wait_no_codex(&home, &id, pgid, Some(&thread))?;
    out.push(untimed(&swept).to_owned());
    out.push(format!("no fake codex left in group {pgid}"));
    out.extend(
        lines_of(&d, &id)
            .into_iter()
            .filter(|l| {
                l.contains("restart 1/3") || l.contains(&format!("thread {thread} resumed"))
            })
            .skip(1),
    );
    d.interrupt()?;
    let args = tui_args(&home, &id)?;
    ensure(args.contains(&format!("resume {thread} ")), || args.clone())?;
    out.push(format!(
        "new TUI screen: {} (the same thread)",
        args.replace(&thread, "<T>")
    ));
    Ok(out.into_iter().map(|l| l.replace(&thread, "<T>")).collect())
}

/// `== sweep`: app-server and TUI ignore SIGHUP, so killing the holder
/// leaves them running; the daemon's sweep SIGKILLs their group. Then the
/// same with the daemon stopped: the next boot sweeps before it starts the
/// instance again.
pub fn sweep_left_behind(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(2);
    let id = format!("g7-{tag}h");
    let script = codex_script(lab, "fake-codex-nohup", "trap '' HUP")?;
    add(&home, &id, &script, 300)?;
    let _bound = BoundSocket::of(&home, &id);
    let (mut d, thread) = up(lab, &home, &id)?;
    d.interrupt()?;
    let pgid = read(&home, &id)?.agent_pid.ok_or("no agent_pid")?;
    let mut d = Daemon::start(lab, &home, &[])?;
    d.ready()?;
    d.expect(&format!("{id}: thread {thread} resumed"))?;
    let holder = kill9_holder(&home, &id)?;
    let swept = d.expect(&format!("{id}: sweep of agent group {pgid}"))?;
    ensure(swept.contains("SIGKILL sent"), || swept.clone())?;
    wait_no_codex(&home, &id, pgid, Some(&thread))?;
    let mut out = vec![
        format!(
            "codex ignores SIGHUP; holder pid={holder} killed -9 by test while the daemon runs"
        ),
        untimed(&swept).to_owned(),
        format!("no fake codex left in group {pgid}"),
    ];
    d.expect_within(
        &format!("{id}: go (resume {thread})"),
        Duration::from_secs(30),
    )?;
    d.interrupt()?;
    // Daemon stopped: kill -9 the holder, then boot.
    let pgid = read(&home, &id)?.agent_pid.ok_or("no agent_pid")?;
    let holder = kill9_holder(&home, &id)?;
    std::thread::sleep(Duration::from_millis(300));
    let before = codex_left(&home, &id, pgid, Some(&thread)).len();
    let mut d = Daemon::start(lab, &home, &[])?;
    d.ready()?;
    let swept = d.expect(&format!("{id}: sweep of agent group {pgid}"))?;
    ensure(swept.contains("SIGKILL sent"), || swept.clone())?;
    d.expect_within(
        &format!("{id}: go (resume {thread})"),
        Duration::from_secs(30),
    )?;
    wait_no_codex(&home, &id, pgid, Some(&thread))?;
    d.interrupt()?;
    out.push(format!(
        "daemon stopped, holder pid={holder} killed -9 by test: {before} codex process(es) left behind"
    ));
    out.push(format!("next boot: {}", untimed(&swept)));
    out.push(format!(
        "no fake codex left in group {pgid}; thread <T> resumed"
    ));
    Ok(out.into_iter().map(|l| l.replace(&thread, "<T>")).collect())
}

/// `== give-up`: a TUI that dies at once. Three restarts, each resuming the
/// same thread, then `failed`; the app-server followed the TUI every time
/// (it hangs up when the TUI, the session leader, ends): no codex left.
pub fn give_up(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(3);
    let id = format!("g7-{tag}x");
    let script = codex_script(
        lab,
        "fake-codex-tui-dies",
        "for a in \"$@\"; do [ \"$a\" = resume ] && exit 1; done",
    )?;
    add(&home, &id, &script, 300)?;
    let _bound = BoundSocket::of(&home, &id);
    let mut d = Daemon::start(lab, &home, &[])?;
    d.ready()?;
    let failed = d.expect_within(&format!("{id} failed:"), Duration::from_secs(90))?;
    std::thread::sleep(Duration::from_secs(1));
    d.interrupt()?;
    let instance = read(&home, &id)?;
    let thread = instance.session_id.clone().ok_or("no thread")?;
    let pgid = instance.agent_pid.ok_or("no agent_pid")?;
    wait_no_codex(&home, &id, pgid, Some(&thread))?;
    let lines = lines_of(&d, &id);
    let restarts = lines.iter().filter(|l| l.contains(": restart ")).count();
    let resumed = lines
        .iter()
        .filter(|l| l.contains(&format!("thread {thread} resumed")))
        .count();
    let created = lines.iter().filter(|l| l.contains("created")).count();
    ensure(restarts == 3 && resumed == 3 && created == 1, || {
        format!("{lines:#?}")
    })?;
    ensure(instance.status == InstanceStatus::Failed, || {
        format!("{:?}", instance.status)
    })?;
    Ok(vec![
        format!("thread <T> created once, then restart 1/3, 2/3, 3/3 each `thread <T> resumed`"),
        untimed(&failed).to_owned(),
        format!(
            "DB status failed; no fake codex left in group {pgid} (the app-server ended with the TUI)"
        ),
    ])
}

/// `== app-server-dies`: the app-server ends while the TUI keeps running;
/// 20 s later the daemon counts a death, stops the holder and resumes the
/// same thread in a new one.
pub fn app_server_dies(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(4);
    let id = format!("g7-{tag}g");
    add(&home, &id, &fake_codex()?, 300)?;
    let _bound = BoundSocket::of(&home, &id);
    let (mut d, thread) = up(lab, &home, &id)?;
    let old = files::running(&home, &id)
        .map_err(|e| e.to_string())?
        .ok_or("no holder")?;
    let mut probe = Probe::connect(&launch::socket_path(&home, &id))?;
    let _ = probe.send(json!({"id": 1, "method": "agendFake/exit", "params": {}}));
    let started = Instant::now();
    let gone = d.expect_within(
        &format!("{id}: app-server is gone"),
        Duration::from_secs(40),
    )?;
    let waited = started.elapsed();
    d.expect_within(&format!("{id}: restart 1/3"), Duration::from_secs(15))?;
    d.expect_within(
        &format!("{id}: thread {thread} resumed"),
        Duration::from_secs(30),
    )?;
    let fresh = d
        .log
        .iter()
        .rev()
        .find(|l| l.contains(&format!("{id}: holder pid=")))
        .cloned()
        .unwrap_or_default();
    d.interrupt()?;
    let log = fs::read_to_string(files::log_path(&home, &id)).unwrap_or_default();
    ensure(log.contains("shutdown requested"), || {
        format!("the old holder got no Shutdown:\n{log}")
    })?;
    ensure(waited >= Duration::from_secs(19), || {
        format!("gone after {waited:?}")
    })?;
    Ok(vec![
        "fake app-server told to exit; the TUI keeps running".into(),
        format!(
            "{} (after {:.1} s)",
            untimed(&gone).replace(&thread, "<T>"),
            waited.as_secs_f64()
        ),
        format!(
            "old holder pid={old} got Shutdown; {}; thread <T> resumed",
            untimed(&fresh)
        ),
    ])
}

/// `== first-start-interrupted`: the daemon is killed after `Spawned`,
/// before `thread/start` (a slow app-server). The next daemon reconnects to
/// the holder, creates the thread, writes `$GO`, and the waiting wrapper
/// starts the TUI: not `failed`.
pub fn first_start_interrupted(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(5);
    let id = format!("g7-{tag}s");
    let script = codex_script(
        lab,
        "fake-codex-slow",
        "for a in \"$@\"; do [ \"$a\" = app-server ] && sleep 3; done",
    )?;
    add(&home, &id, &script, 300)?;
    let _bound = BoundSocket::of(&home, &id);
    let mut d = Daemon::start(lab, &home, &[])?;
    d.expect(&format!("{id}: holder pid="))?;
    // `running` is written right after; the app-server needs 3 s.
    std::thread::sleep(Duration::from_secs(1));
    d.kill9()?;
    let before = read(&home, &id)?;
    ensure(
        before.status == InstanceStatus::Running && before.session_id.is_none(),
        || format!("{:?} {:?}", before.status, before.session_id),
    )?;
    ensure(!launch::go_path(&home, &id).exists(), || {
        "$GO already there".into()
    })?;
    let (mut d, thread) = up(lab, &home, &id)?;
    let reconnected = d.expect(&format!("{id}: reconnected to holder"))?;
    let created = d.expect(&format!("{id}: thread {thread} created"))?;
    d.interrupt()?;
    let after = read(&home, &id)?;
    let args = tui_args(&home, &id)?;
    ensure(after.status == InstanceStatus::Running, || {
        format!("{:?}", after.status)
    })?;
    ensure(args.contains(&format!("resume {thread} ")), || args.clone())?;
    Ok(vec![
        "daemon killed -9 by test after `holder pid=…`, before the thread existed: DB running, no thread, no $GO".into(),
        format!("next boot: {}", untimed(&reconnected)),
        untimed(&created).replace(&thread, "<T>"),
        format!("the waiting wrapper started the TUI: {}", args.replace(&thread, "<T>")),
        format!("DB status {}", after.status.as_str()),
    ])
}

/// `== legacy`: a schema-v3 `agend.db` with a gate 6 codex row (running,
/// no thread, session started) whose holder runs, and a `new` codex row.
/// After migration 0004 the first is `failed` with `legacy_no_thread = 1`,
/// its holder untouched and no app-server contacted; the second starts.
pub fn legacy(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    use agend_core::traits::HolderLaunch;
    use agend_daemon::runtime::HolderRuntime;
    use agend_daemon::store::MIGRATIONS;
    let home = lab.home(6);
    let old = format!("g7-{tag}o");
    let new = format!("g7-{tag}n");
    drop(SqliteStore::open_with(&home, 0, &MIGRATIONS[..3]).map_err(|e| e.to_string())?);
    let workdir = home.join("workspace").join(&new);
    fs::create_dir_all(&workdir).map_err(|e| e.to_string())?;
    let fake = fake_codex()?;
    let _bound = BoundSocket::of(&home, &new);
    {
        let db = rusqlite::Connection::open(home.join("agend.db")).map_err(|e| e.to_string())?;
        db.execute(
            "INSERT INTO instances (id, backend, program, args, working_directory, session_id, \
             status, session_started) VALUES (?1, 'codex', 'codex', '[]', ?2, NULL, 'running', 1)",
            rusqlite::params![old, home.display().to_string()],
        )
        .map_err(|e| e.to_string())?;
        db.execute(
            "INSERT INTO instances (id, backend, program, args, working_directory, session_id, \
             status, session_started) VALUES (?1, 'codex', ?2, '[\"--turn-ms\",\"300\"]', ?3, NULL, 'new', 0)",
            rusqlite::params![new, fake.display().to_string(), workdir.display().to_string()],
        )
        .map_err(|e| e.to_string())?;
    }
    // Its bare codex TUI of gate 6 (a counter stands in for it).
    let runtime = HolderRuntime::new(&home, &lab.agend, Vec::new(), std::sync::Arc::new(|_| {}));
    let started = block_on(runtime.start(&HolderLaunch {
        instance_id: old.clone(),
        backend: Backend::Codex,
        executable: "/bin/bash".into(),
        args: vec!["-c".into(), crate::lab::COUNTER.into(), "agent".into()],
        working_directory: home.display().to_string(),
    }))
    .map_err(|e| e.to_string())?;
    drop(runtime);
    let holder = started.handle.process_id.unwrap_or(0);
    let mut d = Daemon::start(lab, &home, &[])?;
    d.ready()?;
    let failed = d.expect(&format!("{old} failed:"))?;
    d.expect(&format!("{new}: go (resume "))?;
    d.interrupt()?;
    ensure(
        failed.contains(agend_daemon::supervisor::LEGACY_NO_THREAD),
        || failed.clone(),
    )?;
    ensure(
        files::running(&home, &old).ok().flatten() == Some(holder),
        || format!("the old holder {holder} is not the one running"),
    )?;
    let touched = lines_of(&d, &old)
        .into_iter()
        .filter(|l| {
            ["app-server", "created", "resumed", "sweep", "go ("]
                .iter()
                .any(|k| l.contains(k))
        })
        .collect::<Vec<_>>();
    ensure(touched.is_empty(), || format!("{touched:?}"))?;
    let o = read(&home, &old)?;
    let n = read(&home, &new)?;
    ensure(
        o.legacy_no_thread && o.status == InstanceStatus::Failed,
        || format!("{o:?}"),
    )?;
    ensure(!n.legacy_no_thread && n.session_id.is_some(), || {
        format!("{n:?}")
    })?;
    Ok(vec![
        format!("schema v3: {old} codex running, no thread, holder pid={holder}; {new} codex new"),
        untimed(&failed).to_owned(),
        format!(
            "{old}: legacy_no_thread=1, holder pid={holder} still runs, no app-server or thread/start"
        ),
        format!(
            "{new}: legacy_no_thread=0, thread created, status {}",
            n.status.as_str()
        ),
    ])
}
