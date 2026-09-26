//! Fake daemon: an in-process client protocol 1.1 server (JSON Lines over a
//! unix socket, D26) for testing `agend-client`, the CLI and the TUI without
//! a real daemon. Every message is an `agend_core::protocol::client` type
//! encoded with `serde_json`, so the wire shape is the real one; error codes
//! are `client::error_code`.
//!
//! Covered (the CLP contract, `contract::client`, runs against it and the
//! real `agend daemon`):
//! - `hello` first; version negotiation with `protocol::negotiate`; mismatch
//!   answers an `error` (`version_mismatch`) and closes; any other first
//!   line (another request or invalid JSON) answers `hello_required` and
//!   closes. The `caller` of `hello` makes the connection an agent's.
//! - `get_fleet`: the fleet view set with [`FakeDaemon::set_instance`],
//!   [`FakeDaemon::set_task`], [`FakeDaemon::add_attention`] and asks, as of
//!   the newest event id.
//! - Event ids start at a base (unix ms × 1000 when the fake starts); the
//!   first event is base + 1; the last [`RETAINED_EVENTS`] are kept.
//!   `subscribe_events`: no cursor replays the retained events; a cursor from
//!   "oldest − 1" to the newest id continues after it; anything else is
//!   `event_gap`. A subscriber more than [`RETAINED_EVENTS`] events behind
//!   gets `event_gap` and is closed; a write that makes no progress for
//!   [`WRITE_TIMEOUT`] closes the connection.
//! - `resolve_attention`: `forbidden` for an agent caller (before the id is
//!   looked at); `unknown_attention` for an unknown id or an action the item
//!   does not list; otherwise the item leaves the list, `attention_resolved`
//!   is emitted and the reply is `accepted`.
//! - `command`: `status`, `inbox`, `send`, `task_create`, `ask`, the other
//!   agent commands (`accepted`), and the result commands (`done`, `result`,
//!   `review_approve`, `review_changes`) with the event-identity rule: the
//!   result must carry the `stage_id` and `attempt` of the current
//!   assignment (`FakeDaemon::assign`), otherwise `stale_result` and nothing
//!   changes; an accepted result consumes the assignment. (The real daemon
//!   answers `not_supported` until gates 9 and 10.)
//! - `subscribe_terminal`: one `terminal_snapshot`, for any instance id.
//! - `terminal_input`: `not_supported` (gate 11's attach view).
//! - `answer_ask`, for asks from the `ask` command or `FakeDaemon::open_ask`
//!   (an ask with a task and a context recap, as a bound agent's would be).
//!
//! Not covered: terminal byte streaming, persistence.
//!
//! Must NOT: share code paths with the real server beyond `agend_core::protocol`.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agend_core::protocol::ask::{AskEntry, AskThread, ContextRecap};
use agend_core::protocol::client::{
    AgentCommand, AskCreatedData, AttentionRequiredData, AttentionResolvedData,
    ClientCommandResultData, ClientRequest, ClientResponse, CommandResult, DaemonEvent, ErrorData,
    EventData, FleetData, FleetView, InboxMessage, InstanceChangedData, InstanceView, MessagesData,
    RETAINED_EVENTS, ResultIdentity, SUPPORTED_VERSIONS, SelectedVersionData, StatusData,
    TaskChangedData, TaskCreatedData, TaskView, TeamView, TerminalSnapshotData, error_code,
    order_attention,
};
use agend_core::protocol::{ProtocolVersion, negotiate};

use crate::fakes::lock;
use crate::tempdir::TempDir;

/// A write that makes no progress for this long closes the connection.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// What `resolve_attention` from an agent gets (the real daemon says the
/// same).
pub const OPERATOR_ONLY: &str =
    "only the operator can resolve needs-you items; ask the operator with agend ask";

type Writer = Arc<Mutex<UnixStream>>;

struct Subscriber {
    queue: SyncSender<EventData>,
    lagged: Arc<AtomicBool>,
}

