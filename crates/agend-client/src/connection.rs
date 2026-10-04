//! Blocking unix-socket connection to the daemon: JSON Lines of the core
//! protocol types, encoded with `serde_json` (the one wire format, gate 8 P3).
//!
//! Must NOT: spawn or restart the daemon.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::ask::{AnswerSource, AskReply};
use agend_core::protocol::client::{
    AnswerAskData, AttentionAction, ClientRequest, ClientResponse, CommandResult, EventData,
    FleetView, InstanceData, RequestIdData, ResolveAttentionData, SelectedVersionData,
    SubscribeEventsData, TerminalInputData, error_code,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::retry::{RESTART_RETRY_WINDOW, RETRY_EVERY, Redo};
use crate::{ClientError, version};

/// Longest wait for the reply to a request (and to `hello`).
pub const REPLY_WITHIN: Duration = Duration::from_secs(10);

/// One connected, version-checked connection to the daemon.
pub struct Client {
    socket: PathBuf,
    caller: Option<String>,
    pub(crate) reader: BufReader<UnixStream>,
    writer: UnixStream,
    /// The daemon's `hello` reply (selected version, 1.2: who it is).
    hello: SelectedVersionData,
    /// The oldest version accepted at (re)connect.
    needed: ProtocolVersion,
    /// Events read while waiting for a reply.
    events: VecDeque<EventData>,
    next_request: u64,
    retried: Duration,
    terminal_write_lock: Arc<Mutex<()>>,
    pub(crate) terminal_failed: bool,
}

/// Why one attempt failed.
enum Attempt {
    /// Worth another try while the window lasts.
    Retry(String),
    Fail(ClientError),
}

fn retryable(e: &io::Error) -> bool {
    use io::ErrorKind::*;
    matches!(
        e.kind(),
        NotFound
            | ConnectionRefused
            | ConnectionReset
            | ConnectionAborted
            | BrokenPipe
            | UnexpectedEof
            | WouldBlock
            | TimedOut
    )
}

fn line_of(request: &ClientRequest) -> Vec<u8> {
    let mut line = serde_json::to_vec(request).expect("protocol types serialize");
    line.push(b'\n');
    line
}

/// Reads one response; `Ok(None)` at end of stream.
fn read_response(reader: &mut BufReader<UnixStream>) -> io::Result<Option<ClientResponse>> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if !line.trim().is_empty() {
            return serde_json::from_str(&line).map(Some).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("not a client protocol line: {e}"),
                )
            });
        }
    }
}

type Opened = (BufReader<UnixStream>, UnixStream, SelectedVersionData);

/// One connection and `hello`, waiting at most `within` for the reply; the
/// daemon must select at least `needed`.
fn open(
    socket: &Path,
    caller: &Option<String>,
    needed: ProtocolVersion,
    within: Duration,
) -> Result<Opened, Attempt> {
    let fail = |e: io::Error| {
        if retryable(&e) {
            Attempt::Retry(e.to_string())
        } else {
            Attempt::Fail(ClientError::Connect {
                socket: socket.to_path_buf(),
                cause: e.to_string(),
            })
        }
    };
    let stream = UnixStream::connect(socket).map_err(fail)?;
    // Connected: any failure from here until the hello reply means the
    // connection ended before a request was sent, which is retried (P7).
    // (macOS reports a peer that closed right after accepting as ENOTCONN
    // or EINVAL on these calls.)
    let ended = |e: io::Error| Attempt::Retry(format!("the connection ended during hello: {e}"));
    stream
        .set_read_timeout(Some(within.max(Duration::from_millis(10))))
        .map_err(ended)?;
    let mut writer = stream.try_clone().map_err(ended)?;
    let mut reader = BufReader::new(stream);
    writer
        .write_all(&line_of(&ClientRequest::hello_as(caller.clone())))
        .map_err(ended)?;
    match read_response(&mut reader) {
        Ok(Some(ClientResponse::Hello { data })) => {
            version::check_at_least(data.selected, needed)
                .map_err(|m| Attempt::Fail(ClientError::Version(m)))?;
            Ok((reader, writer, data))
        }
        Ok(Some(ClientResponse::Error { data })) if data.code == error_code::VERSION_MISMATCH => {
            Err(Attempt::Fail(ClientError::Version(data.message)))
        }
        Ok(Some(ClientResponse::Error { data })) => Err(Attempt::Fail(ClientError::Daemon {
            code: data.code,
            message: data.message,
        })),
        Ok(Some(other)) => Err(Attempt::Fail(ClientError::Disconnected(format!(
            "unexpected reply to hello: {other:?}"
        )))),
        Ok(None) => Err(Attempt::Retry(
            "the daemon closed the connection during hello".into(),
        )),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            Err(Attempt::Fail(ClientError::Disconnected(e.to_string())))
        }
        Err(e) => Err(ended(e)),
    }
}

