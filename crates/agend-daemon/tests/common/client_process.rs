//! The client protocol against the real `agend daemon` (gate 8), shared by
//! `crates/agend/tests/client_protocol.rs` (which has the built `agend`) and
//! `examples/client_demo.rs` (`#[path]`), so the demo prints what the tests
//! check. Needs the gate 6 lab module as `crate::lab`.
//!
//! - [`RealDaemon`]: the CLP fixture over a real daemon in its own home. Its
//!   `emit` resolves a `failed` instance with `retry` (a real event).
//! - [`InProcess`]: the daemon's own server code (`server` + `fleet`) in this
//!   process, for CLP-8, which needs 2000 events.
//! - Sections for the demo and the tests; each returns the lines to print.
//!
//! Safety: homes under `/tmp/g8-<pid>-<n>` (the lab's); every daemon is a
//! child of this process (SIGINT / `Child::kill`); holders are stopped with
//! `Shutdown` by the lab; children (`agend debug …`) are waited with a
//! deadline and killed (`Child::kill`) only if they overrun.
//!
//! Must NOT: signal anything else.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agend_core::model::Backend;
use agend_core::protocol::client::{
    AttentionAction, ClientRequest, ClientResponse, CommandResult, DAEMON_SOCKET, DaemonEvent,
    InstanceData, RequestIdData, ResolveAttentionData, TaskChangedData, error_code,
};
use agend_daemon::fleet::Fleet;
use agend_daemon::handlers::Context;
use agend_daemon::runtime::{HolderRuntime, files};
use agend_daemon::server::{self, Server};
use agend_daemon::store::instances::new_session_id;
use agend_daemon::store::{Instance, InstanceStatus, SqliteStore};
use agend_testkit::block_on;
use agend_testkit::contract::client::ClientProtocolFixture;
use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};

use crate::lab::{self, Daemon, Lab};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A short tag unique in this process, for instance ids (24 bytes max).
pub fn tag() -> String {
    format!(
        "{}{}",
        std::process::id() % 10_000,
        NEXT.fetch_add(1, Ordering::SeqCst)
    )
}

fn ensure(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what()) }
}

pub fn socket_of(home: &Path) -> PathBuf {
    home.join(DAEMON_SOCKET)
}

/// Adds an instance with `backend` (codex and opencode get no session id).
pub fn add(home: &Path, id: &str, backend: Backend, script: &str) -> Result<Instance, String> {
    let workdir = home.join("workspace").join(id);
    fs::create_dir_all(&workdir).map_err(|e| format!("{}: {e}", workdir.display()))?;
    let instance = Instance {
        id: id.into(),
        backend,
        program: "/bin/bash".into(),
        args: vec!["-c".into(), script.into(), "agent".into()],
        working_directory: workdir.display().to_string(),
        session_id: match backend {
            Backend::Claude => Some(new_session_id().map_err(|e| e.to_string())?),
            _ => None,
        },
        status: InstanceStatus::New,
        session_started: false,
    };
    let store = SqliteStore::open(home, 0).map_err(|e| format!("open store: {e}"))?;
    block_on(store.add_instance(&instance)).map_err(|e| format!("add {id}: {e}"))?;
    Ok(instance)
}

/// Writes `status` of `id` (the daemon must not be running).
pub fn set_status(home: &Path, id: &str, status: InstanceStatus) -> Result<(), String> {
    let store = SqliteStore::open(home, 0).map_err(|e| format!("open store: {e}"))?;
    block_on(store.set_instance_status(id, status)).map_err(|e| e.to_string())
}

/// `resolve_attention` as the operator; `Ok` when accepted, else the reply.
pub fn resolve(socket: &Path, attention_id: &str) -> Result<(), String> {
    let (mut c, _) = ProbeClient::hello(socket, None).map_err(|e| format!("hello: {e}"))?;
    let reply = c
        .request(&ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: "g8-resolve".into(),
                attention_id: attention_id.into(),
                action: AttentionAction::Retry,
            },
        })
        .map_err(|e| format!("resolve: {e}"))?;
    match reply {
        ClientResponse::CommandResult { data } if data.result == CommandResult::Accepted => Ok(()),
        other => Err(format!("resolve {attention_id}: {other:?}")),
    }
}

