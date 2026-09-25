//! `fake-opencode-serve`: the subset of `opencode serve` 1.18.31 that
//! docs/backends/opencode.md and research/spike-opencode.md record. HTTP on
//! `127.0.0.1:<port>` plus a server-sent event stream.
//!
//! | Covered | Behaviour |
//! |---|---|
//! | `GET /global/health` | `{"healthy": true, ...}` |
//! | `POST /session`, `GET /session/:id` | session `{id}`; unknown id → 404 |
//! | `POST /session/:id/message {parts}` | runs a turn and answers `{info, parts}` when it ends |
//! | `POST /session/:id/prompt_async {parts}` | 204; while busy it queues (server-side FIFO, spike O3) |
//! | `POST /session/:id/abort` | 200 `true`; the running assistant message gets `MessageAbortedError`; idle at once |
//! | `GET /session/:id/message` | every `{info, parts}` so far (REST backfill, O1) |
//! | `GET /session/status` | `{}` when idle, `{"<id>": {"type": "busy"}}` when busy |
//! | `GET /permission` | `[]` |
//! | `GET /event` | `server.connected`, then `session.status`, `message.updated`, `message.part.updated`, `session.idle` as `{type, properties}`; no replay for late subscribers |
//!
//! Not covered: permission asks, token deltas (`message.part.delta`),
//! models and providers, `/config`, `/doc`, persistence across restarts.
//!
//! Must NOT: call a model.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::http::{self, Request};
use super::{Args, reply_to};
use crate::fakes::lock;

pub fn main(args: impl IntoIterator<Item = String>) -> ExitCode {
    let parsed = Args::parse(
        args,
        &["--hostname", "--port", "--turn-ms"],
        &[],
        &["serve"],
    );
    let args = match parsed {
        Ok(args) => args,
        Err(e) => return usage(&e),
    };
    if args.get("--hostname").is_some_and(|h| h != "127.0.0.1") {
        return usage("only --hostname 127.0.0.1 is supported");
    }
    let port = match args.get("--port").map_or(Ok(0), str::parse::<u16>) {
        Ok(port) => port,
        Err(e) => return usage(&format!("--port: {e}")),
    };
    let turn_ms = match args.turn_ms() {
        Ok(ms) => ms,
        Err(e) => return usage(&e),
    };
    let server = match Server::start(port, Duration::from_millis(turn_ms)) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("fake-opencode-serve: cannot listen on port {port}: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "fake-opencode-serve listening on http://127.0.0.1:{}",
        server.port()
    );
    super::wait_for_stdin_eof();
    ExitCode::SUCCESS
}

fn usage(error: &str) -> ExitCode {
    eprintln!(
        "fake-opencode-serve: {error}\nusage: fake-opencode-serve [serve] [--hostname 127.0.0.1] [--port <port>] [--turn-ms <ms>]"
    );
    ExitCode::from(2)
}

pub struct Server {
    port: u16,
}

impl Server {
    /// Listens on `127.0.0.1:<port>` (0 picks a free port).
    pub fn start(port: u16, turn: Duration) -> io::Result<Server> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            turn,
        });
        let ticker = Arc::clone(&shared);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(5));
                lock(&ticker.state).finish_due_turns(ticker.turn);
            }
        });
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || {
                    let _ = serve(stream, &shared);
                });
            }
        });
        Ok(Server { port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

struct Shared {
    state: Mutex<State>,
    turn: Duration,
}

#[derive(Default)]
struct State {
    sessions: BTreeMap<String, Session>,
    subscribers: Vec<TcpStream>,
    next_id: u64,
}

#[derive(Default)]
struct Session {
    messages: Vec<Value>,
    active: Option<Active>,
    queue: VecDeque<String>,
}

struct Active {
    assistant_index: usize,
    prompt: String,
    deadline: Instant,
}

impl State {
    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}_fake{:04}", self.next_id)
    }

    fn broadcast(&mut self, kind: &str, properties: Value) {
        let event = json!({"type": kind, "properties": properties}).to_string();
        self.subscribers
            .retain(|s| http::send_event(s, &event).is_ok());
    }

    /// Records the user message and starts the turn (or queues it).
    /// Returns the index the assistant message will have.
    fn prompt(&mut self, session_id: &str, text: String, turn: Duration) -> usize {
        let busy = self.sessions[session_id].active.is_some();
        if busy {
            let session = self.sessions.get_mut(session_id).expect("known session");
            session.queue.push_back(text);
            let queued_before = session.queue.len() - 1;
            return session.messages.len() + 2 * queued_before + 1;
        }
        let user = self.message(session_id, "user", &text, false);
        self.broadcast("message.updated", json!({"info": user["info"]}));
        self.broadcast(
            "session.status",
            json!({"sessionID": session_id, "status": {"type": "busy"}}),
        );
        let assistant = self.message(session_id, "assistant", "", false);
        self.broadcast("message.updated", json!({"info": assistant["info"]}));
        let session = self.sessions.get_mut(session_id).expect("known session");
        session.messages.push(user);
        session.messages.push(assistant);
        let assistant_index = session.messages.len() - 1;
        session.active = Some(Active {
            assistant_index,
            prompt: text,
            deadline: Instant::now() + turn,
        });
        assistant_index
    }

    fn message(&mut self, session_id: &str, role: &str, text: &str, done: bool) -> Value {
        let id = self.id("msg");
        let part_id = self.id("prt");
        let mut info = json!({"id": id, "sessionID": session_id, "role": role, "time": {"created": self.next_id}});
        if done {
            info["time"]["completed"] = json!(self.next_id);
        }
        let parts = if text.is_empty() {
            json!([])
        } else {
            json!([{"id": part_id, "sessionID": session_id, "messageID": id, "type": "text", "text": text}])
        };
        json!({"info": info, "parts": parts})
    }

    /// Ends the running turn: completed with a reply, or aborted.
    fn end_turn(&mut self, session_id: &str, aborted: bool, turn: Duration) {
        let Some(active) = self
            .sessions
            .get_mut(session_id)
            .and_then(|s| s.active.take())
        else {
            return;
        };
        self.next_id += 1;
        let now = self.next_id;
        let part_id = self.id("prt");
        let session = self.sessions.get_mut(session_id).expect("known session");
        let message = &mut session.messages[active.assistant_index];
        message["info"]["time"]["completed"] = json!(now);
        let mut part = Value::Null;
        if aborted {
            message["info"]["error"] =
                json!({"name": "MessageAbortedError", "data": {"message": "Aborted"}});
        } else {
            let id = message["info"]["id"].clone();
            part = json!({"id": part_id, "sessionID": session_id, "messageID": id, "type": "text", "text": reply_to(&active.prompt)});
            message["parts"] = json!([part.clone()]);
        }
        let info = message["info"].clone();
        let next = session.queue.pop_front();
        if !part.is_null() {
            self.broadcast("message.part.updated", json!({"part": part}));
        }
        self.broadcast("message.updated", json!({"info": info}));
        match next {
            Some(text) => {
                self.prompt(session_id, text, turn);
            }
            None => {
                self.broadcast(
                    "session.status",
                    json!({"sessionID": session_id, "status": {"type": "idle"}}),
                );
                self.broadcast("session.idle", json!({"sessionID": session_id}));
            }
        }
    }

    fn finish_due_turns(&mut self, turn: Duration) {
        let now = Instant::now();
        let due: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.active.as_ref().is_some_and(|a| a.deadline <= now))
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.end_turn(&id, false, turn);
        }
    }
}

