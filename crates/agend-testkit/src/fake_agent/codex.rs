//! `fake-codex-app-server`: the subset of `codex app-server` 0.156.1 that
//! docs/backends/codex.md and research/spike-codex.md record. JSON-RPC
//! messages without a `jsonrpc` field (`{id, method, params}`,
//! `{method, params}`, `{id, result}`, `{id, error}`) in WebSocket text
//! frames over a unix socket (`--listen unix://<path>`).
//!
//! | Covered | Behaviour |
//! |---|---|
//! | `initialize` | `{userAgent}` |
//! | `thread/start`, `thread/resume {threadId}` | `{thread: {id}}`; the connection then receives the thread's `turn/*` and `item/*` notifications (only after start or resume, spike S2) |
//! | `turn/start {threadId, input}` | `{turn: {id, status: "inProgress"}}`, `turn/started`, `item/completed` (userMessage); after `--turn-ms`: `item/completed` (agentMessage), `turn/completed` status `completed`. While a turn runs, a new `turn/start` joins it (spike S3) |
//! | `turn/steer {threadId, expectedTurnId, input}` | `{turnId}`; the reply covers the steered text; wrong turn id → error -32600 |
//! | `turn/interrupt {threadId, turnId}` | `{}`, then `turn/completed` status `interrupted` |
//! | `thread/queue/add {threadId, clientUserMessageId, input}` | `{}`; runs as its own turn after the current one (auto-dequeue) |
//! | `item/commandExecution/requestApproval` | server→client request when the prompt starts with `run: <command>`; the reply `{decision}` decides whether the command "runs" |
//! | long `--listen` path | like codex: the socket lives at a short path and the requested path is a symlink to it (pitfall 1) |
//!
//! Not covered: `thread/turns/list`, `thread/items/list`, streaming deltas,
//! `thread/status/changed`, token usage, the other approval kinds, sandbox,
//! real persistence across restarts. Field sets are the minimum above; the
//! real schema (`codex app-server generate-json-schema`) has more.
//!
//! Must NOT: call a model or run the commands it is asked to approve.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use super::{Args, reply_to};
use crate::fakes::lock;

/// Longest socket path bound directly; longer ones get a short real path.
pub const MAX_DIRECT_SOCKET_PATH: usize = 100;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const APPROVAL_PREFIX: &str = "run: ";

pub fn main(args: impl IntoIterator<Item = String>) -> ExitCode {
    let args = match Args::parse(args, &["--listen", "--turn-ms"], &[], &["app-server"]) {
        Ok(args) => args,
        Err(e) => return usage(&e),
    };
    let Some(path) = args.get("--listen").and_then(|l| l.strip_prefix("unix://")) else {
        return usage("--listen unix://<path> is required");
    };
    let turn_ms = match args.turn_ms() {
        Ok(ms) => ms,
        Err(e) => return usage(&e),
    };
    let server = match Server::bind(Path::new(path), Duration::from_millis(turn_ms)) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("fake-codex-app-server: cannot listen on {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("fake-codex-app-server: listening on unix://{path}");
    super::wait_for_stdin_eof();
    drop(server);
    ExitCode::SUCCESS
}

fn usage(error: &str) -> ExitCode {
    eprintln!(
        "fake-codex-app-server: {error}\nusage: fake-codex-app-server [app-server] --listen unix://<path> [--turn-ms <ms>]"
    );
    ExitCode::from(2)
}

/// A running server; removes its socket (and symlink) on drop.
pub struct Server {
    requested: PathBuf,
    bound: PathBuf,
}

impl Server {
    pub fn bind(requested: &Path, turn: Duration) -> io::Result<Server> {
        let bound = socket_path_for(requested);
        if bound != requested {
            std::fs::create_dir_all(bound.parent().expect("short path has a parent"))?;
            let _ = std::fs::remove_file(&bound);
            std::os::unix::fs::symlink(&bound, requested)?;
        }
        let listener = UnixListener::bind(&bound)?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            turn,
        });
        let ticker = Arc::clone(&shared);
        std::thread::spawn(move || tick_loop(&ticker));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || {
                    let _ = serve(stream, &shared);
                });
            }
        });
        Ok(Server {
            requested: requested.to_path_buf(),
            bound,
        })
    }

    /// Where the socket really is (what a client must connect to).
    pub fn bound_path(&self) -> &Path {
        &self.bound
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.bound);
        if self.bound != self.requested {
            let _ = std::fs::remove_file(&self.requested);
        }
    }
}

