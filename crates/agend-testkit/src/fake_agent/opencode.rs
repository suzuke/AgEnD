//! `fake-opencode-serve`: the subset of `opencode serve` 1.18.31 that
//! docs/backends/opencode.md, research/spike-opencode.md and the recordings
//! in `transcripts/opencode/` show. HTTP on `127.0.0.1:<port>` plus a
//! server-sent event stream. The conformance test (`tests/conformance.rs`)
//! compares its traffic with the recordings by shape.
//!
//! | Covered | Behaviour |
//! |---|---|
//! | `GET /global/health` | `{"healthy": true, "version"}` |
//! | `POST /session`, `GET /session/:id` | session info (`id`, `slug`, `title`, `directory`, `path`, `projectID`, `time`, `tokens`, `cost`, `version`; after a prompt also `agent`, `model`, `summary`); unknown id → 404 `NotFoundError` |
//! | `POST /session/:id/prompt_async {parts, model?}` | 204. Idle: the turn starts (`message.updated` user, its text part, `session.status` busy, the assistant message, busy again). Busy: the user message and its part are sent at once and the turn runs after the current one (server-side FIFO, spike O3) |
//! | `POST /session/:id/message {parts}` | the same, then answers `{info, parts}` of the assistant message when the turn ends |
//! | turn end (after `--turn-ms`) | parts `step-start`, `text` (empty, then `message.part.delta`, then the full text), `step-finish`; the assistant message completed (`finish: "stop"`, twice), busy, then the next queued turn or idle + `session.idle` |
//! | `run: <command>` on a line of the prompt | at turn end, a `bash` tool part (`pending`, `running`) and `permission.asked {id, permission, patterns, always, metadata, tool}`; `GET /permission` lists it; `POST /session/:id/permissions/:pid {response}` → 200 `true`, `permission.replied`; `reject` → the tool part `error`, `step-finish` with `tool-calls`, idle (no text reply). Other responses finish the tool part `completed` without running anything (not recorded) |
//! | `POST /session/:id/abort` | 200 `true`; `session.error` `MessageAbortedError`, idle, then the assistant message with `error`, idle again |
//! | `GET /session/:id/message` | every `{info, parts}` so far (REST backfill, O1) |
//! | `GET /session/status` | `{}` when idle, `{"<id>": {"type": "busy"}}` when busy |
//! | `GET /event` | `server.connected`, then every event as `{id, type, properties}`; no replay for late subscribers. Also `session.created`, `session.updated` and `session.diff` bookkeeping |
//! | restart | with `AGEND_FAKE_STATE_DIR` set, sessions and messages persist in `$AGEND_FAKE_STATE_DIR/fake-opencode/state.json` (the real server keeps its database in `XDG_DATA_HOME`), so a restarted fake resumes by session id (O4); without it nothing persists |
//!
//! Not covered: reasoning parts (the model's choice), plugin and catalog
//! events, `server.heartbeat`, token counts, the V2 permission API, models
//! and providers, `/config`, `/doc`.
//!
//! Must NOT: call a model or run the command a permission is asked for.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::http::{self, Request};
use super::{Args, reply_to};
use crate::fakes::lock;

/// Where state persists, under `$AGEND_FAKE_STATE_DIR`.
pub const STATE_FILE: &str = "fake-opencode/state.json";
const VERSION: &str = "1.18.31";

