//! A `Source` that talks client protocol v1 (JSON Lines over a unix socket)
//! to the testkit fake daemon, for the demo and tests. It is a harness, not
//! the product: gate 11 proper implements `Source` on `agend-client` once
//! gate 8 fixes the client protocol and connection code.
//!
//! Protocol v1 has no request that lists teams, tasks or agents, so the
//! catalog is handed in (a gap recorded for gate 8). Everything else (asks,
//! answers, events, terminal snapshots) goes over the socket.
//!
//! Shared by `examples/*.rs` and `tests/*.rs` with `#[path]`.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agend_core::protocol::ask::{AnswerSource, AskReply};
use agend_core::protocol::client::{
    AnswerAskData, ClientRequest, ClientResponse, EventData, InstanceData, SubscribeEventsData,
};
use agend_testkit::fake_daemon::FakeDaemon;
use agend_tui::source::scripted::{DemoStep, demo_script};
use agend_tui::source::{Catalog, Source, SourceError};

/// How long to wait for a reply before treating the daemon as gone.
const REPLY_TIMEOUT: Duration = Duration::from_secs(3);

/// The socket path the source dials; the demo points it at a new fake
/// daemon to show reconnecting.
pub type Address = Arc<Mutex<PathBuf>>;

pub struct DaemonSource {
    address: Address,
    catalog: Catalog,
    connection: Option<Connection>,
    next_request: u64,
}

struct Connection {
    writer: UnixStream,
    /// `None` marks EOF or a read error.
    replies: Receiver<Option<ClientResponse>>,
    /// Events that arrived while waiting for a reply.
    events: Vec<EventData>,
}

impl DaemonSource {
    pub fn new(path: PathBuf, catalog: Catalog) -> (DaemonSource, Address) {
        let address = Arc::new(Mutex::new(path));
        let source = DaemonSource {
            address: Arc::clone(&address),
            catalog,
            connection: None,
            next_request: 0,
        };
        (source, address)
    }

    fn send(&mut self, request: &ClientRequest) -> Result<(), SourceError> {
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| SourceError::Disconnected("not connected".into()))?;
        let mut line = serde_json::to_string(request).expect("protocol types serialize");
        line.push('\n');
        if let Err(e) = connection.writer.write_all(line.as_bytes()) {
            self.connection = None;
            return Err(SourceError::Disconnected(format!("write failed: {e}")));
        }
        Ok(())
    }

    /// Waits for the first reply `matches` accepts, buffering events.
    fn wait_for(
        &mut self,
        matches: impl Fn(&ClientResponse) -> bool,
    ) -> Result<ClientResponse, SourceError> {
        let deadline = Instant::now() + REPLY_TIMEOUT;
        loop {
            let connection = self
                .connection
                .as_mut()
                .ok_or_else(|| SourceError::Disconnected("not connected".into()))?;
            let left = deadline.saturating_duration_since(Instant::now());
            match connection.replies.recv_timeout(left) {
                Ok(Some(ClientResponse::Event { data })) => connection.events.push(data),
                Ok(Some(reply)) if matches(&reply) => return Ok(reply),
                Ok(Some(_)) => {}
                Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                    self.connection = None;
                    return Err(SourceError::Disconnected(
                        "the daemon closed the connection".into(),
                    ));
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.connection = None;
                    return Err(SourceError::Disconnected(format!(
                        "no reply within {} s",
                        REPLY_TIMEOUT.as_secs()
                    )));
                }
            }
        }
    }

    fn request_id(&mut self) -> String {
        self.next_request += 1;
        format!("tui-{}", self.next_request)
    }
}

impl Source for DaemonSource {
    fn connect(&mut self) -> Result<Catalog, SourceError> {
        self.connection = None;
        let path = self
            .address
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let stream = UnixStream::connect(&path).map_err(|e| {
            SourceError::Disconnected(format!("cannot connect to {}: {e}", path.display()))
        })?;
        let reader = stream
            .try_clone()
            .map_err(|e| SourceError::Disconnected(e.to_string()))?;
        let (tx, replies) = channel();
        std::thread::Builder::new()
            .name("tui-daemon-reader".into())
            .spawn(move || {
                for line in BufReader::new(reader).lines() {
                    let Ok(line) = line else { break };
                    if let Ok(reply) = serde_json::from_str::<ClientResponse>(&line)
                        && tx.send(Some(reply)).is_err()
                    {
                        return;
                    }
                }
                let _ = tx.send(None);
            })
            .map_err(|e| SourceError::Disconnected(e.to_string()))?;
        self.connection = Some(Connection {
            writer: stream,
            replies,
            events: Vec::new(),
        });
        self.send(&ClientRequest::hello())?;
        match self.wait_for(|r| {
            matches!(
                r,
                ClientResponse::Hello { .. } | ClientResponse::Error { .. }
            )
        })? {
            ClientResponse::Hello { .. } => {}
            ClientResponse::Error { data } => {
                self.connection = None;
                return Err(SourceError::Disconnected(format!(
                    "{}: {}",
                    data.code, data.message
                )));
            }
            _ => unreachable!("filtered above"),
        }
        self.send(&ClientRequest::SubscribeEvents {
            data: SubscribeEventsData {
                after_event_id: None,
            },
        })?;
        Ok(self.catalog.clone())
    }

    fn poll(&mut self) -> Result<Vec<EventData>, SourceError> {
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| SourceError::Disconnected("not connected".into()))?;
        loop {
            match connection.replies.try_recv() {
                Ok(Some(ClientResponse::Event { data })) => connection.events.push(data),
                Ok(Some(_)) => {}
                Err(TryRecvError::Empty) => return Ok(std::mem::take(&mut connection.events)),
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    self.connection = None;
                    return Err(SourceError::Disconnected(
                        "the daemon closed the connection".into(),
                    ));
                }
            }
        }
    }

    fn terminal(&mut self, instance_id: &str) -> Result<String, SourceError> {
        self.send(&ClientRequest::SubscribeTerminal {
            data: InstanceData {
                instance_id: instance_id.into(),
            },
        })?;
        let reply = self.wait_for(|r| {
            matches!(r, ClientResponse::TerminalSnapshot { data } if data.instance_id == instance_id)
        })?;
        match reply {
            ClientResponse::TerminalSnapshot { data } => Ok(data.screen),
            _ => unreachable!("filtered above"),
        }
    }

    fn answer(&mut self, ask_id: &str, reply: AskReply) -> Result<(), SourceError> {
        let request_id = self.request_id();
        self.send(&ClientRequest::AnswerAsk {
            data: AnswerAskData {
                request_id: request_id.clone(),
                ask_id: ask_id.into(),
                source: AnswerSource::Tui,
                reply,
            },
        })?;
        let id = request_id.clone();
        match self.wait_for(move |r| match r {
            ClientResponse::CommandResult { data } => data.request_id == id,
            ClientResponse::Error { data } => data.request_id.as_deref() == Some(id.as_str()),
            _ => false,
        })? {
            ClientResponse::Error { data } => Err(SourceError::Rejected {
                code: data.code,
                message: data.message,
            }),
            _ => Ok(()),
        }
    }
}

/// Seeds a fake daemon with the same demo as `ScriptedSource::demo`.
pub fn seed_demo(daemon: &FakeDaemon) {
    for step in demo_script() {
        match step {
            DemoStep::Event(event) => {
                daemon.emit(event);
            }
            DemoStep::Ask(thread, recap) => {
                daemon.open_ask(thread, recap);
            }
        }
    }
}