struct State {
    supported: Vec<ProtocolVersion>,
    requests: Vec<ClientRequest>,
    /// Event id base: the first event is `start + 1`.
    start: u64,
    latest: u64,
    events: VecDeque<EventData>,
    subscribers: Vec<Subscriber>,
    assignments: BTreeMap<String, ResultIdentity>,
    status_summary: String,
    inbox: Vec<InboxMessage>,
    asks: BTreeMap<String, AskThread>,
    next_id: u64,
    instances: Vec<InstanceView>,
    tasks: Vec<TaskView>,
    attention: Vec<AttentionRequiredData>,
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
    _dir: Option<TempDir>,
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl FakeDaemon {
    /// Listens on `daemon.sock` in a new temp directory.
    pub fn start() -> io::Result<FakeDaemon> {
        let dir = TempDir::new("fd")?;
        let mut daemon = FakeDaemon::start_at(&dir.path().join("daemon.sock"))?;
        daemon._dir = Some(dir);
        Ok(daemon)
    }

    /// Listens on `socket_path` (a leftover socket file there is replaced),
    /// for tests that restart the "daemon" on the same path.
    pub fn start_at(socket_path: &Path) -> io::Result<FakeDaemon> {
        match std::fs::remove_file(socket_path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        let listener = UnixListener::bind(socket_path)?;
        let start = now_unix_ms() * 1000;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                supported: SUPPORTED_VERSIONS.to_vec(),
                requests: Vec::new(),
                start,
                latest: start,
                events: VecDeque::new(),
                subscribers: Vec::new(),
                assignments: BTreeMap::new(),
                status_summary: "fake daemon: idle".into(),
                inbox: Vec::new(),
                asks: BTreeMap::new(),
                next_id: 0,
                instances: Vec::new(),
                tasks: Vec::new(),
                attention: Vec::new(),
            }),
            stopping: AtomicBool::new(false),
            connections: Mutex::new(Vec::new()),
        });
        let accept_shared = Arc::clone(&shared);
        let accept = std::thread::Builder::new()
            .name("fake-daemon-accept".into())
            .spawn(move || accept_loop(listener, accept_shared))?;
        Ok(FakeDaemon {
            socket_path: socket_path.to_path_buf(),
            shared,
            accept: Some(accept),
            _dir: None,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// The event id base: the first event is this + 1.
    pub fn event_id_start(&self) -> u64 {
        lock(&self.shared.state).start
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

    /// Adds or replaces an instance of the fleet view and emits
    /// `instance_changed` carrying it. Returns the event id.
    pub fn set_instance(&self, instance: InstanceView) -> u64 {
        let mut state = lock(&self.shared.state);
        state
            .instances
            .retain(|i| i.instance_id != instance.instance_id);
        state.instances.push(instance.clone());
        emit(
            &mut state,
            DaemonEvent::InstanceChanged {
                data: InstanceChangedData {
                    instance_id: instance.instance_id.clone(),
                    summary: instance.state.as_str().into(),
                    instance: Some(instance),
                },
            },
        )
    }

    /// Adds or replaces a task of the fleet view and emits `task_changed`
    /// carrying it. Returns the event id.
    pub fn set_task(&self, task: TaskView) -> u64 {
        let mut state = lock(&self.shared.state);
        state.tasks.retain(|t| t.task_id != task.task_id);
        state.tasks.push(task.clone());
        emit(
            &mut state,
            DaemonEvent::TaskChanged {
                data: TaskChangedData {
                    task_id: task.task_id.clone(),
                    summary: task.status.clone(),
                    task: Some(task),
                },
            },
        )
    }

    /// Adds a needs-you item (not an ask) and emits `attention_required`.
    /// `resolve_attention` takes it off with one of its `actions`. Returns
    /// the event id.
    pub fn add_attention(&self, item: AttentionRequiredData) -> u64 {
        let mut state = lock(&self.shared.state);
        state.attention.push(item.clone());
        emit(&mut state, DaemonEvent::AttentionRequired { data: item })
    }

    /// Opens a needs-you ask the way the daemon does after `agend ask` from
    /// an agent bound to a task: the thread becomes answerable with
    /// `answer_ask`, and an `attention_required` event carries the thread,
    /// its task and `recap`. Returns the event id.
    pub fn open_ask(&self, thread: AskThread, recap: Option<ContextRecap>) -> u64 {
        let mut state = lock(&self.shared.state);
        let item = ask_item(thread.clone(), recap);
        state.asks.insert(thread.ask_id.clone(), thread);
        state.attention.push(item.clone());
        emit(&mut state, DaemonEvent::AttentionRequired { data: item })
    }

    /// The fleet view `get_fleet` answers now.
    pub fn fleet(&self) -> FleetView {
        fleet(&lock(&self.shared.state))
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
        lock(&self.shared.state).subscribers.clear();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn ask_item(thread: AskThread, recap: Option<ContextRecap>) -> AttentionRequiredData {
    AttentionRequiredData {
        reason: "ask".into(),
        task_id: thread.task_id.clone(),
        attention_id: Some(thread.ask_id.clone()),
        ask: Some(thread),
        recap,
        unblocks: None,
        waiting_since_unix_ms: Some(now_unix_ms()),
        if_ignored: None,
        actions: Vec::new(),
        instance_id: None,
    }
}

fn fleet(state: &State) -> FleetView {
    let mut teams: Vec<String> = vec![agend_core::model::DEFAULT_TEAM.to_owned()];
    for team in state
        .tasks
        .iter()
        .map(|t| &t.team_id)
        .chain(state.instances.iter().map(|i| &i.team_id))
    {
        if !teams.contains(team) {
            teams.push(team.clone());
        }
    }
    let mut attention = state.attention.clone();
    order_attention(&mut attention);
    FleetView {
        as_of_event_id: state.latest,
        teams: teams
            .into_iter()
            .map(|team_id| TeamView { team_id })
            .collect(),
        tasks: state.tasks.clone(),
        instances: state.instances.clone(),
        attention,
    }
}

fn accept_loop(listener: UnixListener, shared: Arc<Shared>) {
    for stream in listener.incoming() {
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
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
    write_line(&lock(writer), response)
}

/// Writes one response to a stream whose writer lock the caller holds.
fn write_line(mut stream: &UnixStream, response: &ClientResponse) -> io::Result<()> {
    let mut line = serde_json::to_string(response).map_err(io::Error::other)?;
    line.push('\n');
    let written = stream.write_all(line.as_bytes());
    if written.is_err() {
        // A write timeout or a gone client: close the whole connection.
        let _ = stream.shutdown(Shutdown::Both);
    }
    written
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
    state.latest += 1;
    let data = EventData {
        event_id: state.latest,
        event,
    };
    state.events.push_back(data.clone());
    if state.events.len() > RETAINED_EVENTS {
        state.events.pop_front();
    }
    state
        .subscribers
        .retain(|s| match s.queue.try_send(data.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                s.lagged.store(true, Ordering::SeqCst);
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        });
    data.event_id
}

/// Sends queued events to one subscriber until its queue is dropped; a
/// lagging subscriber gets `event_gap` next and its connection is closed.
fn forward(writer: Writer, queue: Receiver<EventData>, lagged: Arc<AtomicBool>) {
    let gap = |writer: &Writer| {
        let message = format!("this client fell more than {RETAINED_EVENTS} events behind");
        let _ = send(writer, &error(None, error_code::EVENT_GAP, message));
        let _ = lock(writer).shutdown(Shutdown::Both);
    };
    for data in queue.iter() {
        if lagged.load(Ordering::SeqCst) {
            return gap(&writer);
        }
        if send(&writer, &ClientResponse::Event { data }).is_err() {
            return;
        }
    }
    if lagged.load(Ordering::SeqCst) {
        gap(&writer);
    }
}

/// `Ok(())` when a subscription may continue after `after` (see the module
/// docs), otherwise the `event_gap` message.
fn check_cursor(state: &State, after: u64) -> Result<(), String> {
    let oldest = state
        .events
        .front()
        .map_or(state.latest + 1, |e| e.event_id);
    if after.saturating_add(1) >= oldest && after <= state.latest {
        Ok(())
    } else {
        Err(format!(
            "cannot continue after event {after} (this daemon has events {} to {}); fetch the fleet view again",
            oldest, state.latest
        ))
    }
}

struct Connection {
    writer: Writer,
    negotiated: bool,
    caller: Option<String>,
}

fn serve(stream: UnixStream, shared: &Shared) -> io::Result<()> {
    let mut conn = Connection {
        writer: Arc::new(Mutex::new(stream.try_clone()?)),
        negotiated: false,
        caller: None,
    };
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: ClientRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(e) if !conn.negotiated => {
                let message = format!("the first message must be hello (invalid JSON line: {e})");
                return send(
                    &conn.writer,
                    &error(None, error_code::HELLO_REQUIRED, message),
                );
            }
            Err(e) => {
                let message = format!("invalid JSON line: {e}");
                send(
                    &conn.writer,
                    &error(None, error_code::INVALID_REQUEST, message),
                )?;
                continue;
            }
        };
        // The writer is held until the replies are written, so an event this
        // request causes reaches this client after the reply, as from the
        // real server.
        let writer = Arc::clone(&conn.writer);
        let stream = lock(&writer);
        let mut state = lock(&shared.state);
        state.requests.push(request.clone());
        if !conn.negotiated {
            let ClientRequest::Hello { data } = request else {
                let reply = error(
                    None,
                    error_code::HELLO_REQUIRED,
                    "the first message must be hello".into(),
                );
                drop(state);
                return write_line(&stream, &reply);
            };
            match negotiate("client", &state.supported, &data.supported) {
                Ok(selected) => {
                    conn.negotiated = true;
                    conn.caller = data.caller;
                    let reply = ClientResponse::Hello {
                        data: SelectedVersionData { selected },
                    };
                    drop(state);
                    write_line(&stream, &reply)?;
                    continue;
                }
                Err(mismatch) => {
                    drop(state);
                    let reply = error(None, error_code::VERSION_MISMATCH, mismatch.message());
                    return write_line(&stream, &reply);
                }
            }
        }
        let replies = handle(&mut state, request, &conn);
        drop(state);
        for reply in replies {
            write_line(&stream, &reply)?;
        }
    }
    Ok(())
}

fn accepted(request_id: String) -> ClientResponse {
    ClientResponse::CommandResult {
        data: ClientCommandResultData {
            request_id,
            result: CommandResult::Accepted,
        },
    }
}

fn handle(state: &mut State, request: ClientRequest, conn: &Connection) -> Vec<ClientResponse> {
    match request {
        ClientRequest::Hello { .. } => vec![error(
            None,
            error_code::INVALID_REQUEST,
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
        ClientRequest::GetFleet { data } => vec![ClientResponse::Fleet {
            data: FleetData {
                request_id: data.request_id,
                fleet: fleet(state),
            },
        }],
        ClientRequest::SubscribeEvents { data } => {
            if let Some(after) = data.after_event_id
                && let Err(message) = check_cursor(state, after)
            {
                return vec![error(None, error_code::EVENT_GAP, message)];
            }
            let after = data.after_event_id.unwrap_or(0);
            let backlog: Vec<EventData> = state
                .events
                .iter()
                .filter(|e| e.event_id > after)
                .cloned()
                .collect();
            let (queue, receiver) = sync_channel(backlog.len() + RETAINED_EVENTS);
            for event in backlog {
                let _ = queue.try_send(event);
            }
            let lagged = Arc::new(AtomicBool::new(false));
            let writer = Arc::clone(&conn.writer);
            let flag = Arc::clone(&lagged);
            let started = std::thread::Builder::new()
                .name("fake-daemon-events".into())
                .spawn(move || forward(writer, receiver, flag));
            if started.is_ok() {
                state.subscribers.push(Subscriber { queue, lagged });
            }
            Vec::new()
        }
        ClientRequest::SubscribeTerminal { data } => vec![ClientResponse::TerminalSnapshot {
            data: TerminalSnapshotData {
                screen: format!("fake screen of {}", data.instance_id),
                instance_id: data.instance_id,
            },
        }],
        ClientRequest::TerminalInput { .. } => vec![error(
            None,
            error_code::NOT_SUPPORTED,
            "terminal input arrives with the attach view (gate 11); nothing was written".into(),
        )],
        ClientRequest::AnswerAsk { data } => {
            let Some(thread) = state.asks.get_mut(&data.ask_id) else {
                let message = format!("no open ask {}", data.ask_id);
                return vec![error(
                    Some(data.request_id),
                    error_code::UNKNOWN_ASK,
                    message,
                )];
            };
            if !thread.accepts(&data.reply) {
                let message = format!("ask {} does not accept this reply now", data.ask_id);
                return vec![error(
                    Some(data.request_id),
                    error_code::INVALID_REQUEST,
                    message,
                )];
            }
            thread.entries.push(AskEntry::Answer {
                from: "operator".into(),
                source: data.source,
                reply: data.reply,
            });
            let thread = thread.clone();
            if let Some(item) = state
                .attention
                .iter_mut()
                .find(|i| i.attention_id.as_deref() == Some(thread.ask_id.as_str()))
            {
                item.ask = Some(thread.clone());
            }
            emit(state, DaemonEvent::AskUpdated { data: thread });
            vec![accepted(data.request_id)]
        }
        ClientRequest::ResolveAttention { data } => {
            if conn.caller.is_some() {
                return vec![error(
                    Some(data.request_id),
                    error_code::FORBIDDEN,
                    OPERATOR_ONLY.into(),
                )];
            }
            let found = state.attention.iter().position(|i| {
                i.attention_id.as_deref() == Some(data.attention_id.as_str())
                    && i.actions.contains(&data.action)
            });
            let Some(index) = found else {
                let message = format!(
                    "no needs-you item {} with action {}",
                    data.attention_id,
                    data.action.as_str()
                );
                return vec![error(
                    Some(data.request_id),
                    error_code::UNKNOWN_ATTENTION,
                    message,
                )];
            };
            state.attention.remove(index);
            emit(
                state,
                DaemonEvent::AttentionResolved {
                    data: AttentionResolvedData {
                        attention_id: data.attention_id,
                        action: data.action,
                    },
                },
            );
            vec![accepted(data.request_id)]
        }
        ClientRequest::Unknown => vec![error(
            None,
            error_code::UNKNOWN_REQUEST,
            "unknown request type".into(),
        )],
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
                        task: None,
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
            let item = ask_item(thread, None);
            state.attention.push(item.clone());
            emit(state, DaemonEvent::AttentionRequired { data: item });
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
            return Err((error_code::UNKNOWN_REQUEST, "unknown command".into()));
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
            error_code::STALE_RESULT,
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
                task: None,
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
    /// A line read partly before a timeout, completed by the next read.
    partial: Vec<u8>,
}

impl ProbeClient {
    pub fn connect(path: &Path) -> io::Result<ProbeClient> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        Ok(ProbeClient {
            writer: stream.try_clone()?,
            reader: BufReader::new(stream),
            partial: Vec::new(),
        })
    }

    /// Connects and says `hello` (`caller`: `None` for the operator);
    /// returns the client and the selected version.
    pub fn hello(path: &Path, caller: Option<&str>) -> io::Result<(ProbeClient, ProtocolVersion)> {
        let mut client = ProbeClient::connect(path)?;
        match client.request(&ClientRequest::hello_as(caller.map(str::to_owned)))? {
            ClientResponse::Hello { data } => Ok((client, data.selected)),
            other => Err(io::Error::other(format!("hello answered {other:?}"))),
        }
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
        loop {
            if self.reader.read_until(b'\n', &mut self.partial)? == 0 {
                return Ok(None);
            }
            if self.partial.last() != Some(&b'\n') {
                continue;
            }
            let line = std::mem::take(&mut self.partial);
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            return serde_json::from_slice(&line)
                .map(Some)
                .map_err(io::Error::other);
        }
    }

    /// [`ProbeClient::recv`] waiting at most `timeout` for the next line; a
    /// timeout is an error of kind `WouldBlock` or `TimedOut` (a line read in
    /// part is kept for the next call).
    pub fn recv_within(&mut self, timeout: Duration) -> io::Result<Option<ClientResponse>> {
        // macOS refuses a new timeout (EINVAL) once the server has closed the
        // connection; what it sent before closing is still there to read.
        let _ = self
            .reader
            .get_ref()
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))));
        self.recv()
    }

    /// `send` then `recv`, failing if the server closed the connection.
    pub fn request(&mut self, request: &ClientRequest) -> io::Result<ClientResponse> {
        self.send(request)?;
        self.recv()?
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"))
    }
}