pub fn main(args: impl IntoIterator<Item = String>) -> ExitCode {
    let parsed = Args::parse(
        args,
        &["--hostname", "--port", "--turn-ms"],
        &["--pure"],
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
    let state = super::state_dir().map(|d| d.join(STATE_FILE));
    let server = match Server::start(port, Duration::from_millis(turn_ms), state) {
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
        "fake-opencode-serve: {error}\nusage: fake-opencode-serve [serve] [--pure] [--hostname 127.0.0.1] [--port <port>] [--turn-ms <ms>]"
    );
    ExitCode::from(2)
}

pub struct Server {
    port: u16,
}

impl Server {
    /// Listens on `127.0.0.1:<port>` (0 picks a free port). With `state`,
    /// sessions are loaded from and saved to that file.
    pub fn start(port: u16, turn: Duration, state: Option<PathBuf>) -> io::Result<Server> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        let mut initial = State {
            file: state,
            ..State::default()
        };
        initial.load();
        let shared = Arc::new(Shared {
            state: Mutex::new(initial),
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
    next_event: u64,
    /// Pending permission asks: id → (session, ask JSON).
    permissions: BTreeMap<String, (String, Value)>,
    file: Option<PathBuf>,
}

struct Session {
    info: Value,
    messages: Vec<Value>,
    active: Option<Active>,
    /// Queued prompts: (user message index, text).
    queue: VecDeque<(usize, String)>,
}

struct Active {
    user_index: usize,
    assistant_index: usize,
    prompt: String,
    deadline: Instant,
    /// A permission ask this turn waits for.
    asking: Option<String>,
}

impl State {
    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}_fake{:04}", self.next_id)
    }

    /// A fake clock: strictly increasing milliseconds.
    fn now(&mut self) -> u64 {
        self.next_id += 1;
        1_700_000_000_000 + self.next_id
    }

    fn load(&mut self) {
        let Some(file) = &self.file else { return };
        let Ok(text) = std::fs::read_to_string(file) else {
            return;
        };
        let Ok(saved) = serde_json::from_str::<Value>(&text) else {
            return;
        };
        self.next_id = saved["next_id"].as_u64().unwrap_or(0);
        for s in saved["sessions"].as_array().into_iter().flatten() {
            let id = s["info"]["id"].as_str().unwrap_or_default().to_owned();
            let messages = s["messages"].as_array().cloned().unwrap_or_default();
            self.sessions.insert(
                id,
                Session {
                    info: s["info"].clone(),
                    messages,
                    active: None,
                    queue: VecDeque::new(),
                },
            );
        }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let sessions: Vec<Value> = self
            .sessions
            .values()
            .map(|s| json!({"info": s.info, "messages": s.messages}))
            .collect();
        let saved = json!({"next_id": self.next_id, "sessions": sessions});
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, saved.to_string());
    }

    fn broadcast(&mut self, kind: &str, properties: Value) {
        self.next_event += 1;
        let event = json!({"id": format!("evt_fake{:06}", self.next_event), "type": kind, "properties": properties})
            .to_string();
        self.subscribers
            .retain(|s| http::send_event(s, &event).is_ok());
    }

    fn status(&mut self, session_id: &str, status: &str) {
        self.broadcast(
            "session.status",
            json!({"sessionID": session_id, "status": {"type": status}}),
        );
    }

    fn part_event(&mut self, session_id: &str, part: Value) {
        let time = self.now();
        self.broadcast(
            "message.part.updated",
            json!({"part": part, "sessionID": session_id, "time": time}),
        );
    }

    fn message_event(&mut self, session_id: &str, info: Value) {
        self.broadcast(
            "message.updated",
            json!({"info": info, "sessionID": session_id}),
        );
    }

    fn session_event(&mut self, kind: &str, session_id: &str) {
        let info = self.sessions[session_id].info.clone();
        self.broadcast(kind, json!({"info": info, "sessionID": session_id}));
    }

    fn new_session(&mut self, directory: &str) -> Value {
        let id = self.id("ses");
        let now = self.now();
        let info = json!({
            "id": id, "slug": "fake-session", "version": VERSION, "projectID": "global",
            "directory": directory, "path": directory.trim_start_matches('/'),
            "title": format!("New session - {id}"), "cost": 0,
            "time": {"created": now, "updated": now},
            "tokens": {"cache": {"read": 0, "write": 0}, "input": 0, "output": 0, "reasoning": 0},
        });
        self.sessions.insert(
            id.clone(),
            Session {
                info: info.clone(),
                messages: Vec::new(),
                active: None,
                queue: VecDeque::new(),
            },
        );
        self.session_event("session.created", &id);
        self.save();
        info
    }

    /// Adds the user message (sent at once) and starts its turn, or queues it.
    fn prompt(&mut self, session_id: &str, text: String, model: &Value, turn: Duration) {
        let (provider, model_id) = (
            model["providerID"].as_str().unwrap_or("fake").to_owned(),
            model["modelID"].as_str().unwrap_or("fake-model").to_owned(),
        );
        let id = self.id("msg");
        let part_id = self.id("prt");
        let created = self.now();
        let user = json!({
            "info": {"id": id, "sessionID": session_id, "role": "user", "agent": "build",
                     "model": {"providerID": provider, "modelID": model_id},
                     "time": {"created": created}},
            "parts": [{"id": part_id, "sessionID": session_id, "messageID": id, "type": "text", "text": text}],
        });
        let session = self.sessions.get_mut(session_id).expect("known session");
        session.messages.push(user.clone());
        let user_index = session.messages.len() - 1;
        let busy = session.active.is_some();
        {
            let info = &mut session.info;
            info["agent"] = json!("build");
            info["model"] = json!({"id": model_id, "providerID": provider, "variant": "default"});
        }
        self.message_event(session_id, user["info"].clone());
        self.part_event(session_id, user["parts"][0].clone());
        if busy {
            let session = self.sessions.get_mut(session_id).expect("known session");
            session.queue.push_back((user_index, text));
        } else {
            self.session_event("session.updated", session_id);
            self.status(session_id, "busy");
            self.start_turn(session_id, user_index, text, turn);
        }
        self.save();
    }

    /// Creates the assistant message for the user message at `user_index`.
    fn start_turn(&mut self, session_id: &str, user_index: usize, text: String, turn: Duration) {
        let id = self.id("msg");
        let created = self.now();
        let session = &self.sessions[session_id];
        let user = &session.messages[user_index]["info"];
        let directory = session.info["directory"].clone();
        let info = json!({
            "id": id, "sessionID": session_id, "role": "assistant", "agent": "build", "mode": "build",
            "parentID": user["id"], "providerID": user["model"]["providerID"], "modelID": user["model"]["modelID"],
            "path": {"cwd": directory, "root": "/"}, "cost": 0, "time": {"created": created},
            "tokens": {"cache": {"read": 0, "write": 0}, "input": 0, "output": 0, "reasoning": 0},
        });
        let session = self.sessions.get_mut(session_id).expect("known session");
        session
            .messages
            .push(json!({"info": info.clone(), "parts": []}));
        let assistant_index = session.messages.len() - 1;
        session.active = Some(Active {
            user_index,
            assistant_index,
            prompt: text,
            deadline: Instant::now() + turn,
            asking: None,
        });
        session.messages[user_index]["info"]["summary"] = json!({"diffs": []});
        session.info["summary"] = json!({"additions": 0, "deletions": 0, "files": 0});
        let summary = session.messages[user_index]["info"].clone();
        self.message_event(session_id, info);
        self.session_event("session.updated", session_id);
        self.broadcast("session.diff", json!({"diff": [], "sessionID": session_id}));
        self.message_event(session_id, summary);
        self.status(session_id, "busy");
    }

    fn add_part(&mut self, session_id: &str, index: usize, part: Value) {
        let session = self.sessions.get_mut(session_id).expect("known session");
        let parts = session.messages[index]["parts"]
            .as_array_mut()
            .expect("parts array");
        match parts.iter_mut().find(|p| p["id"] == part["id"]) {
            Some(existing) => *existing = part.clone(),
            None => parts.push(part.clone()),
        }
        self.part_event(session_id, part);
    }

    fn part(&mut self, session_id: &str, message_id: &Value, kind: &str) -> Value {
        let id = self.id("prt");
        json!({"id": id, "sessionID": session_id, "messageID": message_id, "type": kind})
    }

    /// The turn's deadline passed: reply, or ask for a `run:` command.
    fn reach_deadline(&mut self, session_id: &str, turn: Duration) {
        let session = &self.sessions[session_id];
        let Some(active) = &session.active else {
            return;
        };
        if active.asking.is_some() {
            return;
        }
        let (index, prompt) = (active.assistant_index, active.prompt.clone());
        let message_id = session.messages[index]["info"]["id"].clone();
        let step = self.part(session_id, &message_id, "step-start");
        self.add_part(session_id, index, step);
        if let Some(command) = run_command(&prompt) {
            self.ask(session_id, index, &message_id, &command);
            return;
        }
        let reply = reply_to(&prompt);
        let mut text = self.part(session_id, &message_id, "text");
        let start = self.now();
        text["text"] = json!("");
        text["time"] = json!({"start": start});
        self.add_part(session_id, index, text.clone());
        self.broadcast(
            "message.part.delta",
            json!({"sessionID": session_id, "messageID": message_id, "partID": text["id"],
                   "field": "text", "delta": reply}),
        );
        let end = self.now();
        text["text"] = json!(reply);
        text["time"] = json!({"start": start, "end": end});
        self.add_part(session_id, index, text);
        self.finish(session_id, "stop", turn);
    }

    fn ask(&mut self, session_id: &str, index: usize, message_id: &Value, command: &str) {
        let call_id = self.id("call");
        let mut tool = self.part(session_id, message_id, "tool");
        tool["tool"] = json!("bash");
        tool["callID"] = json!(call_id);
        tool["state"] = json!({"status": "pending", "input": {}, "raw": ""});
        self.add_part(session_id, index, tool.clone());
        let directory = self.sessions[session_id].info["directory"].clone();
        let start = self.now();
        tool["state"] = json!({"status": "running", "time": {"start": start},
                               "input": {"command": command, "timeout": 120_000, "workdir": directory}});
        self.add_part(session_id, index, tool.clone());
        let id = self.id("per");
        let first_word = command.split_whitespace().next().unwrap_or_default();
        let ask = json!({
            "id": id, "sessionID": session_id, "permission": "bash",
            "patterns": [command], "always": [format!("{first_word} *")],
            "metadata": {"command": command},
            "tool": {"messageID": message_id, "callID": call_id},
        });
        self.permissions
            .insert(id.clone(), (session_id.to_owned(), ask.clone()));
        if let Some(active) = self
            .sessions
            .get_mut(session_id)
            .and_then(|s| s.active.as_mut())
        {
            active.asking = Some(id);
        }
        self.broadcast("permission.asked", ask);
    }

    /// Answers a permission ask; `false` if unknown.
    fn reply_permission(
        &mut self,
        session_id: &str,
        id: &str,
        response: &str,
        turn: Duration,
    ) -> bool {
        match self.permissions.get(id) {
            Some((s, _)) if s == session_id => {}
            _ => return false,
        }
        let (_, ask) = self.permissions.remove(id).expect("checked");
        self.broadcast(
            "permission.replied",
            json!({"sessionID": session_id, "requestID": id, "reply": response}),
        );
        let Some(active) = self
            .sessions
            .get(session_id)
            .and_then(|s| s.active.as_ref())
        else {
            return true;
        };
        let index = active.assistant_index;
        let tool = self.sessions[session_id].messages[index]["parts"]
            .as_array()
            .and_then(|parts| parts.iter().find(|p| p["callID"] == ask["tool"]["callID"]))
            .cloned();
        if let Some(mut tool) = tool {
            let start = tool["state"]["time"]["start"].clone();
            let end = self.now();
            let input = tool["state"]["input"].clone();
            tool["state"] = if response == "reject" {
                json!({"status": "error", "input": input, "time": {"start": start, "end": end},
                       "error": "The user rejected permission to use this specific tool call."})
            } else {
                json!({"status": "completed", "input": input, "time": {"start": start, "end": end},
                       "output": "(fake-opencode-serve does not run commands)"})
            };
            self.add_part(session_id, index, tool);
        }
        self.finish(session_id, "tool-calls", turn);
        true
    }

    /// Ends the running turn normally (`finish`: `stop` or `tool-calls`).
    fn finish(&mut self, session_id: &str, finish: &str, turn: Duration) {
        let Some(index) = self.sessions[session_id]
            .active
            .as_ref()
            .map(|a| a.assistant_index)
        else {
            return;
        };
        let message_id = self.sessions[session_id].messages[index]["info"]["id"].clone();
        let mut step = self.part(session_id, &message_id, "step-finish");
        step["reason"] = json!(finish);
        step["cost"] = json!(0);
        step["tokens"] = json!({"cache": {"read": 0, "write": 0}, "input": 0, "output": 0, "reasoning": 0, "total": 0});
        self.add_part(session_id, index, step);
        let completed = self.now();
        let session = self.sessions.get_mut(session_id).expect("known session");
        let info = &mut session.messages[index]["info"];
        info["time"]["completed"] = json!(completed);
        info["finish"] = json!(finish);
        info["tokens"]["total"] = json!(0);
        let info = info.clone();
        self.message_event(session_id, info.clone());
        self.message_event(session_id, info);
        if finish == "stop" {
            self.status(session_id, "busy");
        }
        self.next_or_idle(session_id, turn);
    }

    /// Starts the next queued prompt, or goes idle.
    fn next_or_idle(&mut self, session_id: &str, turn: Duration) {
        let session = self.sessions.get_mut(session_id).expect("known session");
        let user_index = session.active.take().map(|a| a.user_index);
        match session.queue.pop_front() {
            Some((next, text)) => self.start_turn(session_id, next, text, turn),
            None => {
                // Persist before `session.idle`: a client may stop the server
                // as soon as it sees it.
                self.save();
                self.status(session_id, "idle");
                self.broadcast("session.idle", json!({"sessionID": session_id}));
                self.session_event("session.updated", session_id);
                if let Some(i) = user_index {
                    let info = self.sessions[session_id].messages[i]["info"].clone();
                    self.message_event(session_id, info);
                }
            }
        }
        self.save();
    }

    /// `POST /session/:id/abort`: the assistant message gets
    /// `MessageAbortedError`; idle at once.
    fn abort(&mut self, session_id: &str, turn: Duration) {
        let Some(active) = self
            .sessions
            .get_mut(session_id)
            .and_then(|s| s.active.take())
        else {
            return;
        };
        let error = json!({"name": "MessageAbortedError", "data": {"message": "Aborted"}});
        self.permissions.retain(|_, (s, _)| s != session_id);
        self.broadcast(
            "session.error",
            json!({"sessionID": session_id, "error": error}),
        );
        self.status(session_id, "idle");
        self.broadcast("session.idle", json!({"sessionID": session_id}));
        let completed = self.now();
        let session = self.sessions.get_mut(session_id).expect("known session");
        let info = &mut session.messages[active.assistant_index]["info"];
        info["time"]["completed"] = json!(completed);
        info["error"] = error.clone();
        let info = info.clone();
        self.message_event(session_id, info);
        self.next_or_idle(session_id, turn);
    }

    fn finish_due_turns(&mut self, turn: Duration) {
        let now = Instant::now();
        let due: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, s)| {
                s.active
                    .as_ref()
                    .is_some_and(|a| a.asking.is_none() && a.deadline <= now)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.reach_deadline(&id, turn);
        }
    }
}

