//! Runtime adapter (`HolderRuntime`): the daemon-side client of the holder
//! protocol (gate 6 P3, P4). On daemon restart it reconnects to every holder
//! and takes the holder's current screen (no byte replay).
//!
//! - Start: `agend holder <id>` with an empty environment except
//!   `AGEND_HOME` (the holder `setsid`s itself, so no `process_group(0)`),
//!   then the first connection (50 ms retries, 5 s) and `Spawn` with the
//!   agent's whitelisted environment ([`env`]). A thread per started holder
//!   `wait`s for it, so it never stays a zombie; holders reconnected after a
//!   restart are not our children and are watched through their link.
//! - Recover: only the `flock` of each lock file ([`files`]); never a
//!   connection, which would take over the daemon's own (gate 4 P4).
//! - Stop: closes the link, then `Shutdown` on a fresh connection and waits
//!   for the lock to be released.
//! - Dropping the runtime (its last clone) closes every link and tells no
//!   holder anything: holders outlive the daemon (D3).
//! - Terminal (gate 8 P6): [`HolderRuntime::live_terminal`] goes through the
//!   link (screen, then its PTY bytes); [`HolderRuntime::last_screen`] is a
//!   short connection for a holder the daemon has no link to (a `failed`
//!   instance's), used only for such holders.
//!
//! The blocking protocol calls run in `spawn_blocking` inside the daemon's
//! tokio runtime.
//!
//! Must NOT: own the PTY or the agent process, send `Shutdown` except from
//! [`HolderRuntime::stop`] / [`shutdown_holder`], or connect to a holder to
//! find out whether it runs.

pub mod client;
pub mod env;
pub mod files;
pub mod link;
pub mod shims;

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agend_core::protocol::holder::{HolderRequest, SpawnData};
use agend_core::traits::{HolderHandle, HolderLaunch, Runtime};

use crate::log;
use crate::store::instances::validate_id;
use client::Conn;
pub use link::{Attached, EventSink, HolderEvent, SpawnOutcome, TerminalFeed};

/// How long a stopping holder may take to release its lock (its agent gets
/// 5 s after SIGHUP, gate 4 P7).
pub const STOP_WITHIN: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeError(pub String);

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RuntimeError {}

fn err(message: impl Into<String>) -> RuntimeError {
    RuntimeError(message.into())
}

/// A holder this runtime started or reconnected to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub handle: HolderHandle,
    pub attached: Attached,
    /// Tags the link's [`HolderEvent`]s.
    pub generation: u64,
}

#[derive(Clone)]
pub struct HolderRuntime {
    inner: Arc<Inner>,
}

struct Inner {
    home: PathBuf,
    agend: PathBuf,
    daemon_env: Vec<(String, String)>,
    sink: EventSink,
    links: Mutex<BTreeMap<String, link::Link>>,
    next_generation: AtomicU64,
}