/// `requested` itself if it fits, else `<tmp>/fake-codex-daemon/<hash>.sock`.
pub fn socket_path_for(requested: &Path) -> PathBuf {
    if requested.as_os_str().len() <= MAX_DIRECT_SOCKET_PATH {
        return requested.to_path_buf();
    }
    let mut hasher = DefaultHasher::new();
    requested.hash(&mut hasher);
    std::env::temp_dir()
        .join("fake-codex-daemon")
        .join(format!("{:016x}.sock", hasher.finish()))
}

struct Shared {
    state: Mutex<State>,
    turn: Duration,
}

#[derive(Default)]
struct State {
    connections: BTreeMap<u64, Sender<Value>>,
    threads: BTreeMap<String, ThreadState>,
    pending_approvals: BTreeMap<(u64, i64), (String, String)>,
    next_id: u64,
    next_request_id: i64,
}

#[derive(Default)]
struct ThreadState {
    subscribers: BTreeSet<u64>,
    active: Option<Turn>,
    queue: VecDeque<String>,
}

struct Turn {
    id: String,
    texts: Vec<String>,
    deadline: Instant,
    owner: u64,
    approval: Approval,
}

enum Approval {
    NotNeeded,
    Needed(String),
    Asked,
    Answered(String),
}

impl State {
    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}-{}", self.next_id)
    }

    fn notify(&self, thread_id: &str, method: &str, params: Value) {
        let message = json!({"method": method, "params": params});
        if let Some(thread) = self.threads.get(thread_id) {
            for id in &thread.subscribers {
                if let Some(tx) = self.connections.get(id) {
                    let _ = tx.send(message.clone());
                }
            }
        }
    }

    fn start_turn(&mut self, thread_id: &str, text: String, owner: u64, turn: Duration) -> String {
        let turn_id = self.id("turn");
        let item_id = self.id("item");
        let approval = match text.strip_prefix(APPROVAL_PREFIX) {
            Some(command) => Approval::Needed(command.to_owned()),
            None => Approval::NotNeeded,
        };
        let thread = self.threads.get_mut(thread_id).expect("known thread");
        thread.active = Some(Turn {
            id: turn_id.clone(),
            texts: vec![text.clone()],
            deadline: Instant::now() + turn,
            owner,
            approval,
        });
        let started =
            json!({"threadId": thread_id, "turn": {"id": turn_id, "status": "inProgress"}});
        self.notify(thread_id, "turn/started", started);
        let item = json!({"type": "userMessage", "id": item_id, "content": [{"type": "text", "text": text}]});
        self.notify(
            thread_id,
            "item/completed",
            json!({"threadId": thread_id, "turnId": turn_id, "item": item}),
        );
        turn_id
    }

    fn finish_turn(&mut self, thread_id: &str, status: &str, turn: Duration) {
        let thread = self.threads.get_mut(thread_id).expect("known thread");
        let Some(active) = thread.active.take() else {
            return;
        };
        let queued = thread.queue.pop_front();
        if status == "completed" {
            let text = match &active.approval {
                Approval::Answered(decision) if decision.starts_with("accept") => {
                    format!(
                        "ran `{}`",
                        active.texts[0].trim_start_matches(APPROVAL_PREFIX)
                    )
                }
                Approval::Answered(decision) => format!("command not run: {decision}"),
                _ => reply_to(&active.texts.join(" / ")),
            };
            let item_id = self.id("item");
            let item = json!({"type": "agentMessage", "id": item_id, "text": text});
            self.notify(
                thread_id,
                "item/completed",
                json!({"threadId": thread_id, "turnId": active.id, "item": item}),
            );
        }
        let completed = json!({"threadId": thread_id, "turn": {"id": active.id, "status": status}});
        self.notify(thread_id, "turn/completed", completed);
        if let Some(text) = queued {
            self.start_turn(thread_id, text, active.owner, turn);
        }
    }
}