/// `connect`'s retry loop; returns the connection and how long it retried
/// (zero when the first attempt succeeded).
fn open_retrying(
    socket: &Path,
    caller: &Option<String>,
    needed: ProtocolVersion,
) -> Result<(Opened, Duration), ClientError> {
    let started = Instant::now();
    let deadline = started + RESTART_RETRY_WINDOW;
    let mut failed = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match open(socket, caller, needed, left.min(REPLY_WITHIN)) {
            Ok(opened) => {
                let retried = if failed {
                    started.elapsed()
                } else {
                    Duration::ZERO
                };
                return Ok((opened, retried));
            }
            Err(Attempt::Fail(e)) => return Err(e),
            Err(Attempt::Retry(cause)) => {
                failed = true;
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(ClientError::Unreachable {
                        socket: socket.to_path_buf(),
                        cause,
                    });
                }
                std::thread::sleep(RETRY_EVERY.min(left));
            }
        }
    }
}

/// The request id a reply to `request` carries.
pub(crate) fn request_id(request: &ClientRequest) -> Option<&str> {
    match request {
        ClientRequest::Claude { data } => Some(&data.request_id),
        ClientRequest::Command { data } => Some(&data.request_id),
        ClientRequest::AnswerAsk { data } => Some(&data.request_id),
        ClientRequest::GetFleet { data } => Some(&data.request_id),
        ClientRequest::ResolveAttention { data } => Some(&data.request_id),
        ClientRequest::Operator { data } => Some(&data.request_id),
        _ => None,
    }
}

impl Client {
    /// Connects and says `hello`, retrying every 100 ms for up to 10 s while
    /// the socket is missing, refuses connections, or closes before `hello`
    /// is answered. `caller` is the instance id inside an agent, `None` for
    /// the operator.
    pub fn connect(socket: &Path, caller: Option<String>) -> Result<Client, ClientError> {
        Self::connect_needing(socket, caller, version::NEEDED)
    }

    /// [`Client::connect`] accepting any daemon that selects at least
    /// `needed` (`agend daemon restart` needs only `daemon_restart`).
    pub fn connect_needing(
        socket: &Path,
        caller: Option<String>,
        needed: ProtocolVersion,
    ) -> Result<Client, ClientError> {
        let (opened, retried) = open_retrying(socket, &caller, needed)?;
        let mut client = Client::new(socket, caller, opened, needed);
        client.retried = retried;
        Ok(client)
    }

    /// One attempt, no retry (the TUI, `agend debug watch` and `agend
    /// doctor` reconnect on their own schedule or not at all).
    pub fn connect_once(socket: &Path, caller: Option<String>) -> Result<Client, ClientError> {
        match open(socket, &caller, version::NEEDED, REPLY_WITHIN) {
            Ok(opened) => Ok(Client::new(socket, caller, opened, version::NEEDED)),
            Err(Attempt::Fail(e)) => Err(e),
            Err(Attempt::Retry(cause)) => Err(ClientError::Connect {
                socket: socket.to_path_buf(),
                cause,
            }),
        }
    }

    fn new(
        socket: &Path,
        caller: Option<String>,
        opened: Opened,
        needed: ProtocolVersion,
    ) -> Client {
        let (reader, writer, hello) = opened;
        Client {
            socket: socket.to_path_buf(),
            caller,
            reader,
            writer,
            hello,
            needed,
            events: VecDeque::new(),
            next_request: 0,
            retried: Duration::ZERO,
            terminal_write_lock: Arc::new(Mutex::new(())),
            terminal_failed: false,
        }
    }

    /// The version the daemon selected.
    pub fn selected(&self) -> ProtocolVersion {
        self.hello.selected
    }

    /// The daemon's `hello` reply: with 1.2, its version, pid and boot id.
    pub fn daemon(&self) -> &SelectedVersionData {
        &self.hello
    }

