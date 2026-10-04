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
//!   is emitted and the reply is `accepted`. With
//!   [`FakeDaemon::hold_resolved_events`] the event waits until
//!   [`FakeDaemon::release_resolved_events`] (gate 11 B P4: a client must
//!   drop an item on the event, not on `accepted`).
//! - `command` (gate 9: agents only; the operator gets `forbidden`, as from
//!   the real daemon): `status` (the caller's instance, or the assignment's
//!   task and ticket identity), `send` and `inbox` like the real daemon
//!   (a client message id must be a UUID v4; one message per id, the same id
//!   with other content is `invalid_request`; `inbox` by order, `--after` an
//!   id the caller has no message with is `unknown_message`), `task_create`,
//!   `ask`, the other agent commands (`accepted`), and the result commands
//!   (`done`, `result`, `review_approve`, `review_changes`) with the
//!   event-identity rule: the result must carry the `stage_id` and `attempt`
//!   of the current assignment (`FakeDaemon::assign`), otherwise
//!   `stale_result` and nothing changes; an accepted result consumes the
//!   assignment. (The real daemon answers `not_supported` for everything but
//!   `status`, `send` and `inbox` until gate 10.)
//! - `operator` (gate 9: the operator only; an agent gets `forbidden`):
//!   `instance_add` / `instance_remove` change the fleet view
//!   (`instance_exists`, `unknown_instance`); `daemon_restart` checks the
//!   binary with `<binary> --version` (it must print `agend …`; otherwise
//!   `preflight_failed` like the real one), answers `restarting`, then
//!   closes every connection and starts a new boot (new `boot_id`, no
//!   events); `task_cancel` is `not_supported`.
//! - `hello` names the daemon: `agend <version> (fake daemon)`, this
//!   process's pid, and the boot id (the event id base).
//! - `subscribe_terminal` (gate 11 B P1): an instance of the fleet view gets
//!   its screen ([`FakeDaemon::set_screen`]; `fake screen of <id>` until
//!   set), then every [`FakeDaemon::push_terminal_bytes`] as
//!   `terminal_bytes`, which also appends the text to its screen (the next
//!   subscription shows it). Any other id: `no_terminal`. A new
//!   subscription on the same connection replaces the old one, also when it
//!   fails. A subscriber more than [`TERMINAL_CHUNKS`] chunks behind is
//!   closed, like an event subscriber.
//! - `terminal_input` (gate 11 B P6), in this order: an agent caller gets
//!   `forbidden`; an instance not in the fleet view, or `failed`,
//!   `no_terminal`; a codex
//!   instance `not_supported` (until U17); input whose holder request line
//!   would be over `protocol::holder::MAX_REQUEST_LINE` `invalid_request`;
//!   otherwise the bytes are recorded
//!   ([`FakeDaemon::terminal_inputs`]) and nothing is answered. Errors carry
//!   no request id.
//! - `answer_ask`, for asks from the `ask` command or `FakeDaemon::open_ask`
//!   (an ask with a task and a context recap, as a bound agent's would be).
//!
//! Not covered: a PTY (input is recorded, not echoed), persistence.
//!
//! Must NOT: share code paths with the real server beyond `agend_core::protocol`.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, BufRead, BufReader, Read, Write};
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
    AgentCommand, AgentState, AskCreatedData, AttentionRequiredData, AttentionResolvedData,
    ClientCommandResultData, ClientRequest, ClientResponse, CommandResult, DaemonEvent, ErrorData,
    EventData, FleetData, FleetView, InboxMessage, InstanceAddedData, InstanceChangedData,
    InstanceView, MAX_MESSAGE_BYTES, MessageLevel, MessagesData, OperatorCommand, RETAINED_EVENTS,
    RestartingData, ResultIdentity, SUPPORTED_VERSIONS, SelectedVersionData, StatusData,
    TaskChangedData, TaskCreatedData, TaskView, TeamView, TerminalBytesData, TerminalSnapshotData,
    error_code, is_uuid_v4, message_too_long, order_attention, uuid_v4,
};
use agend_core::protocol::holder::{
    HolderRequest, MAX_REQUEST_LINE, OperatorTerminalInputData, operator_input_too_long,
};
use agend_core::protocol::{ProtocolVersion, negotiate};

use crate::fakes::lock;
use crate::tempdir::TempDir;

