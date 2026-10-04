//! Protocol server (gate 8 P1, P8): client protocol 1.1 as JSON Lines on the
//! daemon's unix socket `$AGEND_HOME/run/daemon.sock` (a WebSocket listener
//! is added only when a GUI needs it).
//!
//! - [`bind`]: replaces a stale socket file (the caller holds `agend.db`'s
//!   lock, so no other daemon owns it), binds, and makes the socket 0600
//!   (`run/` is 0700). The daemon binds only after its boot plan, so a
//!   client that connects always sees the complete fleet.
//! - One task per connection on the daemon's tokio runtime. The first line
//!   must be `hello` (else `hello_required` and close); no shared major:
//!   `version_mismatch` and close. The `caller` of `hello` is the
//!   connection's identity for the handlers.
//! - Events: after `subscribe_events`, the backlog, then the fleet's
//!   broadcast. A subscriber that falls more than 1024 events behind gets
//!   `event_gap` and is closed; it reconnects and fetches the fleet view.
//! - Terminal: after `subscribe_terminal`, the screen, then PTY chunks as
//!   `terminal_bytes`; falling behind closes the connection too; a terminal
//!   that ends gets `no_terminal` (the connection stays). A new
//!   `subscribe_terminal` drops the connection's old stream first, so one
//!   that fails leaves no stream (gate 11 B P1).
//! - Every write has [`WRITE_TIMEOUT`]: a client that does not read at all
//!   fills the socket buffer and is closed (it gets no `event_gap`).
//! - [`Server::stop`]: stops accepting, removes the socket file, closes
//!   every connection.
//! - Gate 9: the `hello` reply names the daemon (version, pid, boot id);
//!   an accepted restart is answered first and only then handed to the
//!   supervisor.
//!
//! - Client 1.4 terminal views and ordered controls run through `TerminalHub`,
//!   separately from the socket reader. A dead socket scope releases control;
//!   full frames include their envelope in the 8 MiB line limit.
//!
//! Must NOT: contain command logic (that is `handlers`).

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::terminal_hub::{ReplyScope, TerminalHub, ViewStream, reject};
use agend_core::protocol::client::{
    ClientRequest, ClientResponse, ErrorData, EventData, MAX_LINE_BYTES, MAX_MESSAGE_BYTES,
    TerminalBytesData, V1_4, V1_5, error_code,
};
use agend_core::protocol::terminal::MAX_FRAME_LINE;
use agend_core::protocol::{ProtocolVersion, negotiate};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};

use crate::handlers::{self, Context, Outcome, error};
use crate::log;

/// A write that makes no progress for this long closes the connection.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest socket path (macOS `sun_path` holds 104 bytes), as for holders.
pub const MAX_SOCKET_PATH: usize = 100;

/// `Err` with the message to print when `socket` is too long to bind.
pub fn check_socket_len(socket: &Path) -> Result<(), String> {
    if socket.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(format!(
            "socket path too long: {} ({} bytes, max {MAX_SOCKET_PATH} bytes); use a shorter AGEND_HOME",
            socket.display(),
            socket.as_os_str().len()
        ));
    }
    Ok(())
}