fn tick_loop(shared: &Shared) {
    loop {
        std::thread::sleep(Duration::from_millis(5));
        let mut state = lock(&shared.state);
        let due: Vec<String> = state
            .threads
            .iter()
            .filter(|(_, t)| {
                t.active
                    .as_ref()
                    .is_some_and(|a| a.deadline <= Instant::now())
            })
            .map(|(id, _)| id.clone())
            .collect();
        for thread_id in due {
            let turn = state
                .threads
                .get_mut(&thread_id)
                .and_then(|t| t.active.as_mut());
            let Some(turn) = turn else { continue };
            match &turn.approval {
                Approval::Needed(command) => {
                    let (command, owner, turn_id) = (command.clone(), turn.owner, turn.id.clone());
                    turn.approval = Approval::Asked;
                    state.next_request_id += 1;
                    let request_id = state.next_request_id - 1;
                    let item_id = state.id("exec");
                    let request = json!({
                        "id": request_id,
                        "method": "item/commandExecution/requestApproval",
                        "params": {
                            "kind": "command",
                            "threadId": thread_id,
                            "turnId": turn_id,
                            "itemId": item_id,
                            "reason": "fake-codex-app-server asks before every `run:` command",
                            "command": command,
                        }
                    });
                    state
                        .pending_approvals
                        .insert((owner, request_id), (thread_id.clone(), turn_id));
                    if let Some(tx) = state.connections.get(&owner) {
                        let _ = tx.send(request);
                    }
                }
                Approval::Asked => {}
                Approval::NotNeeded | Approval::Answered(_) => {
                    state.finish_turn(&thread_id, "completed", shared.turn);
                }
            }
        }
    }
}

fn serve(stream: UnixStream, shared: &Shared) -> tungstenite::Result<()> {
    let mut socket = tungstenite::accept(stream).map_err(|e| match e {
        tungstenite::HandshakeError::Failure(e) => e,
        tungstenite::HandshakeError::Interrupted(_) => tungstenite::Error::ConnectionClosed,
    })?;
    socket
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(5)))?;
    let (tx, rx) = mpsc::channel();
    let connection = {
        let mut state = lock(&shared.state);
        state.next_id += 1;
        let id = state.next_id;
        state.connections.insert(id, tx);
        id
    };
    let result = pump(&mut socket, &rx, connection, shared);
    let mut state = lock(&shared.state);
    state.connections.remove(&connection);
    for thread in state.threads.values_mut() {
        thread.subscribers.remove(&connection);
    }
    result
}

fn pump(
    socket: &mut WebSocket<UnixStream>,
    outgoing: &Receiver<Value>,
    connection: u64,
    shared: &Shared,
) -> tungstenite::Result<()> {
    loop {
        while let Ok(message) = outgoing.try_recv() {
            socket.send(Message::text(message.to_string()))?;
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let Ok(message) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if let Some(reply) = handle(&message, connection, shared) {
                    socket.send(Message::text(reply.to_string()))?;
                }
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(e) => return Err(e),
        }
    }
}