/// A write that makes no progress for this long closes the connection.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// What `resolve_attention` from an agent gets (the real daemon says the
/// same).
pub const OPERATOR_ONLY: &str = "only the operator can resolve needs-you items; ask the operator";
/// What `terminal_input` from an agent gets (the real daemon says the same).
pub const TYPE_OPERATOR_ONLY: &str = "only the operator can type into an agent's terminal";
/// What `terminal_input` into a codex instance gets (gate 11 B P6).
pub const CODEX_INPUT: &str = "Codex terminal input requires the approved CLI 0.159.3 and a connected link with durable own-clientId receipts; nothing was written";
/// PTY chunks a terminal subscriber may fall behind before it is closed.
pub const TERMINAL_CHUNKS: usize = 256;

mod full_terminal;

type Writer = Arc<Mutex<UnixStream>>;

struct Subscriber {
    connection: u64,
    queue: SyncSender<EventData>,
    lagged: Arc<AtomicBool>,
}

/// One connection's terminal subscription.
struct TerminalSubscriber {
    connection: u64,
    instance: String,
    queue: SyncSender<String>,
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
    /// Set with [`FakeDaemon::set_status`]; otherwise the summary is made
    /// like the real daemon's.
    status_summary: Option<String>,
    /// Every message sent, in order (the real daemon's `seq`).
    messages: Vec<FakeMessage>,
    /// Where `instance_add` puts a workspace by default.
    home: PathBuf,
    asks: BTreeMap<String, AskThread>,
    next_id: u64,
    instances: Vec<InstanceView>,
    tasks: Vec<TaskView>,
    attention: Vec<AttentionRequiredData>,
    screens: BTreeMap<String, String>,
    terminals: Vec<TerminalSubscriber>,
    inputs: Vec<(String, Vec<u8>)>,
    hold_resolved: bool,
    held_resolved: Vec<DaemonEvent>,
    next_connection: u64,
    full_terminals: BTreeMap<String, full_terminal::Endpoint>,
    full_scopes: BTreeMap<u64, full_terminal::Scope>,
}

/// A message as the fake keeps it.
#[derive(Clone)]
struct FakeMessage {
    id: String,
    from: String,
    to: String,
    body: String,
    level: MessageLevel,
    created_at_unix_ms: u64,
}

