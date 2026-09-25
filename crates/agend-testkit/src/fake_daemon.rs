//! Fake daemon: an in-process client protocol v1 server (JSON Lines over a
//! unix socket, P1/D26) for testing `agend-client`, the CLI and the TUI
//! without a real daemon. Every message is an `agend_core::protocol::client`
//! type encoded with `serde_json`, so the wire shape is the real one.
//!
//! Covered:
//! - `hello` first; version negotiation with `protocol::negotiate`; mismatch
//!   answers an `error` (`version_mismatch`) and closes; any other first
//!   line (another request or invalid JSON) answers `hello_required` and
//!   closes.
//! - `command`: `status`, `inbox`, `send`, `task_create`, `ask`, the other
//!   agent commands (`accepted`), and the result commands (`done`, `result`,
//!   `review_approve`, `review_changes`) with the event-identity rule: the
//!   result must carry the `stage_id` and `attempt` of the current
//!   assignment (`FakeDaemon::assign`), otherwise `stale_result` and nothing
//!   changes; an accepted result consumes the assignment.
//! - `subscribe_events`: backlog after `after_event_id`, then live events.
//! - `subscribe_terminal`: one `terminal_snapshot`.
//! - `answer_ask`.
//!
//! Not covered: terminal byte streaming, authentication, persistence.
//! Error codes other than `stale_result` are the fake's own (not yet fixed in
//! `agend_core`; gate 8 decides).
//!
//! Must NOT: share code paths with the real server beyond `agend_core::protocol`.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use agend_core::protocol::ask::{AskEntry, AskThread};
use agend_core::protocol::client::{
    AgentCommand, AskCreatedData, AttentionRequiredData, ClientCommandResultData, ClientRequest,
    ClientResponse, CommandResult, DaemonEvent, ErrorData, EventData, InboxMessage, MessagesData,
    ResultIdentity, STALE_RESULT, SUPPORTED_VERSIONS, SelectedVersionData, StatusData,
    TaskChangedData, TaskCreatedData, TerminalSnapshotData,
};
use agend_core::protocol::{ProtocolVersion, negotiate};

use crate::fakes::lock;
use crate::tempdir::TempDir;

pub const HELLO_REQUIRED: &str = "hello_required";
pub const VERSION_MISMATCH: &str = "version_mismatch";
pub const INVALID_REQUEST: &str = "invalid_request";
pub const UNKNOWN_REQUEST: &str = "unknown_request";
pub const UNKNOWN_ASK: &str = "unknown_ask";

type Writer = Arc<Mutex<UnixStream>>;

#[derive(Default)]
struct State {
    supported: Vec<ProtocolVersion>,
    requests: Vec<ClientRequest>,
    events: Vec<EventData>,
    subscribers: Vec<Writer>,
    assignments: BTreeMap<String, ResultIdentity>,
    status_summary: String,
    inbox: Vec<InboxMessage>,
    asks: BTreeMap<String, AskThread>,
    next_id: u64,
}

struct Shared {
    state: Mutex<State>,
    stopping: AtomicBool,
    /// Every accepted connection, so drop can close them.
    connections: Mutex<Vec<UnixStream>>,
}

/// A running fake daemon. Drop stops accepting, closes every open
/// connection (clients read EOF) and removes the socket.
pub struct FakeDaemon {
    socket_path: PathBuf,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
    _dir: TempDir,
}