fn text_of(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let parts = value["parts"].as_array()?;
    Some(
        parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn serve(stream: TcpStream, shared: &Shared) -> io::Result<()> {
    let request = http::read_request(&stream)?;
    if request.method == "GET" && request.path == "/event" {
        http::start_event_stream(&stream)?;
        let connected = json!({"type": "server.connected", "properties": {}}).to_string();
        http::send_event(&stream, &connected)?;
        lock(&shared.state).subscribers.push(stream);
        return Ok(());
    }
    let (status, body) = route(&request, shared);
    http::respond(&stream, status, body.as_deref())
}

fn route(request: &Request, shared: &Shared) -> (u16, Option<String>) {
    let segments: Vec<&str> = request.path.trim_matches('/').split('/').collect();
    let mut state = lock(&shared.state);
    match (request.method.as_str(), segments.as_slice()) {
        ("GET", ["global", "health"]) => ok(json!({"healthy": true, "version": "fake-1.18.31"})),
        ("GET", ["permission"]) => ok(json!([])),
        ("POST", ["session"]) => {
            let id = state.id("ses");
            state.sessions.insert(id.clone(), Session::default());
            ok(json!({"id": id}))
        }
        ("GET", ["session", "status"]) => {
            let busy: serde_json::Map<String, Value> = state
                .sessions
                .iter()
                .filter(|(_, s)| s.active.is_some())
                .map(|(id, _)| (id.clone(), json!({"type": "busy"})))
                .collect();
            ok(Value::Object(busy))
        }
        (_, ["session", id, ..]) if !state.sessions.contains_key(*id) => {
            (404, Some(json!({"name": "NotFoundError", "data": {"message": format!("session not found: {id}")}}).to_string()))
        }
        ("GET", ["session", id]) => ok(json!({"id": id})),
        ("GET", ["session", id, "message"]) => ok(Value::Array(state.sessions[*id].messages.clone())),
        ("POST", ["session", id, "prompt_async"]) => match text_of(&request.body) {
            Some(text) => {
                state.prompt(id, text, shared.turn);
                (204, None)
            }
            None => bad_body(),
        },
        ("POST", ["session", id, "abort"]) => {
            let id = (*id).to_owned();
            state.end_turn(&id, true, shared.turn);
            ok(json!(true))
        }
        ("POST", ["session", id, "message"]) => {
            let Some(text) = text_of(&request.body) else {
                return bad_body();
            };
            let id = (*id).to_owned();
            let index = state.prompt(&id, text, shared.turn);
            drop(state);
            wait_for_completion(shared, &id, index)
        }
        _ => (404, Some(json!({"name": "NotFoundError", "data": {"message": "no such route"}}).to_string())),
    }
}

fn ok(value: Value) -> (u16, Option<String>) {
    (200, Some(value.to_string()))
}

fn bad_body() -> (u16, Option<String>) {
    (
        400,
        Some(
            json!({"name": "BadRequest", "data": {"message": "body needs parts[].text"}})
                .to_string(),
        ),
    )
}

fn wait_for_completion(shared: &Shared, session_id: &str, index: usize) -> (u16, Option<String>) {
    loop {
        {
            let state = lock(&shared.state);
            if let Some(message) = state.sessions[session_id].messages.get(index)
                && !message["info"]["time"]["completed"].is_null()
            {
                return ok(message.clone());
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
