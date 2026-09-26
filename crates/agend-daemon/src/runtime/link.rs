//! One long-lived connection per holder (gate 6 P4), on its own std thread:
//! the holder protocol is blocking I/O and the connection lives as long as
//! the holder, so it gets a thread rather than a `spawn_blocking` slot.
//!
//! - First connection: retried every 50 ms for up to 5 s (a just-started
//!   holder binds its socket shortly after it starts). Then, if asked, it
//!   sends `Spawn` and waits for `Spawned` or `already_spawned` (gate 4
//!   G10: a re-sent `Spawn` changes nothing).
//! - Afterwards it reads until the connection ends, reporting `Exited` as
//!   [`HolderEvent::AgentExited`]. When the connection ends it waits 1 s
//!   and connects again while the holder's lock is held (another client
//!   took the connection over, gate 4 P4); once the lock is free it reports
//!   [`HolderEvent::HolderGone`] and ends: "connection closed + lock
//!   released" is how a holder that is not the daemon's child is seen to die.
//!
//! Must NOT: send `Shutdown`, or report anything after [`Link::close`].

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agend_core::protocol::holder::{ExitedData, HolderRequest, HolderResponse, SpawnData};

use super::client::Conn;
use super::files;

/// How long the first connection is retried (gate 6 P3).
pub const CONNECT_WITHIN: Duration = Duration::from_secs(5);
const CONNECT_EVERY: Duration = Duration::from_millis(50);
/// Wait before connecting again after the connection ended (gate 6 P4).
pub const RECONNECT_AFTER: Duration = Duration::from_secs(1);
const SPAWN_REPLY_WITHIN: Duration = Duration::from_secs(10);

/// What a link reports about its holder. `generation` tells a link's events
/// apart from those of an earlier link of the same instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HolderEvent {
    /// The agent ended; the holder still runs and keeps its last screen.
    AgentExited {
        id: String,
        generation: u64,
        exited: ExitedData,
    },
    /// The holder is gone: its connection ended and its lock is free.
    HolderGone { id: String, generation: u64 },
}

/// Receives a link's events (from the link's thread).
pub type EventSink = Arc<dyn Fn(HolderEvent) + Send + Sync>;

/// What the `Spawn` sent on the first connection did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnOutcome {
    Spawned,
    /// The holder already ran its agent (gate 4 G10); nothing changed.
    AlreadySpawned,
}

/// What the first connection saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    /// The holder's screen when the connection was made (no byte replay).
    pub screen: String,
    pub spawn: Option<SpawnOutcome>,
}