impl HolderRuntime {
    /// A runtime for holders under `home`, started as `<agend> holder <id>`.
    /// `daemon_env` is what the agent environment whitelist copies from
    /// (normally `std::env::vars()`); `sink` receives the links' events.
    pub fn new(
        home: &Path,
        agend: &Path,
        daemon_env: Vec<(String, String)>,
        sink: EventSink,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                home: home.to_path_buf(),
                agend: agend.to_path_buf(),
                daemon_env,
                sink,
                links: Mutex::new(BTreeMap::new()),
                next_generation: AtomicU64::new(1),
            }),
        }
    }

    pub fn home(&self) -> &Path {
        &self.inner.home
    }

    /// Starts a holder for `launch` and its agent.
    pub async fn start(&self, launch: &HolderLaunch) -> Result<Started, RuntimeError> {
        let inner = Arc::clone(&self.inner);
        let launch = launch.clone();
        blocking(move || inner.start(&launch)).await
    }

    /// Connects to the running holder of `launch.instance_id` and re-sends
    /// `Spawn` (answered `already_spawned` when its agent was started, gate
    /// 6 P3); `pid` is the holder's, from its lock.
    pub async fn attach(&self, launch: &HolderLaunch, pid: u32) -> Result<Started, RuntimeError> {
        let inner = Arc::clone(&self.inner);
        let launch = launch.clone();
        blocking(move || {
            let (attached, generation) = inner.attach(&launch)?;
            Ok(Started {
                handle: inner.handle(&launch.instance_id, pid),
                attached,
                generation,
            })
        })
        .await
    }

    /// Stops the holder of `id` (`Shutdown`) and waits until it is gone.
    pub async fn stop(&self, id: &str) -> Result<(), RuntimeError> {
        let inner = Arc::clone(&self.inner);
        let id = id.to_owned();
        blocking(move || inner.stop(&id)).await
    }

    /// Every running holder under the home, from the lock files alone.
    pub fn recover(&self) -> Result<Vec<HolderHandle>, RuntimeError> {
        let running = files::running_holders(&self.inner.home)
            .map_err(|e| err(format!("scan run/holders: {e}")))?;
        Ok(running
            .into_iter()
            .map(|(id, pid)| self.inner.handle(&id, pid))
            .collect())
    }

    /// The screen of `id`'s holder and its PTY chunks after it, through the
    /// daemon's link; `None` without a link.
    pub fn live_terminal(&self, id: &str) -> Option<tokio::sync::oneshot::Receiver<TerminalFeed>> {
        self.inner.lock_links().get(id)?.terminal()
    }

    /// The screen of a running holder the daemon has no link to, from one
    /// short connection (it restarts the holder's idle timer, gate 4 G5);
    /// `None` when no holder runs. Never for a holder with a link: a new
    /// connection takes the link's over.
    pub async fn last_screen(&self, id: &str) -> Result<Option<String>, RuntimeError> {
        let home = self.inner.home.clone();
        let id = id.to_owned();
        blocking(move || {
            if files::running(&home, &id)
                .map_err(|e| err(format!("{id}: {e}")))?
                .is_none()
            {
                return Ok(None);
            }
            let socket = files::socket_path(&home, &id);
            let (_, screen) = Conn::connect(&socket)
                .map_err(|e| err(format!("connect to {}: {e}", socket.display())))?;
            Ok(Some(screen))
        })
        .await
    }

    /// Closes the link of `id` without telling the holder.
    pub fn detach(&self, id: &str) {
        let link = self.inner.lock_links().remove(id);
        if let Some(link) = link {
            link.close();
        }
    }
}

/// Runs a blocking holder call on tokio's blocking pool; outside a tokio
/// runtime (the contract tests poll with testkit's `block_on`) it runs on
/// the caller's thread.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, RuntimeError> + Send + 'static,
) -> Result<T, RuntimeError> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle
            .spawn_blocking(f)
            .await
            .map_err(|e| err(format!("holder call panicked: {e}")))?,
        Err(_) => f(),
    }
}

