//! Holder protocol server (P4): versioned JSON Lines over the holder's unix
//! socket, one client at a time.
//!
//! Connection rules:
//! - The first request must be `hello`; the reply is the selected version,
//!   then a `ScreenSnapshot`, then (if the agent already ended) `Exited`, then
//!   live `PtyBytes`. The snapshot and the byte stream come from the same
//!   locked state and the same outbound queue, so nothing is lost or repeated.
//! - A new client that completes `hello` takes over; the old connection is
//!   closed. A version mismatch gets `Error` and is closed; the holder keeps
//!   running.
//! - A client more than 1 MiB behind is disconnected (output side only); it
//!   reconnects for a fresh snapshot. The agent never waits on a slow client.
//!   A `Shutdown` it had already sent is still honoured; a client that was
//!   replaced by a newer one is ignored.
//! - Limits: `hello` within 10 s in total, request lines up to 1 MiB,
//!   `Resize` 1 to 1000 rows and columns (`invalid_size`).
//!
//! Threads (std only, no async runtime): accept, one reader + one writer per
//! connection, PTY reader, PTY writer (`pty`), agent waiter, and the caller's
//! thread, which watches for stop conditions and runs the stop sequence.
//!
//! Must NOT: exit when the daemon disconnects, or restart the agent.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, SyncSender, channel};
use std::time::{Duration, Instant};

use agend_core::protocol::holder::{
    ErrorData, ExitedData, HolderRequest, HolderResponse, PtyBytesData, ResizeData,
    ScreenSnapshotData, SelectedVersionData, SpawnData, SpawnedData, negotiate_version,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use parking_lot::{Condvar, Mutex, MutexGuard};
use portable_pty::{MasterPty, PtySize};

use crate::pty::{self, QueueError};
use crate::screen::{DEFAULT_COLUMNS, DEFAULT_ROWS, ReplySink, Screen};

/// Default for `Config::lag_limit`: outbound bytes a client may leave unread
/// before it is disconnected (P4).
pub const LAG_LIMIT: usize = 1 << 20;
/// Time the agent gets to end after SIGHUP before its group is SIGKILLed.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
/// With no agent running, exit after this long with no connection (P2).
pub const DEFAULT_IDLE_EXIT: Duration = Duration::from_secs(24 * 60 * 60);
/// A client must complete `hello` within this time (total, not per read).
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest request line; a longer one gets `request_too_large` and is closed.
pub const MAX_REQUEST_LINE: usize = 1 << 20;
/// Largest `Resize` accepted, in rows and in columns (`invalid_size` above).
pub const MAX_SCREEN_SIDE: u16 = 1000;
/// After the agent ends, how long to wait for its last output before `Exited`
/// (only reached when a leftover child keeps the PTY open, or under heavy load).
const OUTPUT_DRAIN: Duration = Duration::from_secs(2);

pub struct Config {
    pub instance_id: String,
    /// Deleting this directory stops the holder (P2 safety net).
    pub agend_home: PathBuf,
    pub idle_exit: Duration,
    /// Normally [`LAG_LIMIT`]; tests use a smaller value.
    pub lag_limit: usize,
}

/// Why `serve` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// A client sent `Shutdown`.
    Shutdown,
    /// `AGEND_HOME` no longer exists.
    HomeDeleted,
    /// No agent running and no connection for `idle_exit`.
    Idle,
}

struct Conn {
    id: u64,
    frames: Sender<Vec<u8>>,
    pending: Arc<AtomicUsize>,
    stream: UnixStream,
}