/// Replaces a leftover socket file, binds `socket` and makes it 0600. Must
/// run inside the tokio runtime.
pub fn bind(socket: &Path) -> io::Result<UnixListener> {
    match fs::remove_file(socket) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// A running server; [`Server::stop`] ends it.
pub struct Server {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Server {
    /// Serves connections from `listener` (bound at `socket`) until stopped.
    pub fn start(listener: UnixListener, socket: PathBuf, ctx: Arc<Context>) -> Server {
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(accept_loop(listener, socket, ctx, stopped));
        Server { stop, task }
    }

    /// Stops accepting, removes the socket file, closes every connection,
    /// and returns once all of that is done.
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

async fn accept_loop(
    listener: UnixListener,
    socket: PathBuf,
    ctx: Arc<Context>,
    mut stopped: watch::Receiver<bool>,
) {
    let hub =
        TerminalHub::with_codex_driver(ctx.runtime.clone(), ctx.fleet.clone(), ctx.codex.clone());
    let claude = Arc::new(crate::claude_bridge::ClaudeBridge::default());
    let ingest = tokio::spawn(crate::ingest::run(
        ctx.store.home().to_owned(),
        ctx.clone(),
        claude.clone(),
    ));
    let mut connections = JoinSet::new();
    let next = AtomicU64::new(1);
    loop {
        tokio::select! {
            _ = stopped.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let number = next.fetch_add(1, Ordering::Relaxed);
                    connections.spawn(connection(stream, Arc::clone(&ctx), hub.clone(), number, claude.clone()));
                }
                Err(e) => {
                    log::line(&format!("client socket: accept failed: {e}"));
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    ingest.abort();
    let _ = ingest.await;
    drop(listener);
    let _ = fs::remove_file(&socket);
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    hub.stop();
}

enum Line {
    Text(Vec<u8>),
    /// Longer than [`MAX_LINE_BYTES`]: the rest is not read.
    TooLong,
    End,
}

/// The next line (without its newline), reading at most
/// [`MAX_LINE_BYTES`] of it (gate 9 L17). Cancel safe: what was read of a
/// line waits in `partial` for the next call.
async fn read_line(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
    partial: &mut Vec<u8>,
) -> io::Result<Line> {
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(Line::End);
        }
        let (take, done) = match available.iter().position(|&b| b == b'\n') {
            Some(end) => (end, true),
            None => (available.len(), false),
        };
        if partial.len() + take > MAX_LINE_BYTES {
            return Ok(Line::TooLong);
        }
        partial.extend_from_slice(&available[..take]);
        reader.consume(if done { take + 1 } else { take });
        if done {
            return Ok(Line::Text(std::mem::take(partial)));
        }
    }
}

/// Next item of an optional receiver; never ready without one.
async fn next<T: Clone>(
    receiver: &mut Option<broadcast::Receiver<T>>,
) -> Result<T, broadcast::error::RecvError> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

struct Client {
    number: u64,
    caller: Option<String>,
    writer: OwnedWriteHalf,
}

impl Client {
    fn name(&self) -> String {
        match &self.caller {
            Some(caller) => format!("client #{} ({caller})", self.number),
            None => format!("client #{} (operator)", self.number),
        }
    }

    /// Writes one line; false when the connection must close (the client
    /// is gone, or [`WRITE_TIMEOUT`] passed without progress).
    async fn send(&mut self, response: &ClientResponse) -> bool {
        let full_id = match response {
            ClientResponse::TerminalFrame { data } => Some(data.request_id.clone()),
            ClientResponse::TerminalControlAck { data } => Some(data.request_id.clone()),
            _ => None,
        };
        let mut rejected = false;
        let mut line = if let Some(id) = full_id {
            let mut bounded = FrameLine(Vec::new());
            if serde_json::to_writer(&mut bounded, response).is_err() {
                // The client envelope also counts. Send no partial frame or
                // grant; EOF invalidates the view and releases any real owner.
                rejected = true;
                serde_json::to_vec(&error(
                    Some(id),
                    "frame_too_large",
                    "terminal frame exceeds 8 MiB; nothing was truncated; subscribe again",
                ))
                .unwrap()
            } else {
                bounded.0
            }
        } else {
            match serde_json::to_vec(response) {
                Ok(line) => line,
                Err(e) => {
                    log::line(&format!("{}: cannot encode a response: {e}", self.name()));
                    return false;
                }
            }
        };
        line.push(b'\n');
        match tokio::time::timeout(WRITE_TIMEOUT, self.writer.write_all(&line)).await {
            Ok(Ok(())) => !rejected,
            Ok(Err(_)) => false,
            Err(_) => {
                log::line(&format!(
                    "{}: no write progress for {} s; closed",
                    self.name(),
                    WRITE_TIMEOUT.as_secs()
                ));
                false
            }
        }
    }
}

async fn connection(
    stream: UnixStream,
    ctx: Arc<Context>,
    hub: TerminalHub,
    number: u64,
    claude: Arc<crate::claude_bridge::ClaudeBridge>,
) {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut partial = Vec::new();
    let mut client = Client {
        number,
        caller: None,
        writer,
    };
    let mut negotiated = false;
    let mut selected_version = ProtocolVersion::new(0, 0);
    let alive = Arc::new(AtomicBool::new(true));
    let _guard = ConnectionGuard(alive.clone());
    let (reply_sender, mut replies) = mpsc::channel(8);
    let scope = ReplyScope {
        client: number,
        alive: alive.clone(),
        replies: reply_sender,
    };
    let mut full: Option<ViewStream> = None;
    let mut scope_tick = tokio::time::interval(Duration::from_millis(10));
    let mut events: Option<broadcast::Receiver<EventData>> = None;
    let mut terminal: Option<broadcast::Receiver<String>> = None;
    let mut terminal_of = String::new();
    loop {
        tokio::select! {
            line = read_line(&mut reader, &mut partial) => {
                let line = match line {
                    Ok(Line::Text(line)) => line,
                    Ok(Line::TooLong) => {
                        log::line(&format!(
                            "{}: a line longer than {MAX_LINE_BYTES} bytes; closed",
                            client.name()
                        ));
                        let message = format!(
                            "a protocol line is limited to {MAX_LINE_BYTES} bytes (a message body to {MAX_MESSAGE_BYTES}); the connection is closed"
                        );
                        client.send(&error(None, error_code::INVALID_REQUEST, message)).await;
                        return;
                    }
                    Ok(Line::End) | Err(_) => return,
                };
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let request: ClientRequest = match serde_json::from_slice(&line) {
                    Ok(request) => request,
                    Err(e) if !negotiated => {
                        let message = format!("the first message must be hello (invalid JSON line: {e})");
                        client.send(&error(None, error_code::HELLO_REQUIRED, message)).await;
                        return;
                    }
                    Err(e) => {
                        let message = format!("invalid JSON line: {e}");
                        if !client.send(&error(None, error_code::INVALID_REQUEST, message)).await {
                            return;
                        }
                        continue;
                    }
                };
                if !negotiated {
                    let ClientRequest::Hello { data } = request else {
                        let reply = error(None, error_code::HELLO_REQUIRED, "the first message must be hello");
                        client.send(&reply).await;
                        return;
                    };
                    match negotiate("client", &[V1_5], &data.supported) {
                        Ok(selected) => {
                            negotiated = true;
                            selected_version = selected;
                            client.caller = data.caller;
                            let reply = ClientResponse::Hello { data: ctx.hello(selected) };
                            if !client.send(&reply).await {
                                return;
                            }
                        }
                        Err(mismatch) => {
                            let reply = error(None, error_code::VERSION_MISMATCH, mismatch.message());
                            client.send(&reply).await;
                            return;
                        }
                    }
                    continue;
                }
                if matches!(request, ClientRequest::SubscribeTerminal { .. }) {
                    terminal = None;
                    full = None;
                }
                if matches!(request, ClientRequest::SubscribeTerminalFrames { .. }) {
                    terminal = None;
                    full = None;
                }
                if let Some(result) = full_request(&hub, &scope, &mut full, client.caller.as_deref(), selected_version, request.clone(), line.len() + 1) {
                    if let Err(data) = result && !client.send(&ClientResponse::Error { data }).await { return; }
                    continue;
                }
                if let ClientRequest::Claude { data } = request {
                    let reply = claude.handle(&ctx, client.caller.as_deref(), selected_version, data).await;
                    if !client.send(&reply).await { return; }
                    continue;
                }
                match handlers::handle(&ctx, client.caller.as_deref(), request).await {
                    Outcome::Nothing => {}
                    Outcome::TerminalInput { instance_id, line } => {
                        if let Err(data) = hub.legacy(scope.clone(), &instance_id, line) && !client.send(&ClientResponse::Error { data }).await { return; }
                    }
                    Outcome::Reply(reply) => {
                        if !client.send(&reply).await {
                            return;
                        }
                    }
                    Outcome::Events(subscription) => {
                        for data in subscription.backlog {
                            if !client.send(&ClientResponse::Event { data }).await {
                                return;
                            }
                        }
                        events = Some(subscription.live);
                    }
                    Outcome::Restart { reply, binary } => {
                        // The reply first; then the supervisor stops the
                        // daemon, which closes this connection (the CLI
                        // waits for that EOF, gate 9 P7).
                        client.send(&reply).await;
                        let _ = ctx.supervisor.send(crate::supervisor::Event::Exec(binary));
                    }
                    Outcome::Terminal { snapshot, live } => {
                        if let ClientResponse::TerminalSnapshot { data } = &snapshot {
                            terminal_of = data.instance_id.clone();
                        }
                        if !client.send(&snapshot).await {
                            return;
                        }
                        terminal = live;
                    }
                }
            }
            _ = scope_tick.tick() => { if !alive.load(Ordering::SeqCst) { return; } },
            reply = replies.recv() => {
                if let Some(reply) = reply && !client.send(&reply).await { return; }
            },
            frame = next_frame(&mut full) => match frame {
                Ok(Some(frame)) => { if !client.send(&frame).await { return; } },
                Ok(None) => {},
                Err(_) => { full = None; },
            },
            event = next(&mut events) => match event {
                Ok(data) => {
                    if !client.send(&ClientResponse::Event { data }).await {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    log::line(&format!(
                        "{}: fell {missed} events behind; closed with event_gap",
                        client.name()
                    ));
                    let message = format!(
                        "this client fell {missed} events behind; reconnect and fetch the fleet view"
                    );
                    client.send(&error(None, error_code::EVENT_GAP, message)).await;
                    return;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            chunk = next(&mut terminal) => match chunk {
                Ok(bytes_base64) => {
                    let data = TerminalBytesData { instance_id: terminal_of.clone(), bytes_base64 };
                    if !client.send(&ClientResponse::TerminalBytes { data }).await {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    log::line(&format!(
                        "{}: fell {missed} terminal chunks behind on {terminal_of}; closed",
                        client.name()
                    ));
                    return;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    terminal = None;
                    let message = format!("the terminal of {terminal_of} ended; subscribe again");
                    if !client.send(&error(None, error_code::NO_TERMINAL, message)).await {
                        return;
                    }
                }
            },
        }
    }
}

struct ConnectionGuard(Arc<AtomicBool>);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
async fn next_frame(
    view: &mut Option<ViewStream>,
) -> Result<Option<Arc<ClientResponse>>, watch::error::RecvError> {
    match view {
        Some(view) => view.next().await,
        None => std::future::pending().await,
    }
}
fn full_request(
    hub: &TerminalHub,
    scope: &ReplyScope,
    view: &mut Option<ViewStream>,
    caller: Option<&str>,
    selected: ProtocolVersion,
    request: ClientRequest,
    line_bytes: usize,
) -> Option<Result<(), ErrorData>> {
    let (id, control) = match &request {
        ClientRequest::SubscribeTerminalFrames { data } => (&data.request_id, false),
        ClientRequest::SetTerminalViewport { data } => (&data.request_id, false),
        ClientRequest::TerminalControl { data } => (&data.request_id, true),
        _ => return None,
    };
    // Caller always wins over an invalid instance, view, version or size.
    if control && caller.is_some() {
        return Some(Err(reject(
            Some(id.clone()),
            "forbidden",
            handlers::TYPE_OPERATOR_ONLY,
        )));
    }
    if selected < V1_4 {
        return Some(Err(reject(
            Some(id.clone()),
            "not_supported",
            "full terminal requires client protocol 1.4; upgrade the client",
        )));
    }
    if line_bytes > agend_core::protocol::holder::MAX_REQUEST_LINE {
        return Some(Err(reject(
            Some(id.clone()),
            "invalid_request",
            "terminal request exceeds 1 MiB; nothing was written",
        )));
    }
    if let ClientRequest::TerminalControl { data } = &request
        && let Err(error) = hub.live_instance(&data.instance_id, &data.request_id)
    {
        return Some(Err(error));
    }
    Some(match request {
        ClientRequest::SubscribeTerminalFrames { data } => hub
            .subscribe(scope.clone(), data)
            .map(|stream| *view = Some(stream)),
        ClientRequest::SetTerminalViewport { data } => match view {
            Some(view) => hub.viewport(scope.clone(), view, data),
            None => Err(reject(
                Some(data.request_id),
                "stale_terminal",
                "subscribe to this terminal on this connection first",
            )),
        },
        ClientRequest::TerminalControl { data } => match view {
            Some(view) => hub.control(scope.clone(), view, data),
            None => Err(reject(
                Some(data.request_id),
                "stale_terminal",
                "subscribe to this terminal on this connection first",
            )),
        },
        _ => unreachable!(),
    })
}
struct FrameLine(Vec<u8>);
impl io::Write for FrameLine {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() >= MAX_FRAME_LINE - self.0.len() {
            return Err(io::Error::other("terminal frame exceeds 8 MiB"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
