//! Real `agend daemon` processes against real holders (gate 6 P1–P6),
//! shared by `crates/agend/tests/daemon_process.rs` (which has the built
//! `agend`) and `examples/daemon_probe.rs demo` (`#[path]`), so the demo
//! prints what the tests check.
//!
//! Each section returns the lines to print, or an error naming what broke.
//!
//! Safety: every home is a fresh directory under `/tmp` (short enough for
//! holder socket paths), removed at the end. Every daemon is a child of this
//! process: it gets SIGINT (Ctrl-C) or `Child::kill()` from here, nothing
//! else. Holders are stopped with the protocol's `Shutdown`; the only
//! fallback is SIGKILL of a pid (> 1) read from a lock file in this lab's own
//! home, while that lock is still held. Every wait has a deadline.
//!
//! Must NOT: signal a process it did not start or read from its own lock
//! files, or use a home outside its own directory.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use agend_core::model::Backend;
use agend_daemon::runtime::{files, shutdown_holder};
use agend_daemon::store::instances::new_session_id;
use agend_daemon::store::{Instance, InstanceStatus, SqliteStore};
use agend_testkit::block_on;

/// Counts once a second; prints the session arguments it got first.
pub const COUNTER: &str =
    "echo \"agent args: $*\"; i=0; while :; do i=$((i+1)); echo \"counter=$i\"; sleep 1; done";
/// Records its arguments in its working directory and dies at once.
pub const DIES_AT_ONCE: &str = "echo \"$*\" >> args.log; exit 1";
/// Writes its environment to its working directory, then waits.
pub const WRITES_ENV: &str = "env > agent-env.txt; exec sleep 600";

/// Longest a daemon may take to print an expected line.
const LINE_WITHIN: Duration = Duration::from_secs(30);

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A directory of daemon homes and the `agend` binary. Dropping it stops
/// every holder under it, kills daemons still running, and removes it.
pub struct Lab {
    pub root: PathBuf,
    pub agend: PathBuf,
}

impl Lab {
    pub fn new(agend: &Path) -> Self {
        let root = PathBuf::from(format!(
            "/tmp/g6-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .expect("create lab dir");
        Self {
            root,
            agend: agend.to_path_buf(),
        }
    }

    /// Home `n` of this lab (created on first use).
    pub fn home(&self, n: usize) -> PathBuf {
        let home = self.root.join(format!("h{n}"));
        let _ = fs::DirBuilder::new().mode(0o700).create(&home);
        home
    }

    /// Every running holder in every home of this lab.
    pub fn running_holders(&self) -> Vec<(PathBuf, String, u32)> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.root).into_iter().flatten().flatten() {
            let home = entry.path();
            for (id, pid) in files::running_holders(&home).unwrap_or_default() {
                out.push((home.clone(), id, pid));
            }
        }
        out
    }

    /// Stops every holder of this lab; returns how many were running.
    pub fn stop_all_holders(&self) -> usize {
        let running = self.running_holders();
        for (home, id, pid) in &running {
            if shutdown_holder(home, id).is_err()
                && matches!(files::running(home, id), Ok(Some(p)) if p == *pid)
                && *pid > 1
            {
                // SAFETY: kill(2) on one positive pid > 1 that this lab's
                // own holder wrote into its lock file, whose lock is held.
                unsafe { libc::kill(*pid as libc::pid_t, libc::SIGKILL) };
            }
        }
        running.len()
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        self.stop_all_holders();
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Adds an instance to `home`'s DB (the daemon must not be running).
pub fn add(home: &Path, id: &str, script: &str) -> Result<Instance, String> {
    let workdir = home.join("workspace").join(id);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&workdir)
        .map_err(|e| format!("create {}: {e}", workdir.display()))?;
    let instance = Instance {
        id: id.into(),
        backend: Backend::Claude,
        program: "/bin/bash".into(),
        args: vec!["-c".into(), script.into(), "agent".into()],
        working_directory: workdir.display().to_string(),
        session_id: Some(new_session_id().map_err(|e| e.to_string())?),
        status: InstanceStatus::New,
    };
    let store = SqliteStore::open(home, 0).map_err(|e| format!("open store: {e}"))?;
    block_on(store.add_instance(&instance)).map_err(|e| format!("add {id}: {e}"))?;
    Ok(instance)
}

pub fn remove(home: &Path, id: &str) -> Result<(), String> {
    let store = SqliteStore::open(home, 0).map_err(|e| format!("open store: {e}"))?;
    match block_on(store.remove_instance(id)) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("no instance {id}")),
        Err(e) => Err(e.to_string()),
    }
}