impl Conn {
    /// Replaced by a newer client: close both directions.
    fn close(&self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    /// Dropped for lagging: stop sending, but keep reading what the client
    /// already sent (on macOS `shutdown(Both)` discards unread input, which
    /// could lose a `Shutdown` sent just before).
    fn close_output(&self) {
        let _ = self.stream.shutdown(Shutdown::Write);
    }
}

struct Agent {
    pid: u32,
    master: Box<dyn MasterPty + Send>,
    input: SyncSender<Vec<u8>>,
}

struct State {
    screen: Screen,
    replies: ReplySink,
    conn: Option<Conn>,
    next_conn: u64,
    /// Id of the newest connection that completed `hello`; older ones were
    /// replaced (as opposed to dropped for lagging).
    latest_conn: u64,
    agent: Option<Agent>,
    exited: Option<ExitedData>,
    output_done: bool,
    /// Last time a connection opened or closed, or the agent ended.
    last_seen: Instant,
    stop: Option<Stop>,
}

struct Holder {
    lag_limit: usize,
    instance_id: String,
    state: Mutex<State>,
    changed: Condvar,
}

impl Holder {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock()
    }

    fn log(&self, message: &str) {
        log(&self.instance_id, message);
    }
}

/// One line in the holder log (stderr, which `run` points at `<id>.log`).
pub fn log(instance_id: &str, message: &str) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    eprintln!("{secs} holder {instance_id}: {message}");
}

/// Serves `listener` until a stop condition, then stops the agent (if it is
/// still running) and returns. The caller removes the socket and exits.
pub fn serve(listener: UnixListener, config: Config) -> Stop {
    let replies = ReplySink::default();
    let holder = Arc::new(Holder {
        lag_limit: config.lag_limit,
        instance_id: config.instance_id.clone(),
        state: Mutex::new(State {
            screen: Screen::new(DEFAULT_ROWS, DEFAULT_COLUMNS, replies.clone()),
            replies,
            conn: None,
            next_conn: 1,
            latest_conn: 0,
            agent: None,
            exited: None,
            output_done: false,
            last_seen: Instant::now(),
            stop: None,
        }),
        changed: Condvar::new(),
    });

    let accept = Arc::clone(&holder);
    spawn_thread("accept", move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let holder = Arc::clone(&accept);
                    spawn_thread("connection", move || connection(&holder, stream));
                }
                Err(e) => accept.log(&format!("accept failed: {e}")),
            }
        }
    });

    let tick = (config.idle_exit / 4).clamp(Duration::from_millis(50), Duration::from_secs(60));
    let mut state = holder.lock();
    let stop = loop {
        if let Some(stop) = state.stop {
            break stop;
        }
        if !config.agend_home.exists() {
            break Stop::HomeDeleted;
        }
        let agent_running = state.agent.is_some() && state.exited.is_none();
        if !agent_running && state.conn.is_none() && state.last_seen.elapsed() >= config.idle_exit {
            break Stop::Idle;
        }
        holder.changed.wait_for(&mut state, tick);
    };
    holder.log(&format!("stopping: {stop:?}"));
    stop_agent(&holder, state);
    stop
}

/// P7: SIGHUP the agent's process group (what closing the PTY delivers),
/// wait up to 5 s, then SIGKILL the group, then reap the agent. If the agent
/// already ended, its leftover children in the group are SIGKILLed right
/// away: the unreaped agent (see `exit`) keeps the group id from being reused.
fn stop_agent(holder: &Holder, mut state: MutexGuard<'_, State>) {
    let Some(agent) = state.agent.take() else {
        return;
    };
    let pid = agent.pid;
    // The agent is a session leader (portable-pty calls setsid), so its
    // process group id is its own pid; never 0 or 1, never negative.
    let pgid = libc::pid_t::try_from(pid).expect("pid fits pid_t");
    assert!(pgid > 1, "refusing to signal process group {pgid}");
    // Closing our master handles; the PTY reader thread still holds a
    // duplicate, so the hangup signal is sent explicitly.
    drop(agent);
    if state.exited.is_none() {
        signal_group(pgid, libc::SIGHUP);
        wait_exited(holder, &mut state, SHUTDOWN_GRACE);
    }
    signal_group(pgid, libc::SIGKILL);
    wait_exited(holder, &mut state, Duration::from_secs(2));
    crate::exit::reap(pid);
    holder.log(&format!("agent process group {pgid} stopped"));
}

