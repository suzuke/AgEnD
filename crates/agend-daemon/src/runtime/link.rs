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
//! - Terminal (gate 8 P6): [`Link::terminal`] sends `Snapshot` on this
//!   connection; the holder answers in order with the stream, so the
//!   subscriber gets that screen and then every `PtyBytes` read after it
//!   (per-instance broadcast of [`TERMINAL_CHUNKS`]), nothing twice or lost.
//! - Operator input (gate 11 B P6): [`input`] sends
//!   `OperatorTerminalInput` on this connection. Writes (input, `Snapshot`)
//!   take the link's own write lock for one whole line (never the links
//!   table's), and give up after [`WRITE_WITHIN`] without progress (a holder
//!   that stops reading), closing the connection so the link reconnects. The holder answers only a
//!   refusal (`pty_busy`, `agent_exited`, …); it is logged and dropped, not
//!   passed back to the client (the protocol has no id to match it with).
//!
//! Must NOT: send `Shutdown`, or report anything after [`Link::close`].

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agend_core::protocol::holder::{
    ExitedData, HolderRequest, HolderResponse, OperatorTerminalInputData, SpawnData,
};
use tokio::sync::{broadcast, oneshot};

use super::client::Conn;
use super::files;
use crate::log;

/// How long the first connection is retried (gate 6 P3).
pub const CONNECT_WITHIN: Duration = Duration::from_secs(5);
const CONNECT_EVERY: Duration = Duration::from_millis(50);
/// Wait before connecting again after the connection ended (gate 6 P4).
pub const RECONNECT_AFTER: Duration = Duration::from_secs(1);
const SPAWN_REPLY_WITHIN: Duration = Duration::from_secs(10);
/// Writes the operator's bytes (base64) to the agent's PTY through the
/// holder, on the link's connection (`Link::input_slot`); false when the
/// request cannot be sent within [`WRITE_WITHIN`].
pub fn input(writer: &Writer, bytes_base64: String) -> bool {
    send(writer, &input_line(bytes_base64))
}

/// The holder request line (newline included) that carries `bytes_base64`;
/// the daemon refuses input whose line is over the holder's
/// `MAX_REQUEST_LINE` before sending it.
pub fn input_line(bytes_base64: String) -> Vec<u8> {
    let request = HolderRequest::OperatorTerminalInput {
        data: OperatorTerminalInputData { bytes_base64 },
    };
    let mut line = serde_json::to_vec(&request).unwrap_or_default();
    line.push(b'\n');
    line
}

/// Sends `Snapshot` (the second half of [`Link::terminal`]).
pub fn send_snapshot(writer: &Writer) -> bool {
    match serde_json::to_vec(&HolderRequest::Snapshot) {
        Ok(mut line) => {
            line.push(b'\n');
            send(writer, &line)
        }
        Err(_) => false,
    }
}

/// A write to the holder that makes no progress for this long gives up
/// (the holder stopped reading; as `server::WRITE_TIMEOUT`).
pub const WRITE_WITHIN: Duration = Duration::from_secs(5);
/// PTY chunks a terminal subscriber may fall behind before it is dropped.
pub const TERMINAL_CHUNKS: usize = 256;

/// A terminal subscription: the screen, then the base64 PTY chunks after it.
pub type TerminalFeed = (String, broadcast::Receiver<String>);