    /// Time spent retrying so far: connecting while the daemon was away,
    /// and reconnecting to send a [`Redo::Safe`] request again.
    pub fn retried(&self) -> Duration {
        self.retried
    }

    /// A request id unique on this client.
    pub fn next_request_id(&mut self) -> String {
        self.next_request += 1;
        format!("c{}-{}", std::process::id(), self.next_request)
    }

    fn reconnect(&mut self) -> Result<(), ClientError> {
        let started = Instant::now();
        let ((reader, writer, hello), _) = open_retrying(&self.socket, &self.caller, self.needed)?;
        self.terminal_write_lock = Arc::new(Mutex::new(()));
        self.reader = reader;
        self.writer = writer;
        self.hello = hello;
        self.retried += started.elapsed();
        Ok(())
    }

    /// Reads (and drops) whatever comes until the daemon closes this
    /// connection; false when it is still open after `within` (`agend
    /// daemon restart` waits for the old daemon to let go, gate 9 P7).
    pub fn wait_closed(&mut self, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        let mut buf = [0u8; 4096];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            let _ = self.reader.get_ref().set_read_timeout(Some(left));
            match std::io::Read::read(&mut self.reader, &mut buf) {
                Ok(0) => return true,
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return true,
            }
        }
    }

    /// Sends `request` and waits (10 s) for the reply carrying its request
    /// id; events read meanwhile are kept for [`Client::next_event`]. A
    /// request whose write fails is sent again after reconnecting (a prefix
    /// may have reached the daemon). Use [`crate::exchange_once`] when even
    /// a partial write must not be replayed. If the connection ends after it was
    /// sent, only a [`Redo::Safe`] request is sent again; otherwise
    /// [`ClientError::Restarted`]. The subscription does not survive a
    /// reconnect.
    pub fn request(
        &mut self,
        request: &ClientRequest,
        redo: Redo,
    ) -> Result<ClientResponse, ClientError> {
        self.request_within(request, redo, REPLY_WITHIN)
    }

    /// [`Client::request`] waiting up to `within` for the reply (a restart
    /// preflight takes longer than 10 s at worst).
    pub fn request_within(
        &mut self,
        request: &ClientRequest,
        redo: Redo,
        within: Duration,
    ) -> Result<ClientResponse, ClientError> {
        if crate::terminal::is_terminal_request(request) {
            version::check_at_least(self.selected(), agend_core::protocol::client::V1_4)
                .map_err(ClientError::Version)?;
            return Err(ClientError::Daemon {
                code: error_code::INVALID_REQUEST.into(),
                message: "use Sender::send_terminal and Client::next_full_terminal; terminal operations never reconnect or replay".into(),
            });
        }
        if matches!(request, ClientRequest::Claude { .. }) {
            return Err(ClientError::Daemon {
                code: error_code::INVALID_REQUEST.into(),
                message: "Claude RPCs require exchange_once; never replay them".into(),
            });
        }
        let id = request_id(request).map(str::to_owned);
        let line = line_of(request);
        loop {
            if self.writer.write_all(&line).is_err() {
                self.reconnect()?;
                continue;
            }
            match self.wait_reply(id.as_deref(), within) {
                Err(ReplyError::Ended) if redo == Redo::Safe => self.reconnect()?,
                Err(ReplyError::Ended) => return Err(ClientError::Restarted),
                Err(ReplyError::Other(e)) => return Err(e),
                Ok(ClientResponse::Error { data }) => {
                    return Err(ClientError::Daemon {
                        code: data.code,
                        message: data.message,
                    });
                }
                Ok(reply) => return Ok(reply),
            }
        }
    }

    fn wait_reply(
        &mut self,
        id: Option<&str>,
        within: Duration,
    ) -> Result<ClientResponse, ReplyError> {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ReplyError::Other(no_reply(within)));
            }
            let _ = self.reader.get_ref().set_read_timeout(Some(left));
            let response = match read_response(&mut self.reader) {
                Ok(Some(response)) => response,
                Ok(None) => return Err(ReplyError::Ended),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(ReplyError::Other(no_reply(within)));
                }
                Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                    return Err(ReplyError::Other(ClientError::Disconnected(e.to_string())));
                }
                Err(_) => return Err(ReplyError::Ended),
            };
            match &response {
                ClientResponse::Event { data } => self.events.push_back(data.clone()),
                ClientResponse::CommandResult { data } if Some(data.request_id.as_str()) == id => {
                    return Ok(response);
                }
                ClientResponse::Fleet { data } if Some(data.request_id.as_str()) == id => {
                    return Ok(response);
                }
                ClientResponse::Error { data }
                    if data.request_id.is_none() || data.request_id.as_deref() == id =>
                {
                    return Ok(response);
                }
                _ => {}
            }
        }
    }

    /// The fleet view (a read: sent again after a reconnect).
    pub fn get_fleet(&mut self) -> Result<FleetView, ClientError> {
        let request = ClientRequest::GetFleet {
            data: RequestIdData {
                request_id: self.next_request_id(),
            },
        };
        match self.request(&request, Redo::Safe)? {
            ClientResponse::Fleet { data } => Ok(data.fleet),
            other => Err(unexpected(&other)),
        }
    }

    /// Acts on a needs-you item (operator only; never sent twice).
    pub fn resolve_attention(
        &mut self,
        attention_id: &str,
        action: AttentionAction,
    ) -> Result<(), ClientError> {
        self.resolve_attention_with_note(attention_id, action, None)
    }

    pub fn resolve_attention_with_note(
        &mut self,
        attention_id: &str,
        action: AttentionAction,
        note: Option<String>,
    ) -> Result<(), ClientError> {
        let request = ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: self.next_request_id(),
                attention_id: attention_id.to_owned(),
                action,
                note,
            },
        };
        match self.request(&request, Redo::Never)? {
            ClientResponse::CommandResult { data } if data.result == CommandResult::Accepted => {
                Ok(())
            }
            other => Err(unexpected(&other)),
        }
    }

    /// The operator's answer to a needs-you ask (never sent twice).
    pub fn answer_ask(
        &mut self,
        ask_id: &str,
        source: AnswerSource,
        reply: AskReply,
    ) -> Result<(), ClientError> {
        let request = ClientRequest::AnswerAsk {
            data: AnswerAskData {
                request_id: self.next_request_id(),
                ask_id: ask_id.to_owned(),
                source,
                reply,
            },
        };
        match self.request(&request, Redo::Never)? {
            ClientResponse::CommandResult { data } if data.result == CommandResult::Accepted => {
                Ok(())
            }
            other => Err(unexpected(&other)),
        }
    }

    /// A write-only handle on this connection (a clone of its socket), so
    /// another thread can write while this one blocks in
    /// [`Client::next_terminal`] (gate 11 B P1).
    pub fn sender(&self) -> Result<Sender, ClientError> {
        let stream = self
            .writer
            .try_clone()
            .map_err(|e| ClientError::Disconnected(format!("cannot clone the connection: {e}")))?;
        Ok(Sender {
            stream,
            selected: self.selected(),
            terminal_write_lock: Arc::clone(&self.terminal_write_lock),
        })
    }

    /// Blocks until the next screen or PTY chunk of a terminal subscription
    /// (sent with [`Sender::subscribe_terminal`]). Any error line
    /// (`no_terminal`, `forbidden`, `not_supported`, …) is
    /// [`ClientError::Daemon`]; the end of the connection is
    /// [`ClientError::Disconnected`].
    pub fn next_terminal(&mut self) -> Result<TerminalUpdate, ClientError> {
        let _ = self.reader.get_ref().set_read_timeout(None);
        loop {
            match read_response(&mut self.reader) {
                Ok(Some(ClientResponse::TerminalSnapshot { data })) => {
                    return Ok(TerminalUpdate::Screen {
                        instance_id: data.instance_id,
                        screen: data.screen,
                    });
                }
                Ok(Some(ClientResponse::TerminalBytes { data })) => {
                    let bytes = BASE64.decode(&data.bytes_base64).map_err(|e| {
                        ClientError::Disconnected(format!("terminal_bytes is not base64: {e}"))
                    })?;
                    return Ok(TerminalUpdate::Bytes {
                        instance_id: data.instance_id,
                        bytes,
                    });
                }
                Ok(Some(ClientResponse::Error { data })) => {
                    return Err(ClientError::Daemon {
                        code: data.code,
                        message: data.message,
                    });
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(ClientError::Disconnected(
                        "the daemon closed the connection".into(),
                    ));
                }
                Err(e) => return Err(ClientError::Disconnected(e.to_string())),
            }
        }
    }

    /// Subscribes to events after `after_event_id` (a 1.1 client passes the
    /// fleet view's `as_of_event_id`). A refused cursor arrives as
    /// [`ClientError::Daemon`] `event_gap` from [`Client::next_event`].
    pub fn subscribe_events(&mut self, after_event_id: Option<u64>) -> Result<(), ClientError> {
        let request = ClientRequest::SubscribeEvents {
            data: SubscribeEventsData { after_event_id },
        };
        self.writer
            .write_all(&line_of(&request))
            .map_err(|e| ClientError::Disconnected(format!("cannot subscribe: {e}")))
    }

    /// Blocks until the next event. An error line (`event_gap` when this
    /// client fell behind) is [`ClientError::Daemon`]; the end of the
    /// connection is [`ClientError::Disconnected`].
    pub fn next_event(&mut self) -> Result<EventData, ClientError> {
        if let Some(event) = self.events.pop_front() {
            return Ok(event);
        }
        let _ = self.reader.get_ref().set_read_timeout(None);
        loop {
            match read_response(&mut self.reader) {
                Ok(Some(ClientResponse::Event { data })) => return Ok(data),
                Ok(Some(ClientResponse::Error { data })) => {
                    return Err(ClientError::Daemon {
                        code: data.code,
                        message: data.message,
                    });
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(ClientError::Disconnected(
                        "the daemon closed the connection".into(),
                    ));
                }
                Err(e) => return Err(ClientError::Disconnected(e.to_string())),
            }
        }
    }
}