fn wait_exited(holder: &Holder, state: &mut MutexGuard<'_, State>, limit: Duration) {
    let deadline = Instant::now() + limit;
    while state.exited.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        holder.changed.wait_for(state, left);
    }
}

fn signal_group(pgid: libc::pid_t, signal: libc::c_int) {
    assert!(pgid > 1, "refusing to signal process group {pgid}");
    // SAFETY: kill(2) on the negative of a process group id we created.
    unsafe { libc::kill(-pgid, signal) };
}

fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .expect("spawn holder thread");
}

fn frame(response: &HolderResponse) -> Vec<u8> {
    let mut line = serde_json::to_vec(response).expect("holder response serializes");
    line.push(b'\n');
    line
}

fn error(code: &str, message: impl Into<String>) -> HolderResponse {
    HolderResponse::Error {
        data: ErrorData {
            code: code.into(),
            message: message.into(),
        },
    }
}

/// Queues a frame for the current client, or drops the client if it lags.
fn push(holder: &Holder, state: &mut State, response: &HolderResponse) {
    let Some(conn) = &state.conn else {
        return;
    };
    let line = frame(response);
    let len = line.len();
    let lagging = conn.pending.load(Ordering::SeqCst) + len > holder.lag_limit;
    if !lagging {
        conn.pending.fetch_add(len, Ordering::SeqCst);
        if conn.frames.send(line).is_ok() {
            return;
        }
    } else {
        holder.log(&format!(
            "client fell more than {} bytes behind; disconnected",
            holder.lag_limit
        ));
    }
    conn.close_output();
    state.conn = None;
    state.last_seen = Instant::now();
    holder.changed.notify_all();
}

/// Reads one request line of at most [`MAX_REQUEST_LINE`] bytes. With a
/// deadline, the whole line must arrive before it (a slow trickle times out).
/// `Ok(false)` at end of stream.
fn read_line(
    reader: &mut BufReader<UnixStream>,
    line: &mut Vec<u8>,
    deadline: Option<Instant>,
) -> io::Result<bool> {
    line.clear();
    loop {
        if let Some(deadline) = deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            reader.get_ref().set_read_timeout(Some(left))?;
        }
        let buf = match reader.fill_buf() {
            Ok(buf) => buf,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buf.is_empty() {
            return Ok(false);
        }
        let (take, complete) = match buf.iter().position(|&b| b == b'\n') {
            Some(end) => (end + 1, true),
            None => (buf.len(), false),
        };
        line.extend_from_slice(&buf[..take]);
        reader.consume(take);
        if line.len() > MAX_REQUEST_LINE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request line over 1 MiB",
            ));
        }
        if complete {
            return Ok(true);
        }
    }
}