/// The CLP fixture over a real `agend daemon`: one running counter instance
/// (the terminal) and [`RealDaemon::FAILED`] `failed` claude instances that
/// never ran (each can be retried once).
pub struct RealDaemon {
    pub lab: Lab,
    pub home: PathBuf,
    pub daemon: Option<Daemon>,
    running: String,
    failed: Vec<String>,
}

impl RealDaemon {
    pub const FAILED: usize = 6;

    pub fn start(agend: &Path) -> Result<RealDaemon, String> {
        let lab = Lab::with_prefix(agend, "g8");
        let home = lab.home(1);
        let tag = tag();
        let running = format!("g8-{tag}r");
        add(&home, &running, Backend::Claude, lab::COUNTER)?;
        let mut failed = Vec::new();
        for n in 1..=Self::FAILED {
            let id = format!("g8-{tag}f{n}");
            add(&home, &id, Backend::Claude, lab::COUNTER)?;
            set_status(&home, &id, InstanceStatus::Failed)?;
            failed.push(id);
        }
        let mut daemon = Daemon::start(&lab, &home, &[])?;
        daemon.ready()?;
        Ok(RealDaemon {
            lab,
            home,
            daemon: Some(daemon),
            running,
            failed,
        })
    }

    /// For contract runs: panics if the daemon cannot start.
    pub fn fixture(agend: &Path) -> RealDaemon {
        RealDaemon::start(agend).unwrap_or_else(|e| panic!("real daemon: {e}"))
    }
}

impl Drop for RealDaemon {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.interrupt();
        }
        // The lab stops the holders with Shutdown and removes the homes.
    }
}

impl ClientProtocolFixture for RealDaemon {
    fn socket(&self) -> PathBuf {
        socket_of(&self.home)
    }

    fn emit(&mut self) -> Result<(), String> {
        let item = self.retry_item()?;
        resolve(&self.socket(), &item)
    }

    fn burst(&mut self, _: usize) -> Result<(), String> {
        Err("the agend daemon binary cannot make 2000 real events (see InProcess)".into())
    }

    fn restart(&mut self) -> Result<(), String> {
        if let Some(mut daemon) = self.daemon.take() {
            daemon.interrupt()?;
        }
        let mut daemon = Daemon::start(&self.lab, &self.home, &[])?;
        daemon.ready()?;
        self.daemon = Some(daemon);
        Ok(())
    }

    fn retry_item(&mut self) -> Result<String, String> {
        let id = self
            .failed
            .pop()
            .ok_or("no failed instance left to retry")?;
        Ok(format!("instance-failed:{id}"))
    }

    fn terminal_instance(&self) -> String {
        self.running.clone()
    }
}