impl FakeDaemon {
    /// Listens on `daemon.sock` in a new temp directory.
    pub fn start() -> io::Result<FakeDaemon> {
        let dir = TempDir::new("fd")?;
        let socket_path = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket_path)?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                supported: SUPPORTED_VERSIONS.to_vec(),
                status_summary: "fake daemon: idle".into(),
                ..State::default()
            }),
            stopping: AtomicBool::new(false),
            connections: Mutex::new(Vec::new()),
        });
        let accept_shared = Arc::clone(&shared);
        let accept = std::thread::Builder::new()
            .name("fake-daemon-accept".into())
            .spawn(move || accept_loop(listener, accept_shared))?;
        Ok(FakeDaemon {
            socket_path,
            shared,
            accept: Some(accept),
            _dir: dir,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Versions offered in negotiation (default: `SUPPORTED_VERSIONS`).
    pub fn set_supported_versions(&self, versions: &[ProtocolVersion]) {
        lock(&self.shared.state).supported = versions.to_vec();
    }

    /// The stage attempt whose result the daemon now waits for on `task_id`
    /// (what a `PipelineAction` carries to the agent).
    pub fn assign(&self, task_id: &str, identity: ResultIdentity) {
        lock(&self.shared.state)
            .assignments
            .insert(task_id.to_owned(), identity);
    }

    pub fn assignment(&self, task_id: &str) -> Option<ResultIdentity> {
        lock(&self.shared.state).assignments.get(task_id).cloned()
    }

    pub fn set_status(&self, summary: &str) {
        lock(&self.shared.state).status_summary = summary.to_owned();
    }

    pub fn push_inbox(&self, message: InboxMessage) {
        lock(&self.shared.state).inbox.push(message);
    }

    /// Appends an event to the log and sends it to every subscriber.
    pub fn emit(&self, event: DaemonEvent) -> u64 {
        emit(&mut lock(&self.shared.state), event)
    }

    /// Every request received, in order, from all connections.
    pub fn requests(&self) -> Vec<ClientRequest> {
        lock(&self.shared.state).requests.clone()
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        // The accept loop has exited, so no connection is added after this.
        for connection in lock(&self.shared.connections).drain(..) {
            let _ = connection.shutdown(Shutdown::Both);
        }
    }
}

fn accept_loop(listener: UnixListener, shared: Arc<Shared>) {
    for stream in listener.incoming() {
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let (Ok(for_drop), Ok(for_close)) = (stream.try_clone(), stream.try_clone()) else {
            continue;
        };
        lock(&shared.connections).push(for_drop);
        let shared = Arc::clone(&shared);
        let _ = std::thread::Builder::new()
            .name("fake-daemon-conn".into())
            .spawn(move || {
                let _ = serve(stream, &shared);
                // Other handles (the drop list, subscribers) keep the socket
                // open; shut it down so the client reads EOF.
                let _ = for_close.shutdown(Shutdown::Both);
            });
    }
}

fn send(writer: &Writer, response: &ClientResponse) -> io::Result<()> {
    let mut line = serde_json::to_string(response).map_err(io::Error::other)?;
    line.push('\n');
    lock(writer).write_all(line.as_bytes())
}

fn error(request_id: Option<String>, code: &str, message: String) -> ClientResponse {
    ClientResponse::Error {
        data: ErrorData {
            request_id,
            code: code.to_owned(),
            message,
        },
    }
}

fn emit(state: &mut State, event: DaemonEvent) -> u64 {
    let data = EventData {
        event_id: state.events.len() as u64 + 1,
        event,
    };
    state.events.push(data.clone());
    let response = ClientResponse::Event { data: data.clone() };
    state.subscribers.retain(|s| send(s, &response).is_ok());
    data.event_id
}

fn serve(stream: UnixStream, shared: &Shared) -> io::Result<()> {
    let writer: Writer = Arc::new(Mutex::new(stream.try_clone()?));
    let mut negotiated = false;
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: ClientRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(e) if !negotiated => {
                let message = format!("the first message must be hello (invalid JSON line: {e})");
                return send(&writer, &error(None, HELLO_REQUIRED, message));
            }
            Err(e) => {
                send(
                    &writer,
                    &error(None, INVALID_REQUEST, format!("invalid JSON line: {e}")),
                )?;
                continue;
            }
        };
        let mut state = lock(&shared.state);
        state.requests.push(request.clone());
        if !negotiated {
            let ClientRequest::Hello { data } = request else {
                let reply = error(
                    None,
                    HELLO_REQUIRED,
                    "the first message must be hello".into(),
                );
                drop(state);
                return send(&writer, &reply);
            };
            match negotiate("client", &state.supported, &data.supported) {
                Ok(selected) => {
                    negotiated = true;
                    let reply = ClientResponse::Hello {
                        data: SelectedVersionData { selected },
                    };
                    drop(state);
                    send(&writer, &reply)?;
                    continue;
                }
                Err(mismatch) => {
                    drop(state);
                    return send(&writer, &error(None, VERSION_MISMATCH, mismatch.message()));
                }
            }
        }
        let replies = handle(&mut state, request, &writer);
        drop(state);
        for reply in replies {
            send(&writer, &reply)?;
        }
    }
    Ok(())
}