/// Terminal subscribers of one link.
struct Terminal {
    /// Waiting for the answer to their `Snapshot`.
    pending: Vec<oneshot::Sender<TerminalFeed>>,
    live: broadcast::Sender<String>,
}

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
    /// The holder started the agent now; its pid (gate 7 P2: the codex
    /// sweep's process group).
    Spawned { agent_pid: Option<u32> },
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
    /// Held for the whole write of one request line.
    write: Arc<Mutex<()>>,
    terminal: Arc<Mutex<Terminal>>,
    wake: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Link {
    /// Ends the connection and the thread; the holder keeps running and is
    /// not told anything (no `Shutdown`).
    pub fn close(mut self) {
        self.stop();
    }

    /// Asks the holder for its screen on this connection; the answer is the
    /// screen and a receiver of the PTY chunks after it. `None` when the
    /// request cannot be sent. May wait for this link's write lock.
    pub fn terminal(&self) -> Option<oneshot::Receiver<TerminalFeed>> {
        let (rx, writer) = self.terminal_request();
        send_snapshot(&writer).then_some(rx)
    }

    /// The first, non-blocking half of [`Link::terminal`], for a caller that
    /// holds a lock others need (the daemon's links table): registers the
    /// waiting subscriber and returns where to send `Snapshot`
    /// ([`send_snapshot`]) after that lock is released.
    pub fn terminal_request(&self) -> (oneshot::Receiver<TerminalFeed>, Writer) {
        let (tx, rx) = oneshot::channel();
        // The pending entry goes in before the request is sent, so the
        // reader thread finds it when the answer arrives.
        lock(&self.terminal).pending.push(tx);
        (rx, self.writer())
    }

    /// A link on `stream` with no reader thread (tests of the callers).
    #[cfg(test)]
    pub(crate) fn on_stream(stream: UnixStream) -> Link {
        Link {
            stopping: Arc::new(AtomicBool::new(false)),
            stream: Arc::new(Mutex::new(Some(stream))),
            write: Arc::new(Mutex::new(())),
            terminal: Arc::new(Mutex::new(Terminal {
                pending: Vec::new(),
                live: broadcast::channel(TERMINAL_CHUNKS).0,
            })),
            wake: None,
            thread: None,
        }
    }

    /// Where [`input`] writes: the caller can drop its lock on the links
    /// before writing.
    pub fn writer(&self) -> Writer {
        Writer {
            stream: Arc::clone(&self.stream),
            write: Arc::clone(&self.write),
        }
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

/// The writing side of a link: its current connection and its write lock.
#[derive(Clone)]
pub struct Writer {
    stream: Arc<Mutex<Option<UnixStream>>>,
    write: Arc<Mutex<()>>,
}

/// Writes one request line on the link's connection. The link's write lock
/// is held for the whole line, so lines from several callers never
/// interleave; the slot's lock (and the daemon's links table) is not held
/// meanwhile, so a slow holder only holds up writes to itself. A write
/// that makes no progress for [`WRITE_WITHIN`] gives up and shuts the
/// connection down: part of the line may already be at the holder, so the
/// connection cannot be used again; the link's reader thread reconnects
/// (or reports the holder gone). False when there is no connection or the
/// write fails.
pub fn send_line(writer: &Writer, line: &[u8]) -> bool {
    send(writer, line)
}

fn send(writer: &Writer, line: &[u8]) -> bool {
    let _whole_line = lock(&writer.write);
    let Some(mut stream) = lock(&writer.stream)
        .as_ref()
        .and_then(|s| s.try_clone().ok())
    else {
        return false;
    };
    let sent =
        stream.set_write_timeout(Some(WRITE_WITHIN)).is_ok() && stream.write_all(line).is_ok();
    if !sent {
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }
    sent
}

struct Worker {
    home: PathBuf,
    id: String,
    generation: u64,
    sink: EventSink,
    stopping: Arc<AtomicBool>,
    stream: Arc<Mutex<Option<UnixStream>>>,
    terminal: Arc<Mutex<Terminal>>,
    wake: Receiver<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
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
    let terminal = Arc::new(Mutex::new(Terminal {
        pending: Vec::new(),
        live: broadcast::channel(TERMINAL_CHUNKS).0,
    }));
    let (wake_tx, wake) = mpsc::channel();
    let (first_tx, first) = mpsc::sync_channel(1);
    let worker = Worker {
        home,
        id: id.clone(),
        generation,
        sink,
        stopping: Arc::clone(&stopping),
        stream: Arc::clone(&stream),
        terminal: Arc::clone(&terminal),
        wake,
    };
    let thread = std::thread::Builder::new()
        .name(format!("holder-link-{id}"))
        .spawn(move || worker.run(spawn, first_tx))
        .map_err(|e| format!("cannot start the link thread: {e}"))?;
    let link = Link {
        stopping,
        stream,
        write: Arc::new(Mutex::new(())),
        terminal,
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
                Ok(HolderResponse::Spawned { data }) => {
                    return Ok(SpawnOutcome::Spawned {
                        agent_pid: data.process_id,
                    });
                }
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
                Ok(Some(HolderResponse::ScreenSnapshot { data })) => {
                    let mut terminal = lock(&self.terminal);
                    for waiting in std::mem::take(&mut terminal.pending) {
                        let _ = waiting.send((data.screen.clone(), terminal.live.subscribe()));
                    }
                }
                Ok(Some(HolderResponse::PtyBytes { data })) => {
                    // No subscriber is not an error.
                    let _ = lock(&self.terminal).live.send(data.bytes_base64);
                }
                // The daemon sends nothing else on this connection that
                // can be refused (gate 11 B P6: logged, not passed back).
                Ok(Some(HolderResponse::Error { data })) => log::line(&format!(
                    "{}: operator input dropped: {}",
                    self.id, data.code
                )),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn writer_on(stream: UnixStream) -> Writer {
        Writer {
            stream: Arc::new(Mutex::new(Some(stream))),
            write: Arc::new(Mutex::new(())),
        }
    }

    /// A holder that stops reading must not stall the caller (a tokio
    /// worker, and whoever waits for the link's locks): with its socket
    /// buffer full, an input line gives up after [`WRITE_WITHIN`].
    #[test]
    fn a_write_to_a_holder_that_does_not_read_gives_up() {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let mut filler = ours.try_clone().unwrap();
        filler.set_nonblocking(true).unwrap();
        while filler.write(&[b'x'; 4096]).is_ok() {}
        filler.set_nonblocking(false).unwrap();
        let writer = writer_on(ours);
        let (tx, rx) = mpsc::channel();
        let w = writer.clone();
        let started = Instant::now();
        std::thread::spawn(move || {
            let _ = tx.send(input(&w, "aGVsbG8=".into()));
        });
        let sent = rx.recv_timeout(WRITE_WITHIN * 2);
        let took = started.elapsed();
        assert_eq!(sent, Ok(false), "the write never gave up");
        assert!(
            took >= WRITE_WITHIN - Duration::from_millis(100),
            "{took:?}"
        );
        // The connection slot is free while the write waits and after.
        assert!(writer.stream.try_lock().is_ok());
    }

    /// Lines written by several threads at once reach the holder whole
    /// (round-2 verifier: 193 of 800 12 KB lines were interleaved).
    #[test]
    fn concurrent_writers_never_interleave_lines() {
        use std::io::{BufRead, BufReader};
        let (ours, peer) = UnixStream::pair().unwrap();
        let writer = writer_on(ours);
        let reader = std::thread::spawn(move || {
            let (mut good, mut bad) = (0, 0);
            for line in BufReader::new(peer).lines() {
                match serde_json::from_str::<HolderRequest>(&line.unwrap()) {
                    Ok(HolderRequest::OperatorTerminalInput { .. }) => good += 1,
                    _ => bad += 1,
                }
            }
            (good, bad)
        });
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let w = writer.clone();
                std::thread::spawn(move || {
                    let payload = char::from(b'a' + t).to_string().repeat(12_000);
                    for _ in 0..200 {
                        assert!(input(&w, payload.clone()));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        if let Some(s) = lock(&writer.stream).take() {
            let _ = s.shutdown(std::net::Shutdown::Write);
        }
        assert_eq!(reader.join().unwrap(), (800, 0));
    }

    /// A write that timed out may have left half a line on the holder's
    /// side: the connection is closed (the link reconnects), and the next
    /// request on the new connection arrives whole.
    #[test]
    fn a_timed_out_write_closes_the_connection_and_the_next_one_is_clean() {
        use std::io::{BufRead, BufReader, Read};
        let (ours, mut peer) = UnixStream::pair().unwrap();
        let writer = writer_on(ours);
        let payload = "p".repeat(4 << 20);
        assert!(!input(&writer, payload), "the peer never reads");
        // The old connection is closed: the peer reads what arrived, then EOF.
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut rest = Vec::new();
        peer.read_to_end(&mut rest).expect("EOF, not a timeout");
        assert!(!rest.ends_with(b"\n"), "half a line was left behind");
        // The link's reader thread reconnects and publishes a new one.
        let (fresh, peer) = UnixStream::pair().unwrap();
        *lock(&writer.stream) = Some(fresh);
        assert!(input(&writer, "aGk=".into()));
        let mut line = String::new();
        BufReader::new(peer).read_line(&mut line).unwrap();
        assert!(matches!(
            serde_json::from_str::<HolderRequest>(&line),
            Ok(HolderRequest::OperatorTerminalInput { .. })
        ));
    }
}