struct Shared {
    state: Mutex<State>,
    stopping: AtomicBool,
    /// Connections being served now.
    open: std::sync::atomic::AtomicUsize,
    /// Every accepted connection, so drop can close them.
    /// (accept number, a clone), kept while the connection is served.
    connections: Mutex<Vec<(u64, UnixStream)>>,
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
        // `$AGEND_HOME/run/daemon.sock` → `$AGEND_HOME`.
        let parent = socket_path.parent().unwrap_or(Path::new("/"));
        let home = match parent.file_name() {
            Some(name) if name == "run" => parent.parent().unwrap_or(parent),
            _ => parent,
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                supported: SUPPORTED_VERSIONS.to_vec(),
                requests: Vec::new(),
                start,
                latest: start,
                events: VecDeque::new(),
                subscribers: Vec::new(),
                assignments: BTreeMap::new(),
                status_summary: None,
                messages: Vec::new(),
                home: home.to_path_buf(),
                asks: BTreeMap::new(),
                next_id: 0,
                instances: Vec::new(),
                tasks: Vec::new(),
                attention: Vec::new(),
                screens: BTreeMap::new(),
                terminals: Vec::new(),
                inputs: Vec::new(),
                hold_resolved: false,
                held_resolved: Vec::new(),
                next_connection: 0,
                full_terminals: BTreeMap::new(),
                full_scopes: BTreeMap::new(),
            }),
            stopping: AtomicBool::new(false),
            open: std::sync::atomic::AtomicUsize::new(0),
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

    /// Installs a real frame producer for C-path tests, without a parser
    /// dependency in testkit. No idealized frame is synthesized by the fake.
    pub fn set_terminal_producer(
        &self,
        instance: &str,
        producer: impl agend_core::traits::TerminalProducer + 'static,
    ) -> io::Result<()> {
        let endpoint = full_terminal::Endpoint::start(instance, Box::new(producer))?;
        lock(&self.shared.state)
            .full_terminals
            .insert(instance.into(), endpoint);
        self.set_supported_versions(&[agend_core::protocol::client::V1_4]);
        Ok(())
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

    /// The `status` summary from now on (default: made like the real
    /// daemon's from the caller's instance).
    pub fn set_status(&self, summary: &str) {
        lock(&self.shared.state).status_summary = Some(summary.to_owned());
    }

    /// The ids of the messages to `to`, in order (each id once).
    pub fn message_ids_to(&self, to: &str) -> Vec<String> {
        lock(&self.shared.state)
            .messages
            .iter()
            .filter(|m| m.to == to)
            .map(|m| m.id.clone())
            .collect()
    }

    /// Appends an event to the log and sends it to every subscriber.
    pub fn emit(&self, event: DaemonEvent) -> u64 {
        emit(&mut lock(&self.shared.state), event)
    }

    /// Adds or replaces an instance of the fleet view and emits
    /// `instance_changed` carrying it. Returns the event id.
    pub fn set_instance(&self, instance: InstanceView) -> u64 {
        let mut state = lock(&self.shared.state);
        if let Some(endpoint) = state.full_terminals.get(&instance.instance_id) {
            endpoint.set_live(instance.state != AgentState::Failed);
        }
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

    /// The screen `subscribe_terminal` answers for `instance` from now on.
    pub fn set_screen(&self, instance: &str, screen: &str) {
        lock(&self.shared.state)
            .screens
            .insert(instance.to_owned(), screen.to_owned());
    }

    /// PTY output of `instance`: sent as `terminal_bytes` to its
    /// subscribers and appended (as text, without `\r`) to its screen.
    pub fn push_terminal_bytes(&self, instance: &str, bytes: &[u8]) {
        use base64::Engine;
        let mut state = lock(&self.shared.state);
        let text = String::from_utf8_lossy(bytes).replace('\r', "");
        let screen = state
            .screens
            .entry(instance.to_owned())
            .or_insert_with(|| default_screen(instance));
        screen.push_str(&text);
        let chunk = base64::engine::general_purpose::STANDARD.encode(bytes);
        state.terminals.retain(|t| {
            if t.instance != instance {
                return true;
            }
            match t.queue.try_send(chunk.clone()) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => {
                    t.lagged.store(true, Ordering::SeqCst);
                    false
                }
                Err(TrySendError::Disconnected(_)) => false,
            }
        });
    }

    /// Every accepted `terminal_input`: (instance, decoded bytes), in order.
    pub fn terminal_inputs(&self) -> Vec<(String, Vec<u8>)> {
        lock(&self.shared.state).inputs.clone()
    }

    /// Client connections open now (the TUI tests check none is left).
    pub fn open_connections(&self) -> usize {
        self.shared.open.load(Ordering::SeqCst)
    }

    /// Closes every connection with a terminal subscription, as the daemon
    /// does to a subscriber that fell [`TERMINAL_CHUNKS`] behind (no error
    /// line; the client reads the end of the connection).
    pub fn drop_terminal_subscribers(&self) {
        for t in lock(&self.shared.state).terminals.drain(..) {
            t.lagged.store(true, Ordering::SeqCst);
        }
    }

    /// Terminal subscriptions open now (one per connection at most).
    pub fn terminal_subscribers(&self) -> usize {
        lock(&self.shared.state).terminals.len()
    }

    /// While held, `resolve_attention` answers `accepted` and takes the item
    /// off the list, but its `attention_resolved` waits for
    /// [`FakeDaemon::release_resolved_events`].
    pub fn hold_resolved_events(&self, hold: bool) {
        lock(&self.shared.state).hold_resolved = hold;
    }

    /// Emits the held `attention_resolved` events, in order.
    pub fn release_resolved_events(&self) {
        let mut state = lock(&self.shared.state);
        for event in std::mem::take(&mut state.held_resolved) {
            emit(&mut state, event);
        }
    }

    /// Every request received, in order, from all connections.
    pub fn requests(&self) -> Vec<ClientRequest> {
        lock(&self.shared.state).requests.clone()
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        // The accept loop has exited, so no connection is added after this.
        for (_, connection) in lock(&self.shared.connections).drain(..) {
            let _ = connection.shutdown(Shutdown::Both);
        }
        {
            let mut state = lock(&self.shared.state);
            for scope in state.full_scopes.values() {
                scope.close();
            }
            state.full_scopes.clear();
            state.subscribers.clear();
            state.terminals.clear();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn default_screen(instance: &str) -> String {
    format!("fake screen of {instance}")
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

/// How often a stopping accept loop looks at its stop flag.
pub(crate) const ACCEPT_POLL: Duration = Duration::from_millis(5);

/// The next connection, polling `listener` (non-blocking) until one comes
/// or `stopping` is set. Stopping needs no new descriptor, so it works when
/// the process has none left (a `connect` to wake a blocked `accept`
/// would fail then); failed accepts (out of descriptors) are retried.
pub(crate) fn accept_or_stop(listener: &UnixListener, stopping: &AtomicBool) -> Option<UnixStream> {
    let _ = listener.set_nonblocking(true);
    loop {
        if stopping.load(Ordering::SeqCst) {
            return None;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                // Accepted sockets inherit non-blocking mode on macOS.
                let _ = stream.set_nonblocking(false);
                return Some(stream);
            }
            Err(_) => std::thread::sleep(ACCEPT_POLL),
        }
    }
}

fn accept_loop(listener: UnixListener, shared: Arc<Shared>) {
    let mut accepted = 0u64;
    while let Some(stream) = accept_or_stop(&listener, &shared.stopping) {
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let (Ok(for_drop), Ok(for_close)) = (stream.try_clone(), stream.try_clone()) else {
            continue;
        };
        accepted += 1;
        let number = accepted;
        lock(&shared.connections).push((number, for_drop));
        let shared = Arc::clone(&shared);
        shared.open.fetch_add(1, Ordering::SeqCst);
        let _ = std::thread::Builder::new()
            .name("fake-daemon-conn".into())
            .spawn(move || {
                let _ = serve(stream, &shared);
                shared.open.fetch_sub(1, Ordering::SeqCst);
                // Other handles (subscribers) keep the socket open; shut it
                // down so the client reads EOF, and let go of its clone (a
                // long-lived fake would otherwise hold a descriptor per
                // connection it ever accepted).
                let _ = for_close.shutdown(Shutdown::Both);
                lock(&shared.connections).retain(|(n, _)| *n != number);
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
    if matches!(
        response,
        ClientResponse::TerminalFrame { .. } | ClientResponse::TerminalControlAck { .. }
    ) && line.len() > agend_core::protocol::terminal::MAX_FRAME_LINE
    {
        let rejected = error(
            None,
            error_code::FRAME_TOO_LARGE,
            "complete terminal response exceeds 8 MiB".into(),
        );
        let _ = write_line(stream, &rejected);
        let _ = stream.shutdown(Shutdown::Both);
        return Err(io::Error::other("complete frame exceeds 8 MiB"));
    }
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

/// Sends queued PTY chunks of `instance` to one terminal subscriber until
/// its queue is dropped; a lagging one is closed (no `event_gap`).
fn forward_terminal(
    writer: Writer,
    instance: String,
    queue: Receiver<String>,
    lagged: Arc<AtomicBool>,
) {
    for bytes_base64 in queue.iter() {
        if lagged.load(Ordering::SeqCst) {
            break;
        }
        let data = TerminalBytesData {
            instance_id: instance.clone(),
            bytes_base64,
        };
        if send(&writer, &ClientResponse::TerminalBytes { data }).is_err() {
            return;
        }
    }
    if lagged.load(Ordering::SeqCst) {
        let _ = lock(&writer).shutdown(Shutdown::Both);
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
    id: u64,
    writer: Writer,
    negotiated: bool,
    caller: Option<String>,
    selected: ProtocolVersion,
    full_scope: full_terminal::Scope,
}

fn serve(stream: UnixStream, shared: &Shared) -> io::Result<()> {
    let id = {
        let mut state = lock(&shared.state);
        state.next_connection += 1;
        state.next_connection
    };
    let writer = Arc::new(Mutex::new(stream.try_clone()?));
    let full_scope = full_terminal::Scope::start(id, Arc::clone(&writer))?;
    let mut conn = Connection {
        id,
        writer,
        negotiated: false,
        caller: None,
        selected: agend_core::protocol::client::V1,
        full_scope,
    };
    lock(&shared.state)
        .full_scopes
        .insert(id, conn.full_scope.clone());
    let served = serve_lines(stream, shared, &mut conn);
    conn.full_scope.close();
    // Its subscriptions end with the connection (their forwarders hold its
    // descriptor until their queue is dropped).
    let mut state = lock(&shared.state);
    state.full_scopes.remove(&id);
    state.terminals.retain(|t| t.connection != id);
    state.subscribers.retain(|s| s.connection != id);
    drop(state);
    served
}

fn serve_lines(stream: UnixStream, shared: &Shared, conn: &mut Connection) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        let read = (&mut reader)
            .take(agend_core::protocol::client::MAX_LINE_BYTES as u64 + 1)
            .read_line(&mut line)?;
        if read == 0 {
            break;
        }
        if read > agend_core::protocol::client::MAX_LINE_BYTES {
            send(
                &conn.writer,
                &error(
                    None,
                    error_code::INVALID_REQUEST,
                    "client line exceeds 8 MiB".into(),
                ),
            )?;
            return Ok(());
        }
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
        if conn.negotiated && full_terminal::dispatch(shared, conn, &request, read)? {
            lock(&shared.state).requests.push(request);
            continue;
        }
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
                    conn.selected = selected;
                    conn.caller = data.caller;
                    let reply = ClientResponse::Hello {
                        data: SelectedVersionData {
                            selected,
                            daemon_version: Some(format!(
                                "agend {} (fake daemon)",
                                env!("CARGO_PKG_VERSION")
                            )),
                            daemon_pid: Some(std::process::id()),
                            boot_id: Some(state.start),
                        },
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
        // `daemon_restart` checks its binary without holding the state.
        if let ClientRequest::Operator { data } = &request
            && conn.caller.is_none()
            && let OperatorCommand::DaemonRestart { binary } = &data.command
        {
            drop(state);
            let request_id = data.request_id.clone();
            let reply = match preflight(binary.as_deref()) {
                Ok(preflight) => {
                    let result = CommandResult::Restarting {
                        data: RestartingData { preflight },
                    };
                    write_line(&stream, &command_result(request_id, result))?;
                    drop(stream);
                    restart(shared);
                    return Ok(());
                }
                Err(message) => error(Some(request_id), error_code::PREFLIGHT_FAILED, message),
            };
            write_line(&stream, &reply)?;
            continue;
        }
        if matches!(request, ClientRequest::SubscribeTerminal { .. }) {
            conn.full_scope.replace_view();
        }
        let replies = handle(&mut state, request, conn);
        drop(state);
        for reply in replies {
            write_line(&stream, &reply)?;
        }
    }
    Ok(())
}

/// The fake's preflight: `<binary> --version` must print `agend …` (the
/// real one runs `<binary> daemon preflight` on a copy of the database).
fn preflight(binary: Option<&str>) -> Result<Vec<String>, String> {
    let keeps = format!(
        "the daemon keeps running agend {} (fake daemon)",
        env!("CARGO_PKG_VERSION")
    );
    let Some(binary) = binary else {
        return Ok(vec![
            format!("agend {}", env!("CARGO_PKG_VERSION")),
            "db copy: none (fake daemon)".into(),
            "holder: none (fake daemon)".into(),
        ]);
    };
    let out = std::process::Command::new(binary)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {binary}: {e}; {keeps}"))?;
    let version = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if !out.status.success() {
        let status = out
            .status
            .code()
            .map_or("a signal".to_owned(), |c| format!("status {c}"));
        return Err(format!(
            "{binary} daemon preflight exited with {status}; {keeps}"
        ));
    }
    if !version.starts_with("agend ") {
        return Err(format!(
            "{binary} daemon preflight exited 0 without reporting its steps (is it agend?); {keeps}"
        ));
    }
    Ok(vec![
        version,
        "db copy: none (fake daemon)".into(),
        "holder: none (fake daemon)".into(),
    ])
}

/// A new boot on the same socket: every connection closes (clients read
/// EOF), the event log starts again after a new, larger base (`boot_id`).
fn restart(shared: &Shared) {
    {
        let mut state = lock(&shared.state);
        let start = (now_unix_ms() * 1000).max(state.start + 1000);
        state.start = start;
        state.latest = start;
        state.events.clear();
        for scope in state.full_scopes.values() {
            scope.close();
        }
        state.full_scopes.clear();
        state.subscribers.clear();
        state.terminals.clear();
    }
    for (_, connection) in lock(&shared.connections).drain(..) {
        let _ = connection.shutdown(Shutdown::Both);
    }
}

fn command_result(request_id: String, result: CommandResult) -> ClientResponse {
    ClientResponse::CommandResult {
        data: ClientCommandResultData { request_id, result },
    }
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
        ClientRequest::Claude { data } => vec![error(
            Some(data.request_id),
            error_code::NOT_SUPPORTED,
            "fake daemon has no Claude bridge; use the native server".into(),
        )],
        ClientRequest::Hello { .. } => vec![error(
            None,
            error_code::INVALID_REQUEST,
            "hello was already negotiated".into(),
        )],
        ClientRequest::Command { data } => {
            let request_id = data.request_id;
            let Some(caller) = conn.caller.as_deref() else {
                let message = format!(
                    "{} is an agent command; it runs inside an agent, where AGEND_INSTANCE is set",
                    command_name(&data.command)
                );
                return vec![error(Some(request_id), error_code::FORBIDDEN, message)];
            };
            match command(state, caller, data.command) {
                Ok(result) => vec![command_result(request_id, result)],
                Err((code, message)) => vec![error(Some(request_id), code, message)],
            }
        }
        ClientRequest::Operator { data } => {
            let request_id = data.request_id;
            if conn.caller.is_some() {
                let message = operator_only(&data.command).to_owned();
                return vec![error(Some(request_id), error_code::FORBIDDEN, message)];
            }
            match operator(state, data.command) {
                Ok(result) => vec![command_result(request_id, result)],
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
                state.subscribers.push(Subscriber {
                    connection: conn.id,
                    queue,
                    lagged,
                });
            }
            Vec::new()
        }
        ClientRequest::SubscribeTerminal { data } => {
            // A new subscription replaces this connection's old one, also
            // when it fails (dropping the queue ends its forwarder).
            state.terminals.retain(|t| t.connection != conn.id);
            let id = data.instance_id;
            if !state.instances.iter().any(|i| i.instance_id == id) {
                let message = format!("no instance {id}");
                return vec![error(None, error_code::NO_TERMINAL, message)];
            }
            let screen = state
                .screens
                .get(&id)
                .cloned()
                .unwrap_or_else(|| default_screen(&id));
            let (queue, receiver) = sync_channel(TERMINAL_CHUNKS);
            let lagged = Arc::new(AtomicBool::new(false));
            let (writer, flag, instance) =
                (Arc::clone(&conn.writer), Arc::clone(&lagged), id.clone());
            let started = std::thread::Builder::new()
                .name("fake-daemon-terminal".into())
                .spawn(move || forward_terminal(writer, instance, receiver, flag));
            if started.is_ok() {
                state.terminals.push(TerminalSubscriber {
                    connection: conn.id,
                    instance: id.clone(),
                    queue,
                    lagged,
                });
            }
            vec![ClientResponse::TerminalSnapshot {
                data: TerminalSnapshotData {
                    instance_id: id,
                    screen,
                },
            }]
        }
        ClientRequest::TerminalInput { data } => {
            use base64::Engine;
            if conn.caller.is_some() {
                return vec![error(
                    None,
                    error_code::FORBIDDEN,
                    TYPE_OPERATOR_ONLY.into(),
                )];
            }
            let id = data.instance_id;
            // A `failed` instance has no live terminal (as on the daemon).
            let Some(instance) = state
                .instances
                .iter()
                .find(|i| i.instance_id == id && i.state != AgentState::Failed)
            else {
                let message = format!("{id} has no live terminal; nothing was written");
                return vec![error(None, error_code::NO_TERMINAL, message)];
            };
            if instance.backend == "codex" {
                return vec![error(None, error_code::NOT_SUPPORTED, CODEX_INPUT.into())];
            }
            // What the daemon would send its holder, newline included.
            let line = serde_json::to_vec(&HolderRequest::OperatorTerminalInput {
                data: OperatorTerminalInputData {
                    bytes_base64: data.bytes_base64.clone(),
                },
            })
            .map_or(usize::MAX, |l| l.len() + 1);
            if line > MAX_REQUEST_LINE {
                let message = operator_input_too_long(line);
                return vec![error(None, error_code::INVALID_REQUEST, message)];
            }
            // Like the daemon, which does not decode: an input that is not
            // base64 is refused by the holder, which the daemon only logs.
            // Nothing is written and nothing is answered.
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&data.bytes_base64)
            {
                state.inputs.push((id, bytes));
            }
            Vec::new()
        }
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
            let event = DaemonEvent::AttentionResolved {
                data: AttentionResolvedData {
                    attention_id: data.attention_id,
                    action: data.action,
                },
            };
            if state.hold_resolved {
                state.held_resolved.push(event);
            } else {
                emit(state, event);
            }
            vec![accepted(data.request_id)]
        }
        ClientRequest::SubscribeTerminalFrames { data } => vec![terminal_version_error(
            conn.caller.as_deref(),
            data.request_id,
            false,
        )],
        ClientRequest::SetTerminalViewport { data } => vec![terminal_version_error(
            conn.caller.as_deref(),
            data.request_id,
            false,
        )],
        ClientRequest::TerminalControl { data } => vec![terminal_version_error(
            conn.caller.as_deref(),
            data.request_id,
            true,
        )],
        ClientRequest::Unknown => vec![error(
            None,
            error_code::UNKNOWN_REQUEST,
            "unknown request type".into(),
        )],
    }
}

type CommandError = (&'static str, String);

/// `agend review approve` for `review_approve`, from the command's wire tag.
fn command_name(command: &AgentCommand) -> String {
    let tag = serde_json::to_value(command)
        .ok()
        .and_then(|v| v.get("command")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into());
    format!("agend {}", tag.replace('_', " "))
}

/// What an agent gets for an operator request (the real daemon's words).
fn operator_only(command: &OperatorCommand) -> &'static str {
    match command {
        OperatorCommand::InstanceAdd { .. } => {
            "only the operator can add instances; ask the operator"
        }
        OperatorCommand::InstanceRemove { .. } => {
            "only the operator can remove instances; ask the operator"
        }
        OperatorCommand::DaemonRestart { .. } => {
            "only the operator can restart the daemon; ask the operator"
        }
        OperatorCommand::TaskCancel { .. } => {
            "only the operator can cancel tasks; ask the operator"
        }
        _ => "only the operator can send operator requests; ask the operator",
    }
}

/// Messages `inbox` shows without a cursor (as the real daemon).
const INBOX_LAST: usize = 20;

fn not_an_instance(caller: &str) -> CommandError {
    (
        error_code::UNKNOWN_INSTANCE,
        format!(
            "no instance {caller}: AGEND_INSTANCE names no instance of this daemon (the operator sees them with agend instance list)"
        ),
    )
}

fn fresh_id(state: &mut State) -> String {
    state.next_id += 1;
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&state.start.to_be_bytes());
    bytes[8..].copy_from_slice(&state.next_id.to_be_bytes());
    uuid_v4(bytes)
}

fn operator(state: &mut State, command: OperatorCommand) -> Result<CommandResult, CommandError> {
    let invalid = |m: String| (error_code::INVALID_REQUEST, m);
    match command {
        OperatorCommand::InstanceAdd {
            instance_id: id,
            backend,
            working_directory,
            ..
        } => {
            let valid = !id.is_empty()
                && id.len() <= 24
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !valid {
                return Err(invalid(format!(
                    "invalid name {id:?}: use 1-24 characters from a-z 0-9 - (e.g. dev-1)"
                )));
            }
            if !["claude", "codex", "opencode"].contains(&backend.as_str()) {
                return Err(invalid(format!(
                    "unknown backend {backend:?}: use claude, codex or opencode"
                )));
            }
            if state.instances.iter().any(|i| i.instance_id == id) {
                return Err((
                    error_code::INSTANCE_EXISTS,
                    format!("name {id} is already used"),
                ));
            }
            let working_directory = match working_directory {
                Some(dir) if Path::new(&dir).is_dir() => dir,
                Some(dir) => return Err(invalid(format!("no directory {dir} (it must exist)"))),
                None => state.home.join("workspace").join(&id).display().to_string(),
            };
            let session_id = (backend == "claude").then(|| fresh_id(state));
            let view = InstanceView {
                instance_id: id.clone(),
                team_id: agend_core::model::DEFAULT_TEAM.into(),
                backend,
                state: AgentState::Starting,
                working_directory: Some(working_directory.clone()),
            };
            state.instances.push(view.clone());
            emit(
                state,
                DaemonEvent::InstanceChanged {
                    data: InstanceChangedData {
                        instance_id: id.clone(),
                        summary: "starting".into(),
                        instance: Some(view),
                    },
                },
            );
            Ok(CommandResult::InstanceAdded {
                data: InstanceAddedData {
                    instance_id: id,
                    session_id,
                    working_directory,
                },
            })
        }
        OperatorCommand::InstanceRemove { instance_id: id } => {
            let Some(index) = state.instances.iter().position(|i| i.instance_id == id) else {
                return Err((
                    error_code::UNKNOWN_INSTANCE,
                    format!("no instance {id}; see agend instance list"),
                ));
            };
            state.instances.remove(index);
            if let Some(endpoint) = state.full_terminals.get(&id) {
                endpoint.set_live(false);
            }
            emit(
                state,
                DaemonEvent::InstanceChanged {
                    data: InstanceChangedData {
                        instance_id: id,
                        summary: "removed".into(),
                        instance: None,
                    },
                },
            );
            Ok(CommandResult::Accepted)
        }
        OperatorCommand::TaskCancel { task_id, .. } => Err((
            error_code::INVALID_REQUEST,
            format!("unknown task {task_id}"),
        )),
        // Handled in `serve` (it closes connections).
        _ => Err((
            error_code::UNKNOWN_REQUEST,
            "unknown operator command".into(),
        )),
    }
}

fn status(state: &State, caller: &str) -> Result<CommandResult, CommandError> {
    let current = state.assignments.iter().next();
    let summary = match (&state.status_summary, current) {
        (Some(summary), _) => summary.clone(),
        (None, Some((task, identity))) => format!(
            "{caller}: {task} · {} (attempt {})",
            identity.stage_id, identity.attempt
        ),
        (None, None) => {
            let instance = state
                .instances
                .iter()
                .find(|i| i.instance_id == caller)
                .ok_or_else(|| not_an_instance(caller))?;
            format!(
                "{caller} ({}): no task\nnext: agend inbox | agend send <name> \"<message>\"",
                instance.backend
            )
        }
    };
    Ok(CommandResult::Status {
        data: StatusData {
            task_id: current.map(|(task, _)| task.clone()),
            instance_id: Some(caller.to_owned()),
            summary,
            identity: current.map(|(_, identity)| identity.clone()),
        },
    })
}

fn send_message(
    state: &mut State,
    caller: &str,
    to: String,
    body: String,
    level: Option<MessageLevel>,
    message_id: Option<String>,
) -> Result<CommandResult, CommandError> {
    if body.len() > MAX_MESSAGE_BYTES {
        return Err((error_code::INVALID_REQUEST, message_too_long(body.len())));
    }
    let level = level.unwrap_or(MessageLevel::Queue);
    if level == MessageLevel::Unknown {
        return Err((
            error_code::INVALID_REQUEST,
            "unknown level; use queue, steer or interrupt".into(),
        ));
    }
    let id = match message_id {
        Some(id) if is_uuid_v4(&id) => id,
        Some(id) => {
            return Err((
                error_code::INVALID_REQUEST,
                format!("message id {id} is not a UUID v4"),
            ));
        }
        None => fresh_id(state),
    };
    let known = |name: &str| state.instances.iter().any(|i| i.instance_id == name);
    if !known(caller) {
        return Err(not_an_instance(caller));
    }
    if !known(&to) {
        return Err((
            error_code::UNKNOWN_INSTANCE,
            format!("no instance {to}; the names are in agend status"),
        ));
    }
    if let Some(found) = state.messages.iter().find(|m| m.id == id) {
        let same =
            found.from == caller && found.to == to && found.body == body && found.level == level;
        return if same {
            Ok(CommandResult::Accepted)
        } else {
            Err((
                error_code::INVALID_REQUEST,
                format!("message id {id} is already used by another message"),
            ))
        };
    }
    state.messages.push(FakeMessage {
        id,
        from: caller.to_owned(),
        to,
        body,
        level,
        created_at_unix_ms: now_unix_ms(),
    });
    Ok(CommandResult::Accepted)
}

fn inbox(
    state: &State,
    caller: &str,
    after: Option<String>,
) -> Result<CommandResult, CommandError> {
    let own: Vec<&FakeMessage> = state.messages.iter().filter(|m| m.to == caller).collect();
    let shown: Vec<&FakeMessage> = match after {
        None => own[own.len().saturating_sub(INBOX_LAST)..].to_vec(),
        Some(after) => {
            let Some(at) = own.iter().position(|m| m.id == after) else {
                return Err((
                    error_code::UNKNOWN_MESSAGE,
                    format!(
                        "you have no message {after} (unknown, older than 30 days, or not yours); run agend inbox without --after"
                    ),
                ));
            };
            own[at + 1..].to_vec()
        }
    };
    Ok(CommandResult::Messages {
        data: MessagesData {
            messages: shown
                .into_iter()
                .map(|m| InboxMessage {
                    message_id: m.id.clone(),
                    from: m.from.clone(),
                    body: m.body.clone(),
                    created_at_unix_ms: m.created_at_unix_ms,
                })
                .collect(),
        },
    })
}

fn command(
    state: &mut State,
    caller: &str,
    command: AgentCommand,
) -> Result<CommandResult, CommandError> {
    let result = match command {
        AgentCommand::Status => status(state, caller)?,
        AgentCommand::Inbox { after_message_id } => inbox(state, caller, after_message_id)?,
        AgentCommand::Send {
            to,
            message,
            level,
            message_id,
        } => send_message(state, caller, to, message, level, message_id)?,
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
        AgentCommand::AskFollowUp { .. }
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

    /// A fixture writer for the same client connection. Keep its complete JSON
    /// lines ordered, and drive the reader concurrently during large writes.
    pub fn writer_clone(&self) -> io::Result<UnixStream> {
        self.writer.try_clone()
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

fn terminal_version_error(
    caller: Option<&str>,
    request_id: String,
    control: bool,
) -> ClientResponse {
    let (code, message) = if control && caller.is_some() {
        (
            error_code::FORBIDDEN,
            "only the operator can control a terminal view",
        )
    } else {
        (
            error_code::NOT_SUPPORTED,
            "full terminal requires client protocol 1.4; run: agend daemon restart",
        )
    };
    error(Some(request_id), code, message.into())
}
