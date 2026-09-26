//! Blocking unix-socket connection to the daemon: JSON Lines of the core
//! protocol types, encoded with `serde_json` (the one wire format, gate 8 P3).
//!
//! Must NOT: spawn or restart the daemon.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::client::{
    AttentionAction, ClientRequest, ClientResponse, CommandResult, EventData, FleetView,
    RequestIdData, ResolveAttentionData, SubscribeEventsData, error_code,
};

use crate::retry::{RESTART_RETRY_WINDOW, RETRY_EVERY, Redo};
use crate::{ClientError, version};

/// Longest wait for the reply to a request (and to `hello`).
pub const REPLY_WITHIN: Duration = Duration::from_secs(10);

/// One connected, version-checked connection to the daemon.
pub struct Client {
    socket: PathBuf,
    caller: Option<String>,
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    selected: ProtocolVersion,
    /// Events read while waiting for a reply.
    events: VecDeque<EventData>,
    next_request: u64,
    retried: Duration,
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

/// One connection and `hello`, waiting at most `within` for the reply.
fn open(
    socket: &Path,
    caller: &Option<String>,
    within: Duration,
) -> Result<(BufReader<UnixStream>, UnixStream, ProtocolVersion), Attempt> {
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
    stream
        .set_read_timeout(Some(within.max(Duration::from_millis(10))))
        .map_err(fail)?;
    let mut writer = stream.try_clone().map_err(fail)?;
    let mut reader = BufReader::new(stream);
    writer
        .write_all(&line_of(&ClientRequest::hello_as(caller.clone())))
        .map_err(fail)?;
    match read_response(&mut reader) {
        Ok(Some(ClientResponse::Hello { data })) => {
            version::check(data.selected).map_err(|m| Attempt::Fail(ClientError::Version(m)))?;
            Ok((reader, writer, data.selected))
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
        Err(e) => Err(fail(e)),
    }
}

/// `connect`'s retry loop; returns the connection and how long it retried
/// (zero when the first attempt succeeded).
fn open_retrying(
    socket: &Path,
    caller: &Option<String>,
) -> Result<(BufReader<UnixStream>, UnixStream, ProtocolVersion, Duration), ClientError> {
    let started = Instant::now();
    let deadline = started + RESTART_RETRY_WINDOW;
    let mut failed = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match open(socket, caller, left.min(REPLY_WITHIN)) {
            Ok((reader, writer, selected)) => {
                let retried = if failed {
                    started.elapsed()
                } else {
                    Duration::ZERO
                };
                return Ok((reader, writer, selected, retried));
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
fn request_id(request: &ClientRequest) -> Option<&str> {
    match request {
        ClientRequest::Command { data } => Some(&data.request_id),
        ClientRequest::AnswerAsk { data } => Some(&data.request_id),
        ClientRequest::GetFleet { data } => Some(&data.request_id),
        ClientRequest::ResolveAttention { data } => Some(&data.request_id),
        _ => None,
    }
}

impl Client {
    /// Connects and says `hello`, retrying every 100 ms for up to 10 s while
    /// the socket is missing, refuses connections, or closes before `hello`
    /// is answered. `caller` is the instance id inside an agent, `None` for
    /// the operator.
    pub fn connect(socket: &Path, caller: Option<String>) -> Result<Client, ClientError> {
        let (reader, writer, selected, retried) = open_retrying(socket, &caller)?;
        let mut client = Client::new(socket, caller, reader, writer, selected);
        client.retried = retried;
        Ok(client)
    }

    /// One attempt, no retry (the TUI and `agend debug watch` reconnect on
    /// their own schedule).
    pub fn connect_once(socket: &Path, caller: Option<String>) -> Result<Client, ClientError> {
        match open(socket, &caller, REPLY_WITHIN) {
            Ok((reader, writer, selected)) => {
                Ok(Client::new(socket, caller, reader, writer, selected))
            }
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
        reader: BufReader<UnixStream>,
        writer: UnixStream,
        selected: ProtocolVersion,
    ) -> Client {
        Client {
            socket: socket.to_path_buf(),
            caller,
            reader,
            writer,
            selected,
            events: VecDeque::new(),
            next_request: 0,
            retried: Duration::ZERO,
        }
    }

    /// The version the daemon selected.
    pub fn selected(&self) -> ProtocolVersion {
        self.selected
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
        let (reader, writer, selected, _) = open_retrying(&self.socket, &self.caller)?;
        self.reader = reader;
        self.writer = writer;
        self.selected = selected;
        self.retried += started.elapsed();
        Ok(())
    }

    /// Sends `request` and waits (10 s) for the reply carrying its request
    /// id; events read meanwhile are kept for [`Client::next_event`]. A
    /// request that could not be written is sent again after reconnecting
    /// (it never reached the daemon). If the connection ends after it was
    /// sent, only a [`Redo::Safe`] request is sent again; otherwise
    /// [`ClientError::Restarted`]. The subscription does not survive a
    /// reconnect.
    pub fn request(
        &mut self,
        request: &ClientRequest,
        redo: Redo,
    ) -> Result<ClientResponse, ClientError> {
        let id = request_id(request).map(str::to_owned);
        let line = line_of(request);
        loop {
            if self.writer.write_all(&line).is_err() {
                self.reconnect()?;
                continue;
            }
            match self.wait_reply(id.as_deref()) {
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

    fn wait_reply(&mut self, id: Option<&str>) -> Result<ClientResponse, ReplyError> {
        let deadline = Instant::now() + REPLY_WITHIN;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ReplyError::Other(no_reply()));
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
                    return Err(ReplyError::Other(no_reply()));
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
        let request = ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: self.next_request_id(),
                attention_id: attention_id.to_owned(),
                action,
            },
        };
        match self.request(&request, Redo::Never)? {
            ClientResponse::CommandResult { data } if data.result == CommandResult::Accepted => {
                Ok(())
            }
            other => Err(unexpected(&other)),
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

enum ReplyError {
    /// The connection ended.
    Ended,
    Other(ClientError),
}

fn no_reply() -> ClientError {
    ClientError::Disconnected(format!(
        "no reply from the AgEnD daemon within {} s",
        REPLY_WITHIN.as_secs()
    ))
}

fn unexpected(reply: &ClientResponse) -> ClientError {
    ClientError::Disconnected(format!("unexpected reply: {reply:?}"))
}