fn text_of(params: &Value) -> String {
    params["input"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Handles one client message; returns the response for requests.
fn handle(message: &Value, connection: u64, shared: &Shared) -> Option<Value> {
    let mut state = lock(&shared.state);
    let Some(method) = message["method"].as_str() else {
        // A response to one of our requests (approval decision).
        let id = message["id"].as_i64()?;
        let (thread_id, turn_id) = state.pending_approvals.remove(&(connection, id))?;
        let decision = message["result"]["decision"]
            .as_str()
            .unwrap_or("cancel")
            .to_owned();
        let turn = state.threads.get_mut(&thread_id)?.active.as_mut()?;
        if turn.id == turn_id {
            turn.approval = Approval::Answered(decision);
        }
        return None;
    };
    let id = message.get("id")?.clone();
    let params = &message["params"];
    let thread_id = params["threadId"].as_str().unwrap_or_default().to_owned();
    let result: Result<Value, (i64, String)> = match method {
        "initialize" => Ok(json!({"userAgent": "fake-codex-app-server (codex 0.156.1 subset)"})),
        "thread/start" => {
            let new_id = state.id("thr");
            let mut thread = ThreadState::default();
            thread.subscribers.insert(connection);
            state.threads.insert(new_id.clone(), thread);
            Ok(json!({"thread": {"id": new_id}}))
        }
        "thread/resume" => match state.threads.get_mut(&thread_id) {
            Some(thread) => {
                thread.subscribers.insert(connection);
                Ok(json!({"thread": {"id": thread_id}}))
            }
            None => Err((INVALID_REQUEST, format!("thread not found: {thread_id}"))),
        },
        "turn/start" => {
            let text = text_of(params);
            match state.threads.get_mut(&thread_id).map(|t| t.active.as_mut()) {
                None => Err((INVALID_REQUEST, format!("thread not found: {thread_id}"))),
                Some(Some(active)) => {
                    active.texts.push(text);
                    Ok(json!({"turn": {"id": active.id, "status": "inProgress"}}))
                }
                Some(None) => {
                    let turn_id = state.start_turn(&thread_id, text, connection, shared.turn);
                    Ok(json!({"turn": {"id": turn_id, "status": "inProgress"}}))
                }
            }
        }
        "turn/steer" => {
            let expected = params["expectedTurnId"].as_str().unwrap_or_default();
            let text = text_of(params);
            match state
                .threads
                .get_mut(&thread_id)
                .and_then(|t| t.active.as_mut())
            {
                Some(active) if active.id == expected => {
                    active.texts.push(text);
                    Ok(json!({"turnId": active.id}))
                }
                _ => Err((
                    INVALID_REQUEST,
                    format!("no active turn {expected} on {thread_id}"),
                )),
            }
        }
        "turn/interrupt" => {
            let turn_id = params["turnId"].as_str().unwrap_or_default();
            let matches = state
                .threads
                .get(&thread_id)
                .and_then(|t| t.active.as_ref())
                .is_some_and(|a| a.id == turn_id);
            if matches {
                state.finish_turn(&thread_id, "interrupted", shared.turn);
                Ok(json!({}))
            } else {
                Err((
                    INVALID_REQUEST,
                    format!("no active turn {turn_id} on {thread_id}"),
                ))
            }
        }
        "thread/queue/add" => {
            let text = text_of(params);
            match state.threads.get_mut(&thread_id) {
                None => Err((INVALID_REQUEST, format!("thread not found: {thread_id}"))),
                Some(thread) if thread.active.is_some() => {
                    thread.queue.push_back(text);
                    Ok(json!({}))
                }
                Some(_) => {
                    state.start_turn(&thread_id, text, connection, shared.turn);
                    Ok(json!({}))
                }
            }
        }
        other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(result) => json!({"id": id, "result": result}),
        Err((code, message)) => json!({"id": id, "error": {"code": code, "message": message}}),
    })
}

/// Waits until a server accepts connections on `listen_path` (the socket
/// file appears between `bind` and `listen`, so its existence is not enough).
pub fn wait_until_listening(listen_path: &Path, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let connected = std::fs::canonicalize(listen_path)
            .ok()
            .is_some_and(|real| UnixStream::connect(real).is_ok());
        if connected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "nothing listening on {} after {timeout:?}",
                listen_path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A blocking WebSocket JSON-RPC client for tests and demos. Connects to the
/// resolved socket path (pitfall 1) and buffers notifications and server
/// requests while waiting for a response.
pub struct Probe {
    socket: WebSocket<UnixStream>,
    inbox: VecDeque<Value>,
    next_id: i64,
}

impl Probe {
    pub fn connect(listen_path: &Path) -> Result<Probe, String> {
        let real = std::fs::canonicalize(listen_path).map_err(|e| format!("realpath: {e}"))?;
        let stream = UnixStream::connect(&real).map_err(|e| format!("connect: {e}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        let (socket, _) = tungstenite::client::client("ws://localhost/", stream)
            .map_err(|e| format!("websocket handshake: {e}"))?;
        Ok(Probe {
            socket,
            inbox: VecDeque::new(),
            next_id: 0,
        })
    }

    fn read(&mut self) -> Result<Value, String> {
        loop {
            match self.socket.read().map_err(|e| format!("read: {e}"))? {
                Message::Text(text) => {
                    return serde_json::from_str(text.as_str()).map_err(|e| e.to_string());
                }
                Message::Close(_) => return Err("connection closed".into()),
                _ => {}
            }
        }
    }

    /// Sends a request and waits for its response: `Ok(result)` or
    /// `Err(error object as text)`.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"id": id, "method": method, "params": params}))?;
        loop {
            let message = self.read()?;
            if message["id"] == id && message.get("method").is_none() {
                return match message.get("error") {
                    Some(error) => Err(error.to_string()),
                    None => Ok(message["result"].clone()),
                };
            }
            self.inbox.push_back(message);
        }
    }

    pub fn send(&mut self, message: Value) -> Result<(), String> {
        self.socket
            .send(Message::text(message.to_string()))
            .map_err(|e| format!("send: {e}"))
    }

    /// Next notification or server request.
    pub fn next_message(&mut self) -> Result<Value, String> {
        match self.inbox.pop_front() {
            Some(message) => Ok(message),
            None => self.read(),
        }
    }

    /// Skips messages until one with `method`; returns it.
    pub fn next_method(&mut self, method: &str) -> Result<Value, String> {
        loop {
            let message = self.next_message()?;
            if message["method"] == method {
                return Ok(message);
            }
        }
    }
}