pub fn status(home: &Path, id: &str) -> Result<InstanceStatus, String> {
    let store = SqliteStore::open(home, 0).map_err(|e| format!("open store: {e}"))?;
    block_on(store.instance(id))
        .map_err(|e| e.to_string())?
        .map(|i| i.status)
        .ok_or_else(|| format!("no instance {id}"))
}

/// One `agend daemon` child with its stderr (the log) read line by line.
pub struct Daemon {
    child: Child,
    pub pid: u32,
    lines: Receiver<String>,
    /// Every line read so far.
    pub log: Vec<String>,
}

impl Daemon {
    pub fn start(lab: &Lab, home: &Path, extra_env: &[(&str, &str)]) -> Result<Self, String> {
        let mut cmd = Command::new(&lab.agend);
        cmd.arg("daemon")
            .env_clear()
            .env("AGEND_HOME", home)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .current_dir(home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for key in ["HOME", "USER", "LANG", "TMPDIR"] {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("start agend daemon: {e}"))?;
        let stderr = child.stderr.take().expect("piped");
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let pid = child.id();
        Ok(Self {
            child,
            pid,
            lines,
            log: Vec::new(),
        })
    }

    /// The first log line containing `needle`, read already or waited for
    /// (every line read is kept in `log`).
    pub fn expect(&mut self, needle: &str) -> Result<String, String> {
        self.expect_within(needle, LINE_WITHIN)
    }

    pub fn expect_within(&mut self, needle: &str, within: Duration) -> Result<String, String> {
        if let Some(line) = self.log.iter().find(|l| l.contains(needle)) {
            return Ok(line.clone());
        }
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    self.log.push(line.clone());
                    if line.contains(needle) {
                        return Ok(line);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "daemon pid={} printed no {needle:?} within {within:?}; log:\n{}",
                        self.pid,
                        self.log.join("\n")
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!(
                        "daemon pid={} ended before printing {needle:?}; log:\n{}",
                        self.pid,
                        self.log.join("\n")
                    ));
                }
            }
        }
    }

    /// Waits for the ready line and returns its counts.
    pub fn ready(&mut self) -> Result<BTreeMap<String, u64>, String> {
        let line = self.expect("agend daemon ready:")?;
        Ok(counts(&line))
    }

    /// The ready line without its timestamp.
    pub fn ready_line(&self) -> String {
        self.log
            .iter()
            .find(|l| l.contains("agend daemon ready:"))
            .map(|l| untimed(l).to_owned())
            .unwrap_or_default()
    }

    /// Ctrl-C: SIGINT to this child, then waits (at most 10 s) for it to end.
    /// Returns how long it took.
    pub fn interrupt(&mut self) -> Result<Duration, String> {
        let started = Instant::now();
        assert!(self.pid > 1);
        if self.child.try_wait().map_err(|e| e.to_string())?.is_none() {
            // SAFETY: kill(2) on our own child (pid > 1), not yet reaped.
            unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGINT) };
        }
        let status = self.wait(Duration::from_secs(10))?;
        if !status.success() {
            return Err(format!("daemon pid={} exited with {status}", self.pid));
        }
        Ok(started.elapsed())
    }

    /// `kill -9` of this child (`Child::kill`), then reaps it.
    pub fn kill9(&mut self) -> Result<ExitStatus, String> {
        self.child.kill().map_err(|e| e.to_string())?;
        self.wait(Duration::from_secs(10))
    }

    /// Waits for the child to end; kills it after `limit`.
    pub fn wait(&mut self, limit: Duration) -> Result<ExitStatus, String> {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                // The reader thread ends at EOF; take what is left.
                while let Ok(line) = self.lines.recv_timeout(Duration::from_millis(500)) {
                    self.log.push(line);
                }
                return Ok(status);
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Err(format!(
                    "daemon pid={} did not end within {limit:?}; killed",
                    self.pid
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// `key=value` numbers of a log line.
pub fn counts(line: &str) -> BTreeMap<String, u64> {
    line.split_whitespace()
        .filter_map(|f| f.split_once('='))
        .filter_map(|(k, v)| Some((k.to_owned(), v.trim_end_matches([';', ',']).parse().ok()?)))
        .collect()
}

/// A log line without its leading timestamp.
pub fn untimed(line: &str) -> &str {
    line.split_once(' ').map_or(line, |x| x.1)
}

fn field(line: &str, key: &str) -> Result<u64, String> {
    counts(line)
        .get(key)
        .copied()
        .ok_or_else(|| format!("no {key}= in {line:?}"))
}

fn ensure(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what()) }
}

/// The counter on a reconnect line (`... screen: counter=N`).
fn counter(line: &str) -> Result<u64, String> {
    field(line, "counter")
}

/// The holder log of `id` has no `Shutdown` in it.
fn no_shutdown_received(home: &Path, id: &str) -> Result<(), String> {
    let log = fs::read_to_string(files::log_path(home, id)).unwrap_or_default();
    ensure(!log.contains("shutdown requested"), || {
        format!("holder {id} received Shutdown:\n{log}")
    })
}

/// `== restart`: four daemon boots (work, idle, work, check), boot 3
/// killed with SIGKILL. Holders keep their pid and their counter grows.
/// `home_of(boot)` is the home of each boot: the same one, or (negative
/// check) a new one each boot, which must fail at boot 2.
pub fn restart(
    lab: &Lab,
    tag: &str,
    home_of: &dyn Fn(usize) -> PathBuf,
) -> Result<Vec<String>, String> {
    let first = format!("g6-{tag}a");
    let second = format!("g6-{tag}b");
    let mut out = Vec::new();
    let mut daemon_pids = Vec::new();

    // Boot 1 (work): the instance is started.
    let home = home_of(1);
    add(&home, &first, COUNTER)?;
    let mut d = Daemon::start(lab, &home, &[])?;
    let ready = d.ready()?;
    ensure(ready.get("started") == Some(&1), || {
        format!("boot 1: {ready:?}")
    })?;
    let line = d.expect(&format!("{first}: holder pid="))?;
    let holder = field(&line, "pid")?;
    let waited = d
        .log
        .iter()
        .find_map(|l| l.contains("agend.db opened").then(|| l.clone()));
    let took = d.interrupt()?;
    ensure(took < Duration::from_secs(5), || {
        format!("boot 1: Ctrl-C took {took:?}")
    })?;
    ensure(
        files::running(&home, &first).ok().flatten() == Some(holder as u32),
        || format!("boot 1: holder {holder} of {first} did not survive Ctrl-C"),
    )?;
    no_shutdown_received(&home, &first)?;
    daemon_pids.push(d.pid);
    out.push(format!(
        "boot 1 daemon pid={} holder pid={holder} (work: started {first}); Ctrl-C: exited in {} ms, holder still runs, no Shutdown",
        d.pid,
        took.as_millis()
    ));
    if let Some(waited) = waited {
        out.push(format!("  {}", waited.split_once(' ').map_or("", |x| x.1)));
    }
    std::thread::sleep(Duration::from_millis(1500));

    // Boots 2-4.
    let mut last_counter = 0;
    let mut second_holder = 0;
    for boot in 2..=4 {
        let home = home_of(boot);
        if boot == 3 {
            // Work while no daemon runs: one more instance for boot 3 to start.
            add(&home, &second, COUNTER)?;
        }
        let mut d = Daemon::start(lab, &home, &[])?;
        let ready = d.ready().map_err(|e| format!("boot {boot}: {e}"))?;
        let recovered = ready.get("recovered").copied().unwrap_or(0);
        let expected = if boot == 4 { 2 } else { 1 };
        ensure(recovered == expected, || {
            format!(
                "boot {boot} failed: daemon pid={} recovered {recovered} holder(s), expected {expected} ({})",
                d.pid,
                d.ready_line()
            )
        })?;
        let line = d
            .log
            .iter()
            .find(|l| l.contains(&format!("{first}: reconnected to holder pid=")))
            .cloned()
            .ok_or_else(|| format!("boot {boot}: no reconnect line for {first}"))?;
        let pid = field(&line, "pid")?;
        let now = counter(&line)?;
        ensure(pid == holder, || {
            format!("boot {boot}: holder pid {pid}, expected {holder}")
        })?;
        ensure(now > last_counter, || {
            format!("boot {boot}: counter {now} did not grow from {last_counter}")
        })?;
        last_counter = now;
        daemon_pids.push(d.pid);
        match boot {
            2 => {
                d.interrupt()?;
                out.push(format!(
                    "boot 2 daemon pid={} (idle) holder pid={pid} counter={now}",
                    d.pid
                ));
            }
            3 => {
                let line = d.expect(&format!("{second}: holder pid="))?;
                second_holder = field(&line, "pid")?;
                let status = d.kill9()?;
                out.push(format!(
                    "boot 3 daemon pid={} holder pid={pid} counter={now} (work: started {second} pid={second_holder}); killed -9 by test ({status})",
                    d.pid
                ));
            }
            _ => {
                let line = d
                    .log
                    .iter()
                    .find(|l| l.contains(&format!("{second}: reconnected to holder pid=")))
                    .cloned()
                    .ok_or_else(|| format!("boot 4: no reconnect line for {second}"))?;
                ensure(field(&line, "pid")? == second_holder, || {
                    format!("boot 4: {second} has a new holder: {line}")
                })?;
                let waited = d
                    .log
                    .iter()
                    .find(|l| l.contains("agend.db opened"))
                    .cloned()
                    .unwrap_or_default();
                d.interrupt()?;
                no_shutdown_received(&home, &first)?;
                out.push(format!(
                    "boot 4 daemon pid={} holder pid={pid} counter={now} ok ({second} pid={second_holder} too)",
                    d.pid
                ));
                out.push(format!(
                    "  after the kill -9: {}",
                    waited.split_once(' ').map_or("", |x| x.1)
                ));
            }
        }
        std::thread::sleep(Duration::from_millis(1500));
    }
    let mut unique = daemon_pids.clone();
    unique.sort();
    unique.dedup();
    ensure(unique.len() == 4, || {
        format!("daemon pids not all different: {daemon_pids:?}")
    })?;
    out.push(format!(
        "4 boots, 4 daemon pids, the same holder pid={holder}, counter kept growing"
    ));
    Ok(out)
}

/// `== give-up`: an agent that dies at once. Three restarts with
/// `--resume` (5 s apart), then `failed`, then nothing more; the agent's
/// own record shows it was never started fresh again.
pub fn give_up(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(90);
    let id = format!("g6-{tag}x");
    let instance = add(&home, &id, DIES_AT_ONCE)?;
    let session = instance.session_id.clone().unwrap_or_default();
    let mut d = Daemon::start(lab, &home, &[])?;
    d.ready()?;
    let failed = d.expect_within(&format!("{id} failed:"), Duration::from_secs(90))?;
    // Longer than a restart delay: no 4th restart may follow.
    std::thread::sleep(Duration::from_secs(7));
    d.interrupt()?;
    let mut out: Vec<String> = d
        .log
        .iter()
        .filter(|l| {
            l.contains(&format!("{id}: start"))
                || l.contains(&format!("{id}: restart"))
                || l.contains(&format!("agent {id} exited"))
                || l.contains(&format!("{id} failed"))
        })
        .map(|l| l.split_once(' ').map_or(l.as_str(), |x| x.1).to_owned())
        .collect();
    let restarts: Vec<&String> = out.iter().filter(|l| l.contains(": restart ")).collect();
    ensure(restarts.len() == 3, || {
        format!("expected 3 restarts: {out:#?}")
    })?;
    for (n, line) in restarts.iter().enumerate() {
        ensure(
            line.contains(&format!("restart {}/3 --resume {session}", n + 1)),
            || format!("restart {} without --resume: {line}", n + 1),
        )?;
    }
    ensure(failed.contains("failed:"), || failed.clone())?;
    let args = fs::read_to_string(Path::new(&instance.working_directory).join("args.log"))
        .map_err(|e| format!("args.log: {e}"))?;
    let args: Vec<&str> = args.lines().collect();
    let expected: Vec<String> = std::iter::once(format!("--session-id {session}"))
        .chain((0..3).map(|_| format!("--resume {session}")))
        .collect();
    ensure(args == expected, || {
        format!("the agent was started with {args:?}")
    })?;
    let status = status(&home, &id)?;
    ensure(status == InstanceStatus::Failed, || {
        format!("status {status:?}")
    })?;
    out.push(format!(
        "agent's own record of its starts: {} (1 first start, then only --resume)",
        args.join(" | ")
    ));
    out.push(format!(
        "DB status: {}; no restart 4/3 in the 7 s after",
        status.as_str()
    ));
    Ok(out)
}

/// `== env`: the agent's environment is the whitelist; the daemon's secret
/// and `AGEND_SHIM_BYPASS` are not in it; the shims come first on `PATH`.
pub fn env(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(91);
    let id = format!("g6-{tag}e");
    let instance = add(&home, &id, WRITES_ENV)?;
    let mut d = Daemon::start(
        lab,
        &home,
        &[
            ("TELEGRAM_BOT_TOKEN", "demo-secret"),
            ("AGEND_SHIM_BYPASS", "1"),
        ],
    )?;
    d.ready()?;
    let file = Path::new(&instance.working_directory).join("agent-env.txt");
    let deadline = Instant::now() + Duration::from_secs(10);
    let text = loop {
        match fs::read_to_string(&file) {
            Ok(text) if text.contains("PATH=") => break text,
            _ if Instant::now() > deadline => return Err("the agent wrote no env".into()),
            _ => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    d.interrupt()?;
    let env: BTreeMap<&str, &str> = text.lines().filter_map(|l| l.split_once('=')).collect();
    let bin = home.join("bin");
    ensure(!env.contains_key("TELEGRAM_BOT_TOKEN"), || text.clone())?;
    ensure(!env.contains_key("AGEND_SHIM_BYPASS"), || text.clone())?;
    ensure(env.get("AGEND_INSTANCE") == Some(&id.as_str()), || {
        text.clone()
    })?;
    ensure(
        env.get("PATH")
            .is_some_and(|p| p.starts_with(&format!("{}:", bin.display()))),
        || text.clone(),
    )?;
    let git = fs::read_link(bin.join("git")).map_err(|e| format!("bin/git: {e}"))?;
    let agend = fs::canonicalize(&lab.agend).map_err(|e| e.to_string())?;
    ensure(fs::canonicalize(&git).ok() == Some(agend), || {
        format!("bin/git -> {}", git.display())
    })?;
    let mut names: Vec<&str> = env.keys().copied().collect();
    names.sort();
    Ok(vec![
        "daemon env had TELEGRAM_BOT_TOKEN and AGEND_SHIM_BYPASS=1".into(),
        format!("agent env names: {}", names.join(" ")),
        "TELEGRAM_BOT_TOKEN: absent; AGEND_SHIM_BYPASS: absent".into(),
        format!(
            "PATH={}:… (the shims first)",
            env["PATH"].split(':').next().unwrap_or_default()
        ),
        format!("bin/git -> {}", git.display()),
    ])
}

/// `== second-daemon`: a second daemon on the same home is refused after
/// its 10 s of retries; the first one is untouched. Then the real handoff: a
/// new daemon started while the old one is stopping waits for the lock.
pub fn second_daemon(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(92);
    let id = format!("g6-{tag}s");
    add(&home, &id, COUNTER)?;
    let mut first = Daemon::start(lab, &home, &[])?;
    first.ready()?;
    first.expect(&format!("{id}: holder pid="))?;
    let logs = home.join("logs");
    let snapshot_logs = || -> BTreeMap<String, Vec<u8>> {
        fs::read_dir(&logs)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    fs::read(e.path()).unwrap_or_default(),
                )
            })
            .collect()
    };
    let before = snapshot_logs();
    let started = Instant::now();
    let mut second = Daemon::start(lab, &home, &[])?;
    let status = second.wait(Duration::from_secs(30))?;
    let took = started.elapsed();
    let refusal = second.log.join("\n");
    ensure(status.code() == Some(1), || {
        format!("second daemon: {status}: {refusal}")
    })?;
    ensure(
        refusal.contains("agend.db is in use by another process"),
        || refusal.clone(),
    )?;
    ensure(took >= Duration::from_secs(10), || {
        format!("refused after only {took:?}")
    })?;
    ensure(snapshot_logs() == before, || {
        "the first daemon's log changed".into()
    })?;
    ensure(
        first.child.try_wait().map_err(|e| e.to_string())?.is_none(),
        || "the first daemon stopped".into(),
    )?;
    let mut out = vec![
        format!(
            "second daemon pid={} exit=1 after {:.1} s: {}",
            second.pid,
            took.as_secs_f64(),
            refusal.trim()
        ),
        format!(
            "first daemon pid={} still runs; its log unchanged",
            first.pid
        ),
    ];
    // Handoff: start the next daemon, then stop the first.
    let mut next = Daemon::start(lab, &home, &[])?;
    std::thread::sleep(Duration::from_millis(300));
    first.interrupt()?;
    let opened = next.expect("agend.db opened")?;
    let ready = next.ready()?;
    ensure(ready.get("recovered") == Some(&1), || format!("{ready:?}"))?;
    next.interrupt()?;
    out.push(format!(
        "handoff: new daemon pid={} started before the old one stopped: {}",
        next.pid,
        opened.split_once(' ').map_or("", |x| x.1)
    ));
    Ok(out)
}

