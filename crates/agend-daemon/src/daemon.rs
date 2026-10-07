//! `agend daemon` (gate 6 P1, P5): runs in the foreground only; keeping it
//! running is launchd's or systemd's job (gate 13). The caller (`agend`'s
//! `home::resolve`, gate 9 P3) passes the home.
//!
//! Boot order: check the client socket path fits (100 bytes) → open
//! `agend.db` (the one daemon per home: the DB's exclusive lock, retried
//! every 200 ms for 10 s while an old daemon hands it over) → remove a stale
//! `run/daemon.sock` → housekeeping (failures only logged) → shim symlinks
//! and codex's `ZDOTDIR` (`zsh/.zprofile`, gate 7 P4) → the boot plan (reconnect / start / orphans) → bind `run/daemon.sock`
//! (`run/` 0700, socket 0600, gate 8 P1) → `agend daemon ready: …`. A
//! socket that accepts connections is the readiness signal: no `.ready`
//! file, and a client never sees a half-booted fleet. Housekeeping runs
//! again every hour.
//!
//! SIGINT / SIGTERM: stop handling events, stop the client socket (stop
//! accepting, remove `daemon.sock`, close every client), close the holder
//! connections, close the DB, exit 0. Holders keep running and are never
//! sent `Shutdown` (D3).
//!
//! `agend daemon restart` (gate 9 P7): once the preflight passed, the same
//! stop, then `exec` of the new binary as `agend daemon` (same pid, same
//! environment and terminal; every fd is close-on-exec). The new image
//! reaps the holders it inherited (`crate::reaper`). A SIGINT / SIGTERM
//! that arrives during that hand-off stops the daemon instead (no `exec`). An `exec` that fails
//! (the binary vanished after its preflight) prints why and exits 1.
//!
//! Exit codes: 0 after a signal, 1 when it cannot start (socket path too
//! long, `agend.db` in use or broken, shims, socket) or cannot `exec`.
//!
//! Must NOT: fork into the background, write pid/ready/cookie files, or
//! stop holders when it stops.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

use agend_core::protocol::client::DAEMON_SOCKET;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::driver::codex::{CodexDriver, CodexSink, launch as codex_launch};
use crate::fleet::Fleet;
use crate::handlers::Context;
use crate::log;
use crate::runtime::{EventSink, HolderRuntime, shims};
use crate::server::{self, Server};
use crate::store::{SqliteStore, StoreError};
use crate::supervisor::{Event, Stopped, Supervisor};

/// How long `agend.db` is retried while another process holds it (P1).
pub const DB_RETRY_FOR: Duration = Duration::from_secs(10);
pub const DB_RETRY_EVERY: Duration = Duration::from_millis(200);
/// Housekeeping after boot (P5).
pub const HOUSEKEEPING_EVERY: Duration = Duration::from_secs(60 * 60);

/// Runs the daemon for `home` (absolute, checked by the caller).
pub fn run(home: PathBuf) -> ExitCode {
    run_with_policy(
        home,
        None,
        agend_core::policy::codex_input::CodexInputPolicy::approved(),
    )
}

/// Runs the same daemon in a one-instance U17 diagnostic scope.
/// Not exposed by the normal CLI; requires explicit AGEND_U17_PROBE=1.
/// The caller supplies the real agend binary for holder/shim dispatch.
pub fn run_u17_probe(home: PathBuf, agend: PathBuf, instance: String) -> ExitCode {
    if std::env::var("AGEND_U17_PROBE").as_deref() != Ok("1")
        || instance.is_empty()
        || !home.is_absolute()
        || !agend.is_absolute()
        || !agend.is_file()
    {
        eprintln!(
            "U17 diagnostic daemon requires AGEND_U17_PROBE=1, an instance, absolute home and agend binary"
        );
        return ExitCode::from(2);
    }
    run_with_policy(
        home,
        Some(agend),
        agend_core::policy::codex_input::CodexInputPolicy::for_u17_verification(instance),
    )
}