fn handle(state: &mut State, request: ClientRequest, writer: &Writer) -> Vec<ClientResponse> {
    match request {
        ClientRequest::Hello { .. } => vec![error(
            None,
            INVALID_REQUEST,
            "hello was already negotiated".into(),
        )],
        ClientRequest::Command { data } => {
            let request_id = data.request_id;
            match command(state, data.command) {
                Ok(result) => vec![ClientResponse::CommandResult {
                    data: ClientCommandResultData { request_id, result },
                }],
                Err((code, message)) => vec![error(Some(request_id), code, message)],
            }
        }
        ClientRequest::SubscribeEvents { data } => {
            // Sent under the state lock so no live event overtakes the backlog.
            let after = data.after_event_id.unwrap_or(0);
            for event in state.events.iter().filter(|e| e.event_id > after) {
                if send(
                    writer,
                    &ClientResponse::Event {
                        data: event.clone(),
                    },
                )
                .is_err()
                {
                    return Vec::new();
                }
            }
            state.subscribers.push(Arc::clone(writer));
            Vec::new()
        }
        ClientRequest::SubscribeTerminal { data } => vec![ClientResponse::TerminalSnapshot {
            data: TerminalSnapshotData {
                screen: format!("fake screen of {}", data.instance_id),
                instance_id: data.instance_id,
            },
        }],
        ClientRequest::TerminalInput { .. } => Vec::new(),
        ClientRequest::AnswerAsk { data } => {
            let Some(thread) = state.asks.get_mut(&data.ask_id) else {
                let message = format!("no open ask {}", data.ask_id);
                return vec![error(Some(data.request_id), UNKNOWN_ASK, message)];
            };
            if !thread.accepts(&data.reply) {
                let message = format!("ask {} does not accept this reply now", data.ask_id);
                return vec![error(Some(data.request_id), INVALID_REQUEST, message)];
            }
            thread.entries.push(AskEntry::Answer {
                from: "operator".into(),
                source: data.source,
                reply: data.reply,
            });
            let thread = thread.clone();
            emit(state, DaemonEvent::AskUpdated { data: thread });
            vec![ClientResponse::CommandResult {
                data: ClientCommandResultData {
                    request_id: data.request_id,
                    result: CommandResult::Accepted,
                },
            }]
        }
        ClientRequest::Unknown => vec![error(None, UNKNOWN_REQUEST, "unknown request type".into())],
    }
}