/// What a terminal subscription delivers ([`Client::next_terminal`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalUpdate {
    /// The holder's screen as text (first, and after every new subscription).
    Screen { instance_id: String, screen: String },
    /// PTY output after that screen, decoded.
    Bytes { instance_id: String, bytes: Vec<u8> },
}

/// The write-only side of a connection ([`Client::sender`]): it sends lines
/// and never reads, so it does not wait for replies (they reach whoever
/// reads the [`Client`]).
pub struct Sender {
    pub(crate) stream: UnixStream,
    pub(crate) selected: ProtocolVersion,
    pub(crate) terminal_write_lock: Arc<Mutex<()>>,
}

impl Sender {
    /// Subscribes to `instance_id`'s terminal; a new subscription replaces
    /// the old one (gate 8 C8).
    pub fn subscribe_terminal(&mut self, instance_id: &str) -> Result<(), ClientError> {
        self.send(&ClientRequest::SubscribeTerminal {
            data: InstanceData {
                instance_id: instance_id.to_owned(),
            },
        })
    }

    /// Operator input for `instance_id`'s PTY. Not answered when accepted;
    /// a refusal is an error line without a request id.
    pub fn terminal_input(&mut self, instance_id: &str, bytes: &[u8]) -> Result<(), ClientError> {
        self.send(&ClientRequest::TerminalInput {
            data: TerminalInputData {
                instance_id: instance_id.to_owned(),
                bytes_base64: BASE64.encode(bytes),
            },
        })
    }