/// The daemon's server and fleet in this process, fed events directly: the
/// same code as `agend daemon`, for CLP-8 (2000 events).
pub struct InProcess {
    runtime: tokio::runtime::Runtime,
    fleet: Arc<Fleet>,
    server: Option<Server>,
    root: PathBuf,
    socket: PathBuf,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl InProcess {
    pub fn start() -> InProcess {
        let root = PathBuf::from(format!(
            "/tmp/g8-{}-ip{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(root.join("run")).expect("in-process home");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let socket = socket_of(&root);
        let mut fx = InProcess {
            runtime,
            fleet: Arc::new(Fleet::new(now_ms())),
            server: None,
            root,
            socket,
        };
        fx.serve();
        fx
    }

    fn serve(&mut self) {
        let _entered = self.runtime.enter();
        // No supervisor: its queue is closed (`resolve_attention` answers
        // `not_supported`), and no holders.
        let (supervisor, _) = tokio::sync::mpsc::unbounded_channel();
        let context = Arc::new(Context {
            fleet: Arc::clone(&self.fleet),
            runtime: HolderRuntime::new(
                &self.root,
                Path::new("/nonexistent/agend"),
                Vec::new(),
                Arc::new(|_| {}),
            ),
            supervisor,
        });
        let listener = server::bind(&self.socket).expect("bind");
        self.server = Some(Server::start(listener, self.socket.clone(), context));
    }
}

impl Drop for InProcess {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            self.runtime.block_on(server.stop());
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl ClientProtocolFixture for InProcess {
    fn socket(&self) -> PathBuf {
        self.socket.clone()
    }

    fn emit(&mut self) -> Result<(), String> {
        self.burst(1)
    }

    fn burst(&mut self, n: usize) -> Result<(), String> {
        for _ in 0..n {
            self.fleet.publish(DaemonEvent::TaskChanged {
                data: TaskChangedData {
                    task_id: "T-g8".into(),
                    summary: "burst".into(),
                    task: None,
                },
            });
        }
        Ok(())
    }

    fn restart(&mut self) -> Result<(), String> {
        if let Some(server) = self.server.take() {
            self.runtime.block_on(server.stop());
        }
        std::thread::sleep(Duration::from_millis(5));
        self.fleet = Arc::new(Fleet::new(now_ms()));
        self.serve();
        Ok(())
    }

    fn retry_item(&mut self) -> Result<String, String> {
        Err("the in-process server has no supervisor".into())
    }

    fn terminal_instance(&self) -> String {
        "none".into()
    }
}

/// Runs `agend <args>` with `AGEND_HOME=home` (and `extra` env), waiting at
/// most `limit`; an overrun child is killed (`Child::kill`).
pub fn agend(
    lab: &Lab,
    home: &Path,
    args: &[&str],
    extra: &[(&str, &str)],
    limit: Duration,
) -> Result<(Output, Duration), String> {
    let started = Instant::now();
    let child = spawn_agend(lab, home, args, extra)?;
    wait_output(child, limit).map(|out| (out, started.elapsed()))
}

pub fn spawn_agend(
    lab: &Lab,
    home: &Path,
    args: &[&str],
    extra: &[(&str, &str)],
) -> Result<Child, String> {
    let mut cmd = Command::new(&lab.agend);
    cmd.args(args)
        .env_clear()
        .env("AGEND_HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd.spawn().map_err(|e| format!("run agend {args:?}: {e}"))
}

pub fn wait_output(mut child: Child, limit: Duration) -> Result<Output, String> {
    let deadline = Instant::now() + limit;
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let out = child.wait_with_output().map_err(|e| e.to_string())?;
            return Err(format!(
                "did not end within {limit:?}; stdout:\n{}",
                String::from_utf8_lossy(&out.stdout)
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().map_err(|e| e.to_string())
}

/// Stops our own child (`Child::kill`) and returns what it printed.
pub fn kill_output(mut child: Child) -> Result<String, String> {
    let _ = child.kill();
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    Ok(text(&out.stdout))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim_end().to_owned()
}

/// `== version`: `agend debug ping` against a fake daemon that only speaks
/// 1.0 fails at once with what to do (no 10 s of retries).
pub fn version(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(30);
    fs::create_dir_all(home.join("run")).map_err(|e| e.to_string())?;
    let fake = FakeDaemon::start_at(&socket_of(&home)).map_err(|e| e.to_string())?;
    fake.set_supported_versions(&[agend_core::protocol::client::V1]);
    let (out, took) = agend(lab, &home, &["debug", "ping"], &[], Duration::from_secs(20))?;
    let said = text(&out.stderr);
    ensure(
        out.status.code() == Some(1) && said.contains("needs 1.1") && took < Duration::from_secs(3),
        || {
            format!(
                "ping against a 1.0 daemon: {} in {took:?}: {said}",
                out.status
            )
        },
    )?;
    Ok(vec![
        "a daemon that speaks only client protocol 1.0 (the fake):".into(),
        format!(
            "  agend debug ping → exit 1 in {:.2} s: {said}",
            took.as_secs_f64()
        ),
    ])
}

/// `== socket` (P1): `run/` 0700, the socket 0600, a path over 100 bytes
/// refused before anything is opened, a leftover socket after `kill -9`
/// replaced, and the socket removed on Ctrl-C.
pub fn socket(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(31);
    let mut out = Vec::new();
    let mut daemon = Daemon::start(lab, &home, &[])?;
    daemon.ready()?;
    let sock = socket_of(&home);
    let mode = |p: &Path| fs::symlink_metadata(p).map(|m| m.permissions().mode() & 0o777);
    let (run_mode, sock_mode) = (
        mode(&home.join("run")).map_err(|e| e.to_string())?,
        mode(&sock).map_err(|e| e.to_string())?,
    );
    ensure(run_mode == 0o700 && sock_mode == 0o600, || {
        format!("run/ {run_mode:o}, daemon.sock {sock_mode:o}")
    })?;
    out.push(format!(
        "run/ is {run_mode:o}, run/daemon.sock is {sock_mode:o}, bound before the ready line"
    ));
    let listening = daemon.expect("listening on")?;
    let ready = daemon.expect("agend daemon ready:")?;
    let order = daemon.log.iter().position(|l| *l == listening)
        < daemon.log.iter().position(|l| *l == ready);
    ensure(order, || {
        "ready was logged before the socket was bound".into()
    })?;
    daemon.kill9()?;
    ensure(sock.exists(), || "kill -9 removed the socket?".into())?;
    let mut daemon = Daemon::start(lab, &home, &[])?;
    daemon.ready()?;
    let (ping, _) = agend(lab, &home, &["debug", "ping"], &[], Duration::from_secs(20))?;
    ensure(ping.status.success(), || text(&ping.stderr))?;
    out.push(format!(
        "after kill -9 the old socket file stayed; the next daemon replaced it: {}",
        text(&ping.stdout)
    ));
    daemon.interrupt()?;
    ensure(!sock.exists(), || "Ctrl-C left run/daemon.sock".into())?;
    out.push("Ctrl-C: run/daemon.sock removed".into());
    // 100 bytes is the limit; this home makes the socket path 101.
    let long = lab
        .root
        .join("l".repeat(101 - lab.root.as_os_str().len() - 1 - DAEMON_SOCKET.len() - 1));
    fs::create_dir_all(&long).map_err(|e| e.to_string())?;
    ensure(socket_of(&long).as_os_str().len() == 101, || {
        format!("long path is {}", socket_of(&long).display())
    })?;
    let mut refused = Daemon::start(lab, &long, &[])?;
    let status = refused.wait(Duration::from_secs(20))?;
    let said = refused.log.join("\n");
    ensure(
        status.code() == Some(1)
            && said.contains("socket path too long")
            && !long.join("agend.db").exists(),
        || format!("a 101-byte socket path: {status}: {said}"),
    )?;
    out.push(format!(
        "101-byte socket path: exit 1: {said}; no agend.db created"
    ));
    out.extend(bound_after_boot(lab)?);
    Ok(out)
}

/// The socket appears only after the boot plan: an instance whose holder
/// lock is held (by this process, with its own pid) but that never
/// answers makes the plan wait 5 s for it; a client that connects the
/// moment the socket accepts sees the instance after it already started.
fn bound_after_boot(lab: &Lab) -> Result<Vec<String>, String> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    let home = lab.home(35);
    let t = tag();
    let (slow, next) = (format!("g8-{t}a"), format!("g8-{t}b"));
    add(&home, &slow, Backend::Claude, lab::COUNTER)?;
    add(&home, &next, Backend::Claude, lab::COUNTER)?;
    fs::create_dir_all(files::holders_dir(&home)).map_err(|e| e.to_string())?;
    let mut lock = fs::File::create(files::lock_path(&home, &slow)).map_err(|e| e.to_string())?;
    write!(lock, "{}", std::process::id()).map_err(|e| e.to_string())?;
    // SAFETY: flock on a file this function owns; released when `lock`
    // drops, always before this function returns (the lab's cleanup must
    // never find this process's pid in a held lock).
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("cannot lock the slow instance's lock file".into());
    }
    let socket = socket_of(&home);
    let started = Instant::now();
    let mut daemon = Daemon::start(lab, &home, &[])?;
    let mut client = loop {
        match ProbeClient::hello(&socket, None) {
            Ok((client, _)) => break client,
            Err(_) if started.elapsed() < Duration::from_secs(30) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) => return Err(format!("the socket never accepted: {e}")),
        }
    };
    let appeared = started.elapsed();
    let reply = client
        .request(&ClientRequest::GetFleet {
            data: RequestIdData {
                request_id: "g8-boot".into(),
            },
        })
        .map_err(|e| e.to_string())?;
    drop(lock);
    let ClientResponse::Fleet { data } = reply else {
        return Err(format!("get_fleet: {reply:?}"));
    };
    let state = |id: &str| {
        data.fleet
            .instances
            .iter()
            .find(|i| i.instance_id == id)
            .map(|i| i.state.as_str())
    };
    ensure(
        appeared >= Duration::from_secs(4) && state(&next) == Some("unknown"),
        || {
            format!(
                "the socket accepted after {appeared:?}; {next} was {:?} (expected unknown: started by the boot plan)",
                state(&next)
            )
        },
    )?;
    daemon.ready()?;
    daemon.interrupt()?;
    Ok(vec![format!(
        "boot plan waiting 5 s on {slow}: the socket accepted only after {:.1} s, and the first get_fleet already shows {next} running",
        appeared.as_secs_f64()
    )])
}

/// `== retry` (P5): what `retry` does for each backend and whether the
/// session was ever started; a codex/opencode instance that ran has no
/// action.
pub fn retry(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(32);
    let t = tag();
    let (cr, cn, xn, xs) = (
        format!("g8-{t}cr"),
        format!("g8-{t}cn"),
        format!("g8-{t}xn"),
        format!("g8-{t}xs"),
    );
    let claude_ran = add(&home, &cr, Backend::Claude, lab::RECORDS_ARGS)?;
    // Boot 1 starts it for real: a session and a holder that stays (H8).
    let mut daemon = Daemon::start(lab, &home, &[])?;
    daemon.ready()?;
    daemon.expect(&format!("{cr}: holder pid="))?;
    daemon.interrupt()?;
    let kept = files::running(&home, &cr)
        .ok()
        .flatten()
        .ok_or("the holder of the claude instance is gone")?;
    set_status(&home, &cr, InstanceStatus::Failed)?;
    let claude_new = add(&home, &cn, Backend::Claude, lab::RECORDS_ARGS)?;
    set_status(&home, &cn, InstanceStatus::Failed)?;
    add(&home, &xn, Backend::Codex, lab::RECORDS_ARGS)?;
    set_status(&home, &xn, InstanceStatus::Failed)?;
    add(&home, &xs, Backend::Codex, lab::RECORDS_ARGS)?;
    set_status(&home, &xs, InstanceStatus::Running)?;
    set_status(&home, &xs, InstanceStatus::Failed)?;

    let mut daemon = Daemon::start(lab, &home, &[])?;
    daemon.ready()?;
    let socket = socket_of(&home);
    let (mut c, _) = ProbeClient::hello(&socket, None).map_err(|e| e.to_string())?;
    let fleet = match c
        .request(&ClientRequest::GetFleet {
            data: RequestIdData {
                request_id: "g8-r".into(),
            },
        })
        .map_err(|e| e.to_string())?
    {
        ClientResponse::Fleet { data } => data.fleet,
        other => return Err(format!("get_fleet: {other:?}")),
    };
    let mut out = Vec::new();
    for id in [&cr, &cn, &xn, &xs] {
        let item = fleet
            .attention
            .iter()
            .find(|a| a.instance_id.as_deref() == Some(id.as_str()))
            .ok_or_else(|| format!("{id} is not a needs-you item"))?;
        let actions: Vec<&str> = item.actions.iter().map(|a| a.as_str()).collect();
        let expected: &[&str] = if *id == xs { &[] } else { &["retry"] };
        ensure(actions == expected, || format!("{id}: actions {actions:?}"))?;
        out.push(format!(
            "{}: actions [{}]; if ignored: {}",
            item.attention_id.as_deref().unwrap_or_default(),
            actions.join(", "),
            item.if_ignored.as_deref().unwrap_or_default()
        ));
    }
    // The failed claude's kept holder: its last screen, once, no bytes.
    c.send(&ClientRequest::SubscribeTerminal {
        data: InstanceData {
            instance_id: cr.clone(),
        },
    })
    .map_err(|e| e.to_string())?;
    let first = c
        .recv_within(Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    ensure(
        matches!(&first, Some(ClientResponse::TerminalSnapshot { data }) if data.screen.contains("counter=")),
        || format!("failed {cr} with its holder: {first:?}"),
    )?;
    let more = c.recv_within(Duration::from_millis(2500));
    ensure(more.is_err(), || {
        format!("a failed instance streamed: {more:?}")
    })?;
    out.push(format!(
        "{cr} (failed, holder kept): subscribe_terminal → its last screen, then nothing"
    ));
    let (mut c, _) = ProbeClient::hello(&socket, None).map_err(|e| e.to_string())?;
    c.send(&ClientRequest::SubscribeTerminal {
        data: InstanceData {
            instance_id: cn.clone(),
        },
    })
    .map_err(|e| e.to_string())?;
    let reply = c
        .recv_within(Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    ensure(
        matches!(&reply, Some(ClientResponse::Error { data }) if data.code == error_code::NO_TERMINAL),
        || format!("failed {cn} without a holder: {reply:?}"),
    )?;
    out.push(format!(
        "{cn} (failed, no holder): subscribe_terminal → no_terminal"
    ));

    let refused = resolve(&socket, &format!("instance-failed:{xs}"));
    ensure(
        refused
            .as_ref()
            .is_err_and(|e| e.contains(error_code::UNKNOWN_ATTENTION)),
        || format!("retry of {xs}: {refused:?}"),
    )?;
    for id in [&cr, &cn, &xn] {
        resolve(&socket, &format!("instance-failed:{id}"))?;
        daemon.expect(&format!("{id}: holder pid="))?;
    }
    // The kept holder got `Shutdown` before the new start: one start, no
    // failed start, no restart (a start next to a live holder is refused
    // and would only recover through a restart).
    for id in [&cr, &cn, &xn] {
        let lines = |needle: String| daemon.log.iter().filter(|l| l.contains(&needle)).count();
        let (starts, failed, restarts) = (
            lines(format!("{id}: holder pid=")),
            lines(format!("{id}: start failed")),
            lines(format!("{id}: restart ")),
        );
        ensure(starts == 1 && failed == 0 && restarts == 0, || {
            format!(
                "{id} after retry: {starts} holder start(s), {failed} failed start(s), {restarts} restart(s); log:\n{}",
                daemon.log.join("\n")
            )
        })?;
    }
    let args = |i: &Instance| -> Result<Vec<String>, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = fs::read_to_string(Path::new(&i.working_directory).join("args.log"))
                .unwrap_or_default();
            let lines: Vec<String> = text.lines().map(str::to_owned).collect();
            if lines.len() >= 2 || Instant::now() > deadline {
                return Ok(lines);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let cr_session = claude_ran.session_id.clone().unwrap_or_default();
    let cn_session = claude_new.session_id.clone().unwrap_or_default();
    let starts: Vec<String> = [&cr, &cn, &xn]
        .iter()
        .filter_map(|id| {
            daemon
                .log
                .iter()
                .find(|l| l.contains(&format!("{id}: start")))
                .map(|l| lab::untimed(l).to_owned())
        })
        .collect();
    ensure(
        starts
            == [
                format!("{cr}: start --resume {cr_session}"),
                format!("{cn}: start --session-id {cn_session}"),
                format!("{xn}: start"),
            ],
        || format!("starts after retry: {starts:#?}"),
    )?;
    let cr_args = args(&claude_ran)?;
    ensure(
        cr_args
            == [
                format!("--session-id {cr_session}"),
                format!("--resume {cr_session}"),
            ],
        || format!("{cr} was started with {cr_args:?}"),
    )?;
    let now = files::running(&home, &cr).ok().flatten();
    ensure(now.is_some() && now != Some(kept), || {
        format!("{cr}: the kept holder {kept} was not replaced ({now:?})")
    })?;
    daemon.interrupt()?;
    let statuses: Vec<String> = [&cr, &cn, &xn, &xs]
        .iter()
        .map(|id| {
            lab::status(&home, id)
                .map(|s| format!("{id}={}", s.as_str()))
                .unwrap_or_else(|e| e)
        })
        .collect();
    out.extend(
        starts
            .iter()
            .map(|s| format!("retry → {s}"))
            .map(|s| s.replace(&cr_session, "<S1>").replace(&cn_session, "<S2>")),
    );
    out.push(format!(
        "{cr}: kept holder pid={kept} got Shutdown first; agent's own record: {}",
        cr_args.join(" | ").replace(&cr_session, "<S1>")
    ));
    out.push(format!(
        "{xs} (codex, ran before): retry → unknown_attention; DB now: {}",
        statuses.join(" ")
    ));
    Ok(out)
}

/// `== terminal` (P6): a running instance's screen, then its bytes; an
/// unknown instance gets `no_terminal`; agent commands are not supported.
pub fn terminal(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(33);
    let id = format!("g8-{}t", tag());
    add(&home, &id, Backend::Claude, lab::COUNTER)?;
    let mut daemon = Daemon::start(lab, &home, &[])?;
    daemon.ready()?;
    // Let the counter print its first line.
    std::thread::sleep(Duration::from_millis(1500));
    let socket = socket_of(&home);
    let (mut c, _) = ProbeClient::hello(&socket, None).map_err(|e| e.to_string())?;
    c.send(&ClientRequest::SubscribeTerminal {
        data: InstanceData {
            instance_id: id.clone(),
        },
    })
    .map_err(|e| e.to_string())?;
    let first = c
        .recv_within(Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    let screen = match first {
        Some(ClientResponse::TerminalSnapshot { data }) if data.instance_id == id => data.screen,
        other => return Err(format!("first reply {other:?}")),
    };
    let next = c
        .recv_within(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    ensure(
        matches!(&next, Some(ClientResponse::TerminalBytes { data }) if data.instance_id == id),
        || format!("after the screen: {next:?}"),
    )?;
    let last = screen
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default();
    let mut out = vec![format!(
        "{id}: terminal_snapshot (last line {last:?}), then terminal_bytes"
    )];
    let (mut c, _) = ProbeClient::hello(&socket, None).map_err(|e| e.to_string())?;
    c.send(&ClientRequest::SubscribeTerminal {
        data: InstanceData {
            instance_id: "g8-nobody".into(),
        },
    })
    .map_err(|e| e.to_string())?;
    let reply = c
        .recv_within(Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    let Some(ClientResponse::Error { data }) = reply else {
        return Err(format!("unknown instance: {reply:?}"));
    };
    ensure(data.code == error_code::NO_TERMINAL, || data.code.clone())?;
    out.push(format!("g8-nobody: error {}: {}", data.code, data.message));
    let reply = c
        .request(&ClientRequest::Command {
            data: agend_core::protocol::client::ClientCommandData {
                request_id: "g8-c".into(),
                command: agend_core::protocol::client::AgentCommand::Status,
            },
        })
        .map_err(|e| e.to_string())?;
    let Some(("not_supported", message)) = (match &reply {
        ClientResponse::Error { data } => Some((data.code.as_str(), data.message.clone())),
        _ => None,
    }) else {
        return Err(format!("command status: {reply:?}"));
    };
    out.push(format!("command status: error not_supported: {message}"));
    daemon.interrupt()?;
    Ok(out)
}

/// `== restart` (P4, P7): `agend debug ping --count 12 --interval 300`
/// while the daemon restarts (with a 1.5 s pause, like a person) → all ok,
/// at least one retried; a running `agend debug watch` reconnects and
/// fetches the fleet view again.
pub fn restart(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(34);
    let id = format!("g8-{}p", tag());
    add(&home, &id, Backend::Claude, lab::COUNTER)?;
    let mut daemon = Daemon::start(lab, &home, &[])?;
    daemon.ready()?;
    let watch = spawn_agend(lab, &home, &["debug", "watch"], &[])?;
    let ping = spawn_agend(
        lab,
        &home,
        &["debug", "ping", "--count", "12", "--interval", "300"],
        &[],
    )?;
    std::thread::sleep(Duration::from_millis(1000));
    daemon.interrupt()?;
    std::thread::sleep(Duration::from_millis(1500));
    let mut daemon = Daemon::start(lab, &home, &[])?;
    daemon.ready()?;
    let pinged = wait_output(ping, Duration::from_secs(60))?;
    std::thread::sleep(Duration::from_millis(1500));
    daemon.interrupt()?;
    // The watch never ends by itself: stop our own child.
    let watched = kill_output(watch)?;
    let lines = text(&pinged.stdout);
    let ok = lines.lines().filter(|l| l.starts_with("ok ")).count();
    let retried: Vec<&str> = lines.lines().filter(|l| l.contains("(retried")).collect();
    ensure(
        pinged.status.success() && ok == 12 && !retried.is_empty(),
        || {
            format!(
                "ping during the restart: {}\n{lines}\n{}",
                pinged.status,
                text(&pinged.stderr)
            )
        },
    )?;
    let fleets = watched.lines().filter(|l| l.starts_with("fleet:")).count();
    ensure(
        fleets >= 2 && watched.contains("disconnected: the daemon closed the connection"),
        || format!("watch across the restart:\n{watched}"),
    )?;
    let mut out = vec![format!(
        "ping --count 12 --interval 300 across a restart: {ok}/12 ok; {}",
        retried.join("; ")
    )];
    out.push("debug watch across the restart:".into());
    out.extend(
        watched
            .lines()
            .filter(|l| {
                l.starts_with("fleet:")
                    || l.starts_with("disconnected")
                    || l.starts_with("reconnecting")
            })
            .map(|l| format!("  {l}")),
    );
    Ok(out)
}