fn connection(holder: &Arc<Holder>, stream: UnixStream) {
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut line = Vec::new();
    let mut direct = &stream;
    match read_line(&mut reader, &mut line, Some(Instant::now() + HELLO_TIMEOUT)) {
        Ok(true) => {}
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            let _ = direct.write_all(&frame(&error("request_too_large", e.to_string())));
            return;
        }
        _ => return,
    }
    let hello = match serde_json::from_slice(&line) {
        Ok(HolderRequest::Hello { data }) => data,
        _ => {
            let _ = direct.write_all(&frame(&error(
                "expected_hello",
                "the first request must be hello",
            )));
            return;
        }
    };
    let version = match negotiate_version(&hello) {
        Ok(version) => version,
        Err(mismatch) => {
            holder.log(&mismatch.message());
            let _ = direct.write_all(&frame(&error("version_mismatch", mismatch.message())));
            return;
        }
    };
    let _ = stream.set_read_timeout(None);

    let (Ok(out), Ok(control)) = (stream.try_clone(), stream.try_clone()) else {
        return;
    };
    let (frames, queue) = channel::<Vec<u8>>();
    let pending = Arc::new(AtomicUsize::new(0));
    let unsent = Arc::clone(&pending);
    spawn_thread("connection-writer", move || {
        let mut out = out;
        for line in queue {
            if out.write_all(&line).is_err() {
                let _ = out.shutdown(Shutdown::Write);
                break;
            }
            unsent.fetch_sub(line.len(), Ordering::SeqCst);
        }
        let _ = out.shutdown(Shutdown::Write);
    });

    let id = {
        let mut state = holder.lock();
        let id = state.next_conn;
        state.next_conn += 1;
        if let Some(old) = state.conn.take() {
            old.close();
            holder.log("connection replaced by a new client");
        }
        state.latest_conn = id;
        state.conn = Some(Conn {
            id,
            frames,
            pending,
            stream: control,
        });
        state.last_seen = Instant::now();
        let greeting = [
            Some(HolderResponse::Hello {
                data: SelectedVersionData { selected: version },
            }),
            Some(snapshot(&state)),
            state
                .exited
                .clone()
                .map(|data| HolderResponse::Exited { data }),
        ];
        for response in greeting.iter().flatten() {
            push(holder, &mut state, response);
        }
        holder.changed.notify_all();
        id
    };

    loop {
        match read_line(&mut reader, &mut line, None) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                if e.kind() == io::ErrorKind::InvalidData {
                    let mut state = holder.lock();
                    if state.conn.as_ref().map(|c| c.id) == Some(id) {
                        push(
                            holder,
                            &mut state,
                            &error("request_too_large", e.to_string()),
                        );
                    }
                }
                break;
            }
        }
        let request = serde_json::from_slice::<HolderRequest>(&line);
        let mut state = holder.lock();
        if state.conn.as_ref().map(|c| c.id) != Some(id) {
            if state.latest_conn > id {
                return; // replaced by a newer client: it is in charge now
            }
            // Dropped for lagging: nothing can be replied, but a `Shutdown`
            // the client sent is still honoured.
            if matches!(request, Ok(HolderRequest::Shutdown)) {
                handle(holder, &mut state, HolderRequest::Shutdown);
            }
            continue;
        }
        let reply = match request {
            Ok(request) => handle(holder, &mut state, request),
            Err(e) => Some(error("bad_request", format!("cannot parse request: {e}"))),
        };
        if let Some(reply) = reply {
            push(holder, &mut state, &reply);
        }
    }

    let mut state = holder.lock();
    if state.conn.as_ref().map(|c| c.id) == Some(id) {
        // Dropping the queue lets the writer send what is left (for example
        // a final `Error`) and then close its side.
        state.conn = None;
        state.last_seen = Instant::now();
        holder.changed.notify_all();
    }
}

fn snapshot(state: &State) -> HolderResponse {
    HolderResponse::ScreenSnapshot {
        data: ScreenSnapshotData {
            screen: state.screen.text(),
        },
    }
}

/// Handles one request from the current client; returns its reply, if any.
/// Control keys and operator input succeed silently; failures reply `Error`.
fn handle(
    holder: &Arc<Holder>,
    state: &mut State,
    request: HolderRequest,
) -> Option<HolderResponse> {
    match request {
        HolderRequest::Hello { .. } => Some(error("unexpected_hello", "already said hello")),
        HolderRequest::Spawn { data } => Some(spawn_agent(holder, state, &data)),
        HolderRequest::Resize {
            data: ResizeData { rows, columns },
        } => {
            let valid = 1..=MAX_SCREEN_SIDE;
            if !valid.contains(&rows) || !valid.contains(&columns) {
                return Some(error(
                    "invalid_size",
                    format!("rows and columns must be 1 to {MAX_SCREEN_SIDE}"),
                ));
            }
            state.screen.resize(rows, columns);
            let resized = state.agent.as_ref().map(|agent| {
                agent.master.resize(PtySize {
                    rows,
                    cols: columns,
                    pixel_width: 0,
                    pixel_height: 0,
                })
            });
            match resized {
                Some(Err(e)) => Some(error("resize_failed", e.to_string())),
                _ => None,
            }
        }
        HolderRequest::SendControlKey { data } => match pty::control_key_bytes(data.key) {
            Some(bytes) => write_pty(state, bytes.to_vec()),
            None => Some(error(
                "unknown_control_key",
                "not a control key this holder knows; nothing was written",
            )),
        },
        HolderRequest::OperatorTerminalInput { data } => match BASE64.decode(&data.bytes_base64) {
            Ok(bytes) => write_pty(state, bytes),
            Err(e) => Some(error("bad_request", format!("bytes_base64: {e}"))),
        },
        HolderRequest::Snapshot => Some(snapshot(state)),
        HolderRequest::Shutdown => {
            holder.log("shutdown requested");
            state.stop = Some(Stop::Shutdown);
            holder.changed.notify_all();
            None
        }
        HolderRequest::Unknown => Some(error(
            "unsupported_request",
            "this holder does not know that request",
        )),
    }
}