/// `== orphan`: an instance removed from the DB while the daemon is down;
/// the next boot stops its holder.
pub fn orphan(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(93);
    let id = format!("g6-{tag}o");
    add(&home, &id, COUNTER)?;
    let mut d = Daemon::start(lab, &home, &[])?;
    d.ready()?;
    let line = d.expect(&format!("{id}: holder pid="))?;
    let holder = field(&line, "pid")?;
    d.interrupt()?;
    remove(&home, &id)?;
    let mut d = Daemon::start(lab, &home, &[])?;
    let ready = d.ready()?;
    ensure(ready.get("orphans") == Some(&1), || format!("{ready:?}"))?;
    let sent = d
        .log
        .iter()
        .find(|l| l.contains(&format!("orphan {id}: Shutdown sent")))
        .cloned()
        .ok_or_else(|| format!("no orphan line: {:#?}", d.log))?;
    d.interrupt()?;
    let running = files::running(&home, &id).map_err(|e| e.to_string())?;
    ensure(running.is_none(), || {
        format!("orphan holder still runs: {running:?}")
    })?;
    Ok(vec![
        format!(
            "holder pid={holder} of {id}; {id} removed from the DB while the daemon was stopped"
        ),
        format!("next boot: {}", d.ready_line()),
        sent.split_once(' ').map_or("", |x| x.1).to_owned(),
        format!("holder pid={holder} gone"),
    ])
}