fn run_with_policy(
    home: PathBuf,
    agend: Option<PathBuf>,
    codex_input: agend_core::policy::codex_input::CodexInputPolicy,
) -> ExitCode {
    if let Err(e) = server::check_socket_len(&home.join(DAEMON_SOCKET)) {
        eprintln!("agend daemon: {e}");
        return ExitCode::from(1);
    }
    let telegram = match crate::notifier::config::load(&home) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("agend daemon: {error}");
            return ExitCode::from(1);
        }
    };
    let exe = match agend.map(Ok).unwrap_or_else(std::env::current_exe) {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("agend daemon: cannot find its own binary: {e}");
            return ExitCode::from(1);
        }
    };
    let (store, waited) = match open_store(&home) {
        Ok(opened) => opened,
        Err(e) => {
            eprintln!("agend daemon: {e}");
            return ExitCode::from(1);
        }
    };
    // Only now, with agend.db ours, does this daemon write to logs/.
    if let Err(e) = log::to_files(&home) {
        eprintln!("agend daemon: cannot write logs/: {e}");
    }
    log::line(&format!(
        "agend daemon starting: pid={} home={}",
        std::process::id(),
        home.display()
    ));
    log::line(&format!(
        "agend.db opened (waited {} ms for the lock)",
        waited.as_millis()
    ));
    let _stop_flags = match stop_flag::install() {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("agend daemon: cannot watch stop signals: {e}");
            return ExitCode::from(1);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            log::line(&format!("agend daemon: cannot build the runtime: {e}"));
            return ExitCode::from(1);
        }
    };
    let stopped = runtime.block_on(serve(home, exe, store, codex_input, telegram));
    // Pending restart timers and the like are dropped, not awaited.
    runtime.shutdown_timeout(Duration::from_secs(1));
    if matches!(stopped, Ok(Stopped::Exec(_))) {
        stop_flag::begin_exec_handoff();
    }
    match stopped {
        Ok(Stopped::Signal(_)) => ExitCode::SUCCESS,
        Ok(Stopped::Exec(_)) if STOP_SIGNALLED.load(Ordering::SeqCst) => {
            log::line("a stop signal came during the restart; not restarting");
            ExitCode::SUCCESS
        }
        Ok(Stopped::Exec(binary)) => exec(&binary),
        Err(code) => code,
    }
}

/// Replaces this process with `<binary> daemon`; returns only on failure.
fn exec(binary: &Path) -> ExitCode {
    use std::os::unix::process::CommandExt;
    log::line(&format!("exec {} daemon", binary.display()));
    let error = std::process::Command::new(binary).arg("daemon").exec();
    log::line(&format!(
        "agend daemon: cannot exec {}: {error}; start it again with: agend daemon",
        binary.display()
    ));
    ExitCode::from(1)
}

/// Opens `agend.db`, retrying while another process holds it (the old
/// daemon of a restart is still exiting). Returns how long it waited.
fn open_store(home: &Path) -> Result<(SqliteStore, Duration), StoreError> {
    let started = Instant::now();
    loop {
        match SqliteStore::open(home, log::now_unix_ms()) {
            Ok(store) => return Ok((store, started.elapsed())),
            Err(StoreError::InUse) if started.elapsed() < DB_RETRY_FOR => {
                std::thread::sleep(DB_RETRY_EVERY);
            }
            Err(e) => return Err(e),
        }
    }
}

/// `run/` 0700 (created if missing, tightened if not), and no leftover
/// `daemon.sock` from a daemon that died: this one holds `agend.db`.
fn prepare_run_dir(home: &Path) -> std::io::Result<PathBuf> {
    let run = home.join("run");
    DirBuilder::new().recursive(true).mode(0o700).create(&run)?;
    fs::set_permissions(&run, fs::Permissions::from_mode(0o700))?;
    let socket = home.join(DAEMON_SOCKET);
    match fs::remove_file(&socket) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(socket),
    }
}

/// A stop signal arrived. Also read after the supervisor has returned to
/// `exec`: a Ctrl-C during that hand-off must stop the daemon, not be lost
/// in the old image while the new one keeps running (gate 9).
mod stop_flag;
use stop_flag::STOP_SIGNALLED;

fn forward_signal(kind: SignalKind, name: &'static str, events: UnboundedSender<Event>) {
    match signal(kind) {
        Ok(mut stream) => {
            tokio::spawn(async move {
                if stream.recv().await.is_some() {
                    STOP_SIGNALLED.store(true, Ordering::SeqCst);
                    let _ = events.send(Event::Stop(name));
                }
            });
        }
        Err(e) => log::line(&format!("cannot watch {name}: {e}")),
    }
}