impl Inner {
    fn lock_links(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, link::Link>> {
        self.links.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn handle(&self, id: &str, pid: u32) -> HolderHandle {
        HolderHandle {
            instance_id: id.to_owned(),
            process_id: Some(pid),
            socket_path: files::socket_path(&self.home, id).display().to_string(),
        }
    }

    fn start(&self, launch: &HolderLaunch) -> Result<Started, RuntimeError> {
        let id = &launch.instance_id;
        validate_id(id).map_err(err)?;
        if let Ok(Some(pid)) = files::running(&self.home, id) {
            return Err(err(format!("holder for {id} already running (pid {pid})")));
        }
        let mut child = Command::new(&self.agend)
            .arg("holder")
            .arg(id)
            .env_clear()
            .env("AGEND_HOME", &self.home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                err(format!(
                    "cannot run {} holder {id}: {e}",
                    self.agend.display()
                ))
            })?;
        let pid = child.id();
        let reap_id = id.clone();
        // Reaps the holder when it ends. Its stderr carries only what it
        // says before it points stderr at its own log (a refused start).
        let reaper = std::thread::Builder::new()
            .name(format!("holder-wait-{id}"))
            .spawn(move || {
                let mut said = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut said);
                }
                let status = child.wait();
                let said = said.trim();
                let said = if said.is_empty() {
                    String::new()
                } else {
                    format!("; it said: {said}")
                };
                match status {
                    Ok(status) => log::line(&format!(
                        "holder {reap_id} (pid {pid}) exited: {status}{said}"
                    )),
                    Err(e) => log::line(&format!("holder {reap_id} (pid {pid}): wait: {e}")),
                }
            });
        if let Err(e) = reaper {
            return Err(err(format!("cannot start the reaper thread: {e}")));
        }
        let (attached, generation) = self.attach(launch)?;
        Ok(Started {
            handle: self.handle(id, pid),
            attached,
            generation,
        })
    }

    fn attach(&self, launch: &HolderLaunch) -> Result<(Attached, u64), RuntimeError> {
        let id = launch.instance_id.clone();
        // The old link first: a new connection takes over the old one, which
        // would then reconnect and take it back.
        let old = self.lock_links().remove(&id);
        if let Some(old) = old {
            old.close();
        }
        let spawn = SpawnData {
            instance_id: id.clone(),
            program: launch.executable.clone(),
            args: launch.args.clone(),
            env: env::agent_env(&self.home, &id, launch.backend, self.daemon_env.clone()),
            working_directory: launch.working_directory.clone(),
        };
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst);
        let (link, attached) = link::open(
            self.home.clone(),
            id.clone(),
            generation,
            Some(spawn),
            Arc::clone(&self.sink),
        )
        .map_err(|e| err(format!("holder {id}: {e}")))?;
        self.lock_links().insert(id, link);
        Ok((attached, generation))
    }

    fn stop(&self, id: &str) -> Result<(), RuntimeError> {
        let link = self.lock_links().remove(id);
        if let Some(link) = link {
            link.close();
        }
        shutdown_holder(&self.home, id).map_err(err)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        let links = std::mem::take(&mut *self.lock_links());
        for (_, link) in links {
            link.close();
        }
    }
}

/// Sends `Shutdown` to the holder of `id` under `home`, if it runs, and
/// waits until its lock is released. Blocking; takes over whatever
/// connection the holder had.
pub fn shutdown_holder(home: &Path, id: &str) -> Result<(), String> {
    let running = |home: &Path| files::running(home, id).map_err(|e| format!("{id}: {e}"));
    if running(home)?.is_none() {
        return Ok(());
    }
    let socket = files::socket_path(home, id);
    let deadline = Instant::now() + link::CONNECT_WITHIN;
    let mut conn = loop {
        match Conn::connect(&socket) {
            Ok((conn, _)) => break conn,
            Err(_) if running(home)?.is_none() => return Ok(()),
            Err(e) if Instant::now() >= deadline => {
                return Err(format!("cannot connect to {}: {e}", socket.display()));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    conn.send(&HolderRequest::Shutdown)
        .map_err(|e| format!("send Shutdown to {id}: {e}"))?;
    let deadline = Instant::now() + STOP_WITHIN;
    while running(home)?.is_some() {
        if Instant::now() >= deadline {
            return Err(format!(
                "holder {id} still runs {}s after Shutdown",
                STOP_WITHIN.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

impl Runtime for HolderRuntime {
    type Error = RuntimeError;

    async fn start_holder(&self, launch: &HolderLaunch) -> Result<HolderHandle, RuntimeError> {
        self.start(launch).await.map(|started| started.handle)
    }

    async fn stop_holder(&self, instance_id: &str) -> Result<(), RuntimeError> {
        self.stop(instance_id).await
    }

    async fn recover_holders(&self) -> Result<Vec<HolderHandle>, RuntimeError> {
        self.recover()
    }
}