/// Records its arguments in its working directory, then counts.
pub const RECORDS_ARGS: &str =
    "echo \"$*\" >> args.log; i=0; while :; do i=$((i+1)); echo \"counter=$i\"; sleep 1; done";

/// `== crash-before-spawn` (verifier r1 F1): the daemon is killed right
/// after it logs the first start, before the agent ran. The next boot must
/// still start the agent with `--session-id` (the session was never
/// created), never `--resume`. Retried with a new instance (up to 5 times)
/// when the kill came too late to hit the window.
pub fn crash_before_spawn(lab: &Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(94);
    for attempt in 1..=5 {
        let id = format!("g6-{tag}c{attempt}");
        let instance = add(&home, &id, RECORDS_ARGS)?;
        let session = instance.session_id.clone().unwrap_or_default();
        let args_log = Path::new(&instance.working_directory).join("args.log");
        let mut d = Daemon::start(lab, &home, &[])?;
        d.expect(&format!("{id}: start --session-id {session}"))?;
        d.kill9()?;
        if args_log.exists() {
            // The agent already ran: not the window this checks.
            remove(&home, &id)?;
            lab.stop_all_holders();
            continue;
        }
        let holder = files::running(&home, &id).ok().flatten();
        let mut d = Daemon::start(lab, &home, &[])?;
        d.ready()?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let args = loop {
            match fs::read_to_string(&args_log) {
                Ok(text) if !text.is_empty() => break text,
                _ if Instant::now() > deadline => {
                    return Err(format!(
                        "the agent never started; log:\n{}",
                        d.log.join("\n")
                    ));
                }
                _ => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        d.interrupt()?;
        let first = args.lines().next().unwrap_or_default().to_owned();
        ensure(first == format!("--session-id {session}"), || {
            format!(
                "after the crash the agent was started with {args:?}, expected --session-id {session}"
            )
        })?;
        ensure(!args.contains("--resume"), || {
            format!("resumed a session never created: {args:?}")
        })?;
        let status = status(&home, &id)?;
        ensure(status == InstanceStatus::Running, || {
            format!("status {status:?}")
        })?;
        let after = d
            .log
            .iter()
            .find(|l| l.contains(&format!("{id}:")))
            .map(|l| untimed(l).to_owned())
            .unwrap_or_default();
        return Ok(vec![
            format!(
                "attempt {attempt}: daemon killed -9 right after `{id}: start --session-id <S>`; agent had not run; holder {}",
                holder.map_or("not started".into(), |p| format!(
                    "pid={p} left without an agent"
                ))
            ),
            format!("next boot: {}", after.replace(&session, "<S>")),
            format!(
                "agent's own record: {} (the session is created, not resumed)",
                first.replace(&session, "<S>")
            ),
            format!("DB status: {}", status.as_str()),
        ]);
    }
    Err("the kill never landed before the agent started (5 attempts)".into())
}