/// The command of the first `run: <command>` line in a prompt.
fn run_command(prompt: &str) -> Option<String> {
    prompt
        .lines()
        .find_map(|l| l.trim().strip_prefix("run: "))
        .map(|c| c.trim().to_owned())
}

fn text_of(body: &[u8]) -> Option<(String, Value)> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let parts = value["parts"].as_array()?;
    let text = parts
        .iter()
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Some((text, value["model"].clone()))
}

fn serve(stream: TcpStream, shared: &Shared) -> io::Result<()> {
    let request = http::read_request(&stream)?;
    if request.method == "GET" && request.path == "/event" {
        http::start_event_stream(&stream)?;
        let mut state = lock(&shared.state);
        state.next_event += 1;
        let connected = json!({"id": format!("evt_fake{:06}", state.next_event), "type": "server.connected", "properties": {}})
            .to_string();
        http::send_event(&stream, &connected)?;
        state.subscribers.push(stream);
        return Ok(());
    }
    let (status, body) = route(&request, shared);
    http::respond(&stream, status, body.as_deref())
}

fn route(request: &Request, shared: &Shared) -> (u16, Option<String>) {
    let segments: Vec<&str> = request.path.trim_matches('/').split('/').collect();
    let mut state = lock(&shared.state);
    match (request.method.as_str(), segments.as_slice()) {
        ("GET", ["global", "health"]) => ok(json!({"healthy": true, "version": VERSION})),
        ("GET", ["permission"]) => ok(Value::Array(
            state.permissions.values().map(|(_, ask)| ask.clone()).collect(),
        )),
        ("POST", ["session"]) => {
            let cwd = std::env::current_dir()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
            ok(state.new_session(&cwd))
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
        (_, ["session", id, ..]) if !state.sessions.contains_key(*id) => (
            404,
            Some(json!({"name": "NotFoundError", "data": {"message": format!("session not found: {id}")}}).to_string()),
        ),
        ("GET", ["session", id]) => ok(state.sessions[*id].info.clone()),
        ("GET", ["session", id, "message"]) => ok(Value::Array(state.sessions[*id].messages.clone())),
        ("POST", ["session", id, "prompt_async"]) => match text_of(&request.body) {
            Some((text, model)) => {
                state.prompt(id, text, &model, shared.turn);
                (204, None)
            }
            None => bad_body(),
        },
        ("POST", ["session", id, "abort"]) => {
            let id = (*id).to_owned();
            state.abort(&id, shared.turn);
            ok(json!(true))
        }
        ("POST", ["session", id, "permissions", pid]) => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
            let response = body["response"].as_str().unwrap_or("reject").to_owned();
            let (id, pid) = ((*id).to_owned(), (*pid).to_owned());
            if state.reply_permission(&id, &pid, &response, shared.turn) {
                ok(json!(true))
            } else {
                (404, Some(json!({"name": "NotFoundError", "data": {"message": format!("permission not found: {pid}")}}).to_string()))
            }
        }
        ("POST", ["session", id, "message"]) => {
            let Some((text, model)) = text_of(&request.body) else {
                return bad_body();
            };
            let id = (*id).to_owned();
            let before = state.sessions[&id].messages.len();
            state.prompt(&id, text, &model, shared.turn);
            drop(state);
            wait_for_reply(shared, &id, before)
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

/// Waits for the assistant reply to the user message added at index
/// `user_index` (the first completed assistant message after it).
fn wait_for_reply(shared: &Shared, session_id: &str, user_index: usize) -> (u16, Option<String>) {
    let user_id =
        lock(&shared.state).sessions[session_id].messages[user_index]["info"]["id"].clone();
    loop {
        {
            let state = lock(&shared.state);
            let reply = state.sessions[session_id].messages.iter().find(|m| {
                m["info"]["parentID"] == user_id && !m["info"]["time"]["completed"].is_null()
            });
            if let Some(message) = reply {
                return ok(message.clone());
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
