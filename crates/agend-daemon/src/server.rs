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
//!   that ends gets `no_terminal` (the connection stays).
//! - Every write has [`WRITE_TIMEOUT`]: a client that does not read at all
//!   fills the socket buffer and is closed (it gets no `event_gap`).
//! - [`Server::stop`]: stops accepting, removes the socket file, closes
//!   every connection.
//!
//! Must NOT: contain command logic (that is `handlers`).

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use agend_core::protocol::client::{
    ClientRequest, ClientResponse, EventData, SelectedVersionData, TerminalBytesData, error_code,
    negotiate_version,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, watch};
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
    let mut connections = JoinSet::new();
    let next = AtomicU64::new(1);
    loop {
        tokio::select! {
            _ = stopped.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let number = next.fetch_add(1, Ordering::Relaxed);
                    connections.spawn(connection(stream, Arc::clone(&ctx), number));
                }
                Err(e) => {
                    log::line(&format!("client socket: accept failed: {e}"));
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    drop(listener);
    let _ = fs::remove_file(&socket);
    connections.abort_all();
    while connections.join_next().await.is_some() {}
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
        let mut line = match serde_json::to_vec(response) {
            Ok(line) => line,
            Err(e) => {
                log::line(&format!("{}: cannot encode a response: {e}", self.name()));
                return false;
            }
        };
        line.push(b'\n');
        match tokio::time::timeout(WRITE_TIMEOUT, self.writer.write_all(&line)).await {
            Ok(Ok(())) => true,
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

async fn connection(stream: UnixStream, ctx: Arc<Context>, number: u64) {
    let (reader, writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let mut client = Client {
        number,
        caller: None,
        writer,
    };
    let mut negotiated = false;
    let mut events: Option<broadcast::Receiver<EventData>> = None;
    let mut terminal: Option<broadcast::Receiver<String>> = None;
    let mut terminal_of = String::new();
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { return };
                if line.trim().is_empty() {
                    continue;
                }
                let request: ClientRequest = match serde_json::from_str(&line) {
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
                    match negotiate_version(&data) {
                        Ok(selected) => {
                            negotiated = true;
                            client.caller = data.caller;
                            let reply = ClientResponse::Hello { data: SelectedVersionData { selected } };
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
                match handlers::handle(&ctx, client.caller.as_deref(), request).await {
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