pub struct Link {
    stopping: Arc<AtomicBool>,
    stream: Arc<Mutex<Option<UnixStream>>>,
    wake: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Link {
    /// Ends the connection and the thread; the holder keeps running and is
    /// not told anything (no `Shutdown`).
    pub fn close(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(stream) = self.stream.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.wake.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Worker {
    home: PathBuf,
    id: String,
    generation: u64,
    sink: EventSink,
    stopping: Arc<AtomicBool>,
    stream: Arc<Mutex<Option<UnixStream>>>,
    wake: Receiver<()>,
}

/// Connects to the holder of `id` (sending `spawn` first if given) and keeps
/// the connection on a new thread. Returns once the first connection (and
/// the `Spawn` reply) is done.
pub fn open(
    home: PathBuf,
    id: String,
    generation: u64,
    spawn: Option<SpawnData>,
    sink: EventSink,
) -> Result<(Link, Attached), String> {
    let stopping = Arc::new(AtomicBool::new(false));
    let stream = Arc::new(Mutex::new(None));
    let (wake_tx, wake) = mpsc::channel();
    let (first_tx, first) = mpsc::sync_channel(1);
    let worker = Worker {
        home,
        id: id.clone(),
        generation,
        sink,
        stopping: Arc::clone(&stopping),
        stream: Arc::clone(&stream),
        wake,
    };
    let thread = std::thread::Builder::new()
        .name(format!("holder-link-{id}"))
        .spawn(move || worker.run(spawn, first_tx))
        .map_err(|e| format!("cannot start the link thread: {e}"))?;
    let link = Link {
        stopping,
        stream,
        wake: Some(wake_tx),
        thread: Some(thread),
    };
    match first.recv() {
        Ok(Ok(attached)) => Ok((link, attached)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("the link thread ended".into()),
    }
}

impl Worker {
    /// Sleeps `d`; true when the link is being closed.
    fn pause(&self, d: Duration) -> bool {
        match self.wake.recv_timeout(d) {
            Err(RecvTimeoutError::Timeout) => self.stopping.load(Ordering::SeqCst),
            _ => true,
        }
    }

    /// Makes `conn` the one `close` shuts down; false if closing already.
    fn publish(&self, conn: &Conn) -> bool {
        let mut slot = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        if self.stopping.load(Ordering::SeqCst) {
            return false;
        }
        *slot = conn.stream().ok();
        true
    }

    fn run(self, spawn: Option<SpawnData>, first: mpsc::SyncSender<Result<Attached, String>>) {
        let socket = files::socket_path(&self.home, &self.id);
        let deadline = Instant::now() + CONNECT_WITHIN;
        let (mut conn, screen) = loop {
            match Conn::connect(&socket) {
                Ok(connected) => break connected,
                Err(e) if Instant::now() >= deadline => {
                    let _ = first.send(Err(format!(
                        "cannot connect to {} within {}s: {e}",
                        socket.display(),
                        CONNECT_WITHIN.as_secs()
                    )));
                    return;
                }
                Err(_) => {
                    if self.pause(CONNECT_EVERY) {
                        let _ = first.send(Err("closed".into()));
                        return;
                    }
                }
            }
        };
        if !self.publish(&conn) {
            let _ = first.send(Err("closed".into()));
            return;
        }
        let outcome = match spawn {
            Some(data) => self.spawn(&mut conn, data).map(Some),
            None => Ok(None),
        };
        let failed = outcome.is_err();
        let _ = first.send(outcome.map(|spawn| Attached { screen, spawn }));
        if failed {
            return;
        }
        loop {
            self.read_until_closed(&mut conn);
            match self.reconnect(&socket) {
                Some(next) => conn = next,
                None => return,
            }
        }
    }

    fn spawn(&self, conn: &mut Conn, data: SpawnData) -> Result<SpawnOutcome, String> {
        conn.send(&HolderRequest::Spawn { data })
            .map_err(|e| format!("send Spawn: {e}"))?;
        loop {
            match conn.recv_within(SPAWN_REPLY_WITHIN) {
                Ok(HolderResponse::Spawned { .. }) => return Ok(SpawnOutcome::Spawned),
                Ok(HolderResponse::Error { data }) if data.code == "already_spawned" => {
                    return Ok(SpawnOutcome::AlreadySpawned);
                }
                Ok(HolderResponse::Error { data }) => {
                    return Err(format!("Spawn refused: {}: {}", data.code, data.message));
                }
                // An agent that ended before this connection: the holder
                // sends `Exited` right after the screen.
                Ok(HolderResponse::Exited { data }) => self.exited(data),
                Ok(_) => {}
                Err(e) => return Err(format!("no reply to Spawn: {e}")),
            }
        }
    }

    fn exited(&self, exited: ExitedData) {
        if !self.stopping.load(Ordering::SeqCst) {
            (self.sink)(HolderEvent::AgentExited {
                id: self.id.clone(),
                generation: self.generation,
                exited,
            });
        }
    }

    fn read_until_closed(&self, conn: &mut Conn) {
        loop {
            match conn.recv(None) {
                Ok(Some(HolderResponse::Exited { data })) => self.exited(data),
                Ok(_) => {}
                Err(_) => return,
            }
        }
    }

    /// After the connection ended: `Some` new connection while the holder
    /// runs; `None` once it is gone (reported) or the link is closing.
    fn reconnect(&self, socket: &std::path::Path) -> Option<Conn> {
        loop {
            if self.pause(RECONNECT_AFTER) {
                return None;
            }
            if matches!(files::running(&self.home, &self.id), Ok(None)) {
                if !self.stopping.load(Ordering::SeqCst) {
                    (self.sink)(HolderEvent::HolderGone {
                        id: self.id.clone(),
                        generation: self.generation,
                    });
                }
                return None;
            }
            if let Ok((conn, _)) = Conn::connect(socket) {
                if !self.publish(&conn) {
                    return None;
                }
                return Some(conn);
            }
        }
    }
}