async fn serve(
    home: PathBuf,
    exe: PathBuf,
    store: SqliteStore,
    codex_input: agend_core::policy::codex_input::CodexInputPolicy,
    telegram: Option<(
        agend_core::config::TelegramConfig,
        crate::notifier::config::Token,
    )>,
) -> Result<Stopped, ExitCode> {
    let (events, mut queue) = unbounded_channel();
    forward_signal(SignalKind::interrupt(), "SIGINT", events.clone());
    forward_signal(SignalKind::terminate(), "SIGTERM", events.clone());
    if STOP_SIGNALLED.load(Ordering::SeqCst) {
        return Ok(Stopped::Signal("stop during startup"));
    }

    let socket = match prepare_run_dir(&home) {
        Ok(socket) => socket,
        Err(e) => {
            log::line(&format!("agend daemon: cannot prepare run/: {e}"));
            return Err(ExitCode::from(1));
        }
    };
    crate::housekeeping::run(&store, &home, log::now_unix_ms()).await;
    match shims::ensure(&home, &exe) {
        Ok(fixed) if !fixed.is_empty() => {
            log::line(&format!("shims: {} -> {}", fixed.join(", "), exe.display()))
        }
        Ok(_) => {}
        Err(e) => {
            log::line(&format!(
                "agend daemon: cannot set up the shims in bin/: {e}"
            ));
            return Err(ExitCode::from(1));
        }
    }
    let daemon_env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    let launch_path =
        crate::runtime::env::launch_path(&home, &daemon_env.iter().cloned().collect());
    if let Err(e) = codex_launch::ensure_zdotdir(&home, &launch_path) {
        log::line(&format!(
            "agend daemon: cannot write {}: {e}",
            codex_launch::zdotdir(&home).join(".zprofile").display()
        ));
        return Err(ExitCode::from(1));
    }

    let sink_events = events.clone();
    let sink: EventSink = Arc::new(move |event| {
        let _ = sink_events.send(Event::Holder(event));
    });
    // Before any holder starts: the holders an exec restart left us.
    let inherited = crate::reaper::inherited(&home);
    let runtime = HolderRuntime::new(&home, &exe, daemon_env, sink);
    let fleet = Arc::new(Fleet::new(log::now_unix_ms()));
    use agend_core::attention_read::AttentionReadStore;
    fleet.restore_read_keys(store.attention_read_keys().await.map_err(|e| {
        log::line(&format!("read receipts: {e}"));
        ExitCode::from(1)
    })?);
    let store = Arc::new(store);
    let codex_events = events.clone();
    let codex_sink: CodexSink = Arc::new(move |event| {
        let _ = codex_events.send(Event::Codex(event));
    });
    let codex =
        CodexDriver::with_input_policy(&home, Arc::clone(&store), codex_sink, codex_input.clone());
    let mut supervisor = Supervisor::new(
        Arc::clone(&store),
        runtime.clone(),
        codex.clone(),
        events.clone(),
        Arc::clone(&fleet),
    );
    crate::reaper::watch(inherited);
    let report = match supervisor.boot().await {
        Ok(report) => report,
        Err(e) => {
            log::line(&format!("agend daemon: boot failed: {e}"));
            return Err(ExitCode::from(1));
        }
    };
    let (pipeline, pipeline_worker) = crate::pipeline::start(
        &home,
        &exe,
        Arc::clone(&store),
        Arc::clone(&fleet),
        codex.clone(),
    )
    .await
    .map_err(|e| {
        log::line(&format!("pipeline boot: {e}"));
        ExitCode::from(1)
    })?;
    supervisor.set_pipeline(pipeline.clone());
    // Bound only now (P1): a client that connects sees the whole fleet.
    let listener = match server::bind(&socket) {
        Ok(listener) => listener,
        Err(e) => {
            log::line(&format!(
                "agend daemon: cannot listen on {}: {e}",
                socket.display()
            ));
            return Err(ExitCode::from(1));
        }
    };
    let context = Arc::new(Context {
        pipeline,
        fleet,
        runtime,
        supervisor: events.clone(),
        store,
        codex,
        exe,
        restarting: AtomicBool::new(false),
        codex_input,
    });
    let telegram_worker = telegram
        .map(|(config, token)| crate::notifier::worker::start(config, token, context.clone()));
    let server = Server::start(listener, socket.clone(), Arc::clone(&context));
    log::line(&format!("listening on {}", socket.display()));
    log::line(&format!(
        "agend daemon ready: instances={} recovered={} started={} orphans={}",
        report.instances, report.recovered, report.started, report.orphans
    ));

    let hourly = events.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(HOUSEKEEPING_EVERY);
        every.tick().await; // the first tick is immediate; boot already did it
        loop {
            every.tick().await;
            if hourly.send(Event::Housekeeping).is_err() {
                return;
            }
        }
    });

    let stopped = supervisor.run(&mut queue).await;
    // Reject new operations and release pending completion waiters before workers stop.
    drop(queue);
    let why = match &stopped {
        Stopped::Signal(signal) => (*signal).to_owned(),
        Stopped::Exec(binary) => format!("restart with {}", binary.display()),
    };
    log::line(&format!(
        "agend daemon stopping ({why}); holders keep running"
    ));
    server.stop().await;
    if let Some(worker) = telegram_worker {
        worker.stop().await;
    }
    // Closes every holder connection (no Shutdown) and then the DB: the
    // server's tasks are gone, so this is the last handle on both.
    pipeline_worker.abort();
    drop(context);
    drop(supervisor);
    log::line("agend daemon stopped");
    Ok(stopped)
}