    /// Shuts the connection down both ways: a thread blocked reading the
    /// [`Client`] reads the end of the stream at once. Dropping a `Sender`
    /// only closes its own handle; the connection stays open.
    pub fn close(&self) {
        shutdown(&self.stream);
    }

    fn send(&mut self, request: &ClientRequest) -> Result<(), ClientError> {
        let _guard = self
            .terminal_write_lock
            .lock()
            .map_err(|_| ClientError::Disconnected("terminal writer lock is poisoned".into()))?;
        self.stream
            .write_all(&line_of(request))
            .map_err(|e| ClientError::Disconnected(format!("cannot write to the daemon: {e}")))
    }
}

enum ReplyError {
    /// The connection ended.
    Ended,
    Other(ClientError),
}

fn no_reply(within: Duration) -> ClientError {
    ClientError::Disconnected(format!(
        "no reply from the AgEnD daemon within {} s",
        within.as_secs()
    ))
}

fn unexpected(reply: &ClientResponse) -> ClientError {
    ClientError::Disconnected(format!("unexpected reply: {reply:?}"))
}

/// macOS can reject SHUT_RDWR after peer write EOF while our write half is
/// still open. Close each direction as a fallback so the peer sees release.
pub(crate) fn shutdown(stream: &UnixStream) {
    if stream.shutdown(std::net::Shutdown::Both).is_err() {
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let _ = stream.shutdown(std::net::Shutdown::Read);
    }
}