type CommandError = (&'static str, String);

fn command(state: &mut State, command: AgentCommand) -> Result<CommandResult, CommandError> {
    let result = match command {
        AgentCommand::Status => CommandResult::Status {
            data: StatusData {
                task_id: None,
                instance_id: None,
                summary: state.status_summary.clone(),
            },
        },
        AgentCommand::Inbox { after_message_id } => {
            let start = after_message_id
                .and_then(|id| state.inbox.iter().position(|m| m.message_id == id))
                .map_or(0, |i| i + 1);
            CommandResult::Messages {
                data: MessagesData {
                    messages: state.inbox[start..].to_vec(),
                },
            }
        }
        AgentCommand::TaskCreate { title, .. } => {
            state.next_id += 1;
            let task_id = format!("T-{}", state.next_id);
            emit(
                state,
                DaemonEvent::TaskChanged {
                    data: TaskChangedData {
                        task_id: task_id.clone(),
                        summary: format!("created: {title}"),
                    },
                },
            );
            CommandResult::TaskCreated {
                data: TaskCreatedData { task_id },
            }
        }
        AgentCommand::Ask { question, options } => {
            state.next_id += 1;
            let ask_id = format!("A-{}", state.next_id);
            let thread = AskThread {
                ask_id: ask_id.clone(),
                task_id: None,
                entries: vec![AskEntry::Question {
                    from: "agent".into(),
                    text: question,
                    options,
                }],
            };
            state.asks.insert(ask_id.clone(), thread.clone());
            emit(
                state,
                DaemonEvent::AttentionRequired {
                    data: AttentionRequiredData {
                        reason: "ask".into(),
                        task_id: None,
                        ask: Some(thread),
                        recap: None,
                    },
                },
            );
            CommandResult::AskCreated {
                data: AskCreatedData { ask_id },
            }
        }
        AgentCommand::Done { task_id, identity }
        | AgentCommand::ReviewApprove { task_id, identity } => {
            accept_result(state, &task_id, identity, "done")?
        }
        AgentCommand::Result {
            task_id, identity, ..
        }
        | AgentCommand::ReviewChanges {
            task_id, identity, ..
        } => accept_result(state, &task_id, identity, "result")?,
        AgentCommand::Unknown => {
            return Err((UNKNOWN_REQUEST, "unknown command".into()));
        }
        AgentCommand::Send { .. }
        | AgentCommand::AskFollowUp { .. }
        | AgentCommand::AskResolve { .. }
        | AgentCommand::Block { .. }
        | AgentCommand::Unblock { .. }
        | AgentCommand::Remind { .. } => CommandResult::Accepted,
    };
    Ok(result)
}

/// The event-identity rule: only a result for the current stage attempt is
/// accepted; anything else is `stale_result` and changes nothing.
fn accept_result(
    state: &mut State,
    task_id: &str,
    identity: Option<ResultIdentity>,
    what: &str,
) -> Result<CommandResult, CommandError> {
    let current = state.assignments.get(task_id);
    if identity.is_none() || identity.as_ref() != current {
        let expected = current.map_or_else(
            || "no result is expected".to_owned(),
            |c| format!("expected stage {} attempt {}", c.stage_id, c.attempt),
        );
        return Err((
            STALE_RESULT,
            format!("stale {what} for {task_id}: {expected}; nothing changed"),
        ));
    }
    let identity = state.assignments.remove(task_id).expect("checked above");
    emit(
        state,
        DaemonEvent::TaskChanged {
            data: TaskChangedData {
                task_id: task_id.to_owned(),
                summary: format!(
                    "{what} accepted for stage {} attempt {}",
                    identity.stage_id, identity.attempt
                ),
            },
        },
    );
    Ok(CommandResult::Accepted)
}

/// A blocking JSON Lines client for tests and demos. Deliberately separate
/// from `agend-client` so a bug there cannot hide behind the same code.
pub struct ProbeClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl ProbeClient {
    pub fn connect(path: &Path) -> io::Result<ProbeClient> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        Ok(ProbeClient {
            writer: stream.try_clone()?,
            reader: BufReader::new(stream),
        })
    }

    pub fn send(&mut self, request: &ClientRequest) -> io::Result<()> {
        let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())
    }

    /// Sends a raw line, for malformed-input tests.
    pub fn send_raw(&mut self, line: &str) -> io::Result<()> {
        self.writer.write_all(format!("{line}\n").as_bytes())
    }

    /// Next response; `Ok(None)` when the server closed the connection.
    pub fn recv(&mut self) -> io::Result<Option<ClientResponse>> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        serde_json::from_str(&line)
            .map(Some)
            .map_err(io::Error::other)
    }

    /// `send` then `recv`, failing if the server closed the connection.
    pub fn request(&mut self, request: &ClientRequest) -> io::Result<ClientResponse> {
        self.send(request)?;
        self.recv()?
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"))
    }
}