fn write_pty(state: &State, bytes: Vec<u8>) -> Option<HolderResponse> {
    let Some(agent) = &state.agent else {
        return Some(error("not_spawned", "no agent has been spawned"));
    };
    if state.exited.is_some() {
        return Some(error("agent_exited", "the agent has exited"));
    }
    match pty::enqueue(&agent.input, bytes) {
        Ok(()) => None,
        Err(QueueError::Busy) => Some(error(
            "pty_busy",
            "the agent is not reading its input; nothing was written",
        )),
        Err(QueueError::Closed) => Some(error("agent_exited", "the agent's PTY is closed")),
    }
}

fn spawn_agent(holder: &Arc<Holder>, state: &mut State, data: &SpawnData) -> HolderResponse {
    if state.agent.is_some() {
        return error(
            "already_spawned",
            "this holder already ran its agent; it never starts another",
        );
    }
    if data.instance_id != holder.instance_id {
        return error(
            "instance_mismatch",
            format!(
                "this holder is for {}, not {}",
                holder.instance_id, data.instance_id
            ),
        );
    }
    let (rows, columns) = state.screen.size();
    let spawned = match pty::spawn(data, rows, columns) {
        Ok(spawned) => spawned,
        Err(message) => return error("spawn_failed", message),
    };
    let pid = spawned.pid;
    let input = pty::start_writer(spawned.writer);
    let _ = state.replies.set(input.clone());
    state.agent = Some(Agent {
        pid,
        master: spawned.master,
        input,
    });
    holder.log(&format!("agent {} started (pid {pid})", data.program));

    let reader = Arc::clone(holder);
    let mut pty_out = spawned.reader;
    spawn_thread("pty-reader", move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = match pty_out.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            let mut state = reader.lock();
            state.screen.process(&buf[..n]);
            let bytes = HolderResponse::PtyBytes {
                data: PtyBytesData {
                    bytes_base64: BASE64.encode(&buf[..n]),
                },
            };
            push(&reader, &mut state, &bytes);
            // Hand the lock to a waiting thread: a flooding agent must not
            // starve `Shutdown`, new clients or the stop checks.
            MutexGuard::unlock_fair(state);
        }
        reader.lock().output_done = true;
        reader.changed.notify_all();
    });

    let waiter = Arc::clone(holder);
    spawn_thread("agent-waiter", move || {
        let exited = crate::exit::wait(pid);
        let deadline = Instant::now() + OUTPUT_DRAIN;
        let mut state = waiter.lock();
        while !state.output_done {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            waiter.changed.wait_for(&mut state, left);
        }
        waiter.log(&format!(
            "agent exited: code={:?} signal={:?}",
            exited.code, exited.signal
        ));
        push(
            &waiter,
            &mut state,
            &HolderResponse::Exited {
                data: exited.clone(),
            },
        );
        state.exited = Some(exited);
        state.last_seen = Instant::now();
        waiter.changed.notify_all();
    });

    HolderResponse::Spawned {
        data: SpawnedData {
            instance_id: data.instance_id.clone(),
            process_id: Some(pid),
        },
    }
}
