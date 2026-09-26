//! `fake-codex-app-server`: the subset of `codex app-server` 0.156.1 that
//! docs/backends/codex.md, research/spike-codex.md and the recordings in
//! `transcripts/codex/` show. JSON-RPC messages without a `jsonrpc` field
//! (`{id, method, params}`, `{method, params}` with `emittedAtMs`,
//! `{id, result}`, `{id, error}`) in WebSocket text frames over a unix
//! socket (`--listen unix://<path>`). The conformance test
//! (`tests/conformance.rs`) compares its traffic with the recordings by shape.
//!
//! | Covered | Behaviour |
//! |---|---|
//! | `initialize` | `{codexHome, platformFamily, platformOs, userAgent}` |
//! | `thread/start {model?, cwd?, approvalPolicy?, sandbox?, config?}` | the full thread settings and `thread` object; `thread/started`. The connection then receives the thread's notifications (only after start or resume, spike S2) |
//! | `turn/start {threadId, input}` | `{turn}` (`inProgress`), `thread/status/changed` active, `turn/started`. After `--turn-ms` the turn "runs": per input (the prompt, then each steer) `item/started` + `item/completed` userMessage, `item/started` agentMessage, one `item/agentMessage/delta`, `item/completed`, `thread/tokenUsage/updated`; then `thread/status/changed` idle and `turn/completed` (`completed`, the agent messages as `items`). A `turn/start` while a turn runs joins it (spike S3) |
//! | `turn/steer {threadId, expectedTurnId, input}` | `{turnId}`; the text is one more input of the running turn (its own user message and reply); wrong turn id → error -32600 |
//! | `turn/interrupt {threadId, turnId}` | `{}`, `thread/status/changed` idle, `turn/completed` `interrupted` |
//! | `thread/queue/add {threadId, clientUserMessageId, input}` | `{queuedSubmission}`, `thread/queue/changed`; runs as its own turn after the current one (auto-dequeue: `thread/queue/changed`, active, `turn/started`; its user message has `clientId`) |
//! | `run: <command>` as the prompt | the turn asks: `thread/status/changed` `waitingOnApproval`, a `commandExecution` item (`/bin/zsh -lc '<command>'`), and the server→client request `item/commandExecution/requestApproval`; the reply `{decision}` gives `serverRequest/resolved`, the item `declined` (or `completed` for `accept*`, without running anything; not recorded), then the agent message |
//! | `thread/resume {threadId}` | `deprecationNotice` (no `excludeTurns`), `thread/status/changed` idle, the settings with the full `thread` (all turns), `thread/tokenUsage/updated`, `thread/goal/cleared` |
//! | restart | with `AGEND_FAKE_STATE_DIR` set, threads persist in `$AGEND_FAKE_STATE_DIR/fake-codex/threads.json` (the real server keeps rollouts under `CODEX_HOME`), so a restarted fake resumes by id (spike S4); without it nothing persists |
//! | `--listen` path | like codex: the socket always lives at a short path (`<tmp>/fake-codex-<hash>.sock`, codex: `/private/tmp/codex-daemon-<uid>/<sha256>`) and the requested path is a symlink to it (pitfall 1) |
//!
//! Not covered: `thread/turns/list`, `thread/items/list`, reasoning items
//! (the model's choice), the other approval kinds, sandboxing, MCP server,
//! account and rate-limit notifications (see `recorder::shape::IGNORED`).
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

/// Longest path the short socket path can have.
pub const MAX_DIRECT_SOCKET_PATH: usize = 100;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const APPROVAL_PREFIX: &str = "run: ";
const CLI_VERSION: &str = "0.156.1";
/// Where threads persist, under `$AGEND_FAKE_STATE_DIR`.
pub const THREADS_FILE: &str = "fake-codex/threads.json";

pub fn main(args: impl IntoIterator<Item = String>) -> ExitCode {
    // `-c key=value` and `--disable <feature>` are accepted and ignored
    // (the recorder passes the real CLI's run settings).
    let known = ["--listen", "--turn-ms", "-c", "--disable"];
    let args = match Args::parse(args, &known, &[], &["app-server"]) {
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
    let home = super::state_dir();
    let server = match Server::bind(Path::new(path), Duration::from_millis(turn_ms), home) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("fake-codex-app-server: cannot start on {path}: {e}");
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
        "fake-codex-app-server: {error}\nusage: fake-codex-app-server [app-server] --listen unix://<path> [-c key=value]... [--disable <feature>]... [--turn-ms <ms>]"
    );
    ExitCode::from(2)
}

/// A running server; removes its socket (and symlink) on drop.
pub struct Server {
    requested: PathBuf,
    bound: PathBuf,
}

impl Server {
    /// Listens on `requested` (a symlink to a short socket path). With
    /// `state_dir`, threads are loaded from and saved under it.
    pub fn bind(
        requested: &Path,
        turn: Duration,
        state_dir: Option<PathBuf>,
    ) -> io::Result<Server> {
        let bound = socket_path_for(requested);
        let _ = std::fs::remove_file(&bound);
        std::os::unix::fs::symlink(&bound, requested)?;
        let listener = UnixListener::bind(&bound)?;
        let mut state = State {
            home: state_dir,
            ..State::default()
        };
        state
            .load()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
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
        let _ = std::fs::remove_file(&self.requested);
    }
}

/// `<tmp>/fake-codex-<hash>.sock` for any requested path (a file directly in
/// the temp dir, so nothing is left behind once removed). Real codex always
/// binds under `/private/tmp/codex-daemon-<uid>/` too, even for short paths.
pub fn socket_path_for(requested: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    requested.hash(&mut hasher);
    std::env::temp_dir().join(format!("fake-codex-{:016x}.sock", hasher.finish()))
}

struct Shared {
    state: Mutex<State>,
    turn: Duration,
}

#[derive(Default)]
struct State {
    connections: BTreeMap<u64, Sender<Value>>,
    threads: BTreeMap<String, ThreadState>,
    /// (connection, request id) → (thread, turn).
    pending_approvals: BTreeMap<(u64, i64), (String, String)>,
    next_id: u64,
    next_request_id: i64,
    home: Option<PathBuf>,
    originator: String,
}

struct ThreadState {
    /// The thread object (`turns` kept empty; see `turns`).
    thread: Value,
    /// Settings echoed by `thread/start` and `thread/resume`.
    settings: Value,
    turns: Vec<Value>,
    subscribers: BTreeSet<u64>,
    active: Option<Turn>,
    queue: VecDeque<(String, String)>,
}

struct Turn {
    id: String,
    /// The prompt, then each steer (or joining `turn/start`).
    inputs: Vec<String>,
    /// Inputs already answered.
    done: usize,
    client_id: Option<String>,
    started_at: u64,
    deadline: Instant,
    owner: u64,
    /// `Some(item)` while waiting for an approval decision.
    asking: Option<Value>,
    items: Vec<Value>,
    agent_items: Vec<Value>,
}

impl State {
    fn id(&mut self) -> String {
        self.next_id += 1;
        format!("00000000-0000-7000-8000-{:012}", self.next_id)
    }

    /// A fake clock: strictly increasing milliseconds.
    fn now_ms(&mut self) -> u64 {
        self.next_id += 1;
        1_700_000_000_000 + self.next_id
    }

    /// Loads state from `home`/[`THREADS_FILE`]. No file is empty state
    /// (nothing has been saved yet, or nothing persists). A present but
    /// unparsable file is an error: `save` writes atomically (temp file +
    /// rename), so a non-empty file that fails to parse means real
    /// corruption, not a save caught mid-write, and starting empty would
    /// silently answer 404 to every thread a client expects to resume.
    fn load(&mut self) -> Result<(), String> {
        let Some(file) = self.home.as_ref().map(|h| h.join(THREADS_FILE)) else {
            return Ok(());
        };
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("cannot read state file {}: {e}", file.display())),
        };
        if text.trim().is_empty() {
            return Ok(());
        }
        let saved: Value = serde_json::from_str(&text).map_err(|e| {
            format!(
                "corrupt state file {} ({} bytes): {e}",
                file.display(),
                text.len()
            )
        })?;
        self.next_id = saved["next_id"].as_u64().unwrap_or(0);
        for t in saved["threads"].as_array().into_iter().flatten() {
            let id = t["thread"]["id"].as_str().unwrap_or_default().to_owned();
            self.threads.insert(
                id,
                ThreadState {
                    thread: t["thread"].clone(),
                    settings: t["settings"].clone(),
                    turns: t["turns"].as_array().cloned().unwrap_or_default(),
                    subscribers: BTreeSet::new(),
                    active: None,
                    queue: VecDeque::new(),
                },
            );
        }
        Ok(())
    }

    fn save(&self) {
        let Some(file) = self.home.as_ref().map(|h| h.join(THREADS_FILE)) else {
            return;
        };
        let threads: Vec<Value> = self
            .threads
            .values()
            .map(|t| json!({"thread": t.thread, "settings": t.settings, "turns": t.turns}))
            .collect();
        let saved = json!({"next_id": self.next_id, "threads": threads});
        let _ = super::write_state_atomic(&file, &saved.to_string());
    }

    fn notification(&mut self, method: &str, params: Value) -> Value {
        let at = self.now_ms();
        json!({"method": method, "params": params, "emittedAtMs": at})
    }

    /// Sends a notification to every connection subscribed to the thread.
    fn notify(&mut self, thread_id: &str, method: &str, params: Value) {
        let message = self.notification(method, params);
        if let Some(thread) = self.threads.get(thread_id) {
            for id in &thread.subscribers {
                if let Some(tx) = self.connections.get(id) {
                    let _ = tx.send(message.clone());
                }
            }
        }
    }

    fn status(&mut self, thread_id: &str, status: Value) {
        self.notify(
            thread_id,
            "thread/status/changed",
            json!({"threadId": thread_id, "status": status}),
        );
    }

    fn item_event(&mut self, thread_id: &str, turn_id: &str, method: &str, item: &Value) {
        let at = self.now_ms();
        let key = if method == "item/started" {
            "startedAtMs"
        } else {
            "completedAtMs"
        };
        self.notify(
            thread_id,
            method,
            json!({"threadId": thread_id, "turnId": turn_id, "item": item, key: at}),
        );
    }

    fn start_thread(&mut self, params: &Value) -> Value {
        let id = self.id();
        let cwd = params["cwd"].as_str().map_or_else(
            || {
                std::env::current_dir()
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_default()
            },
            str::to_owned,
        );
        let model = params["model"].as_str().unwrap_or("fake-model").to_owned();
        let effort = params["config"]["model_reasoning_effort"]
            .as_str()
            .unwrap_or("medium")
            .to_owned();
        let sandbox = match params["sandbox"].as_str().unwrap_or("workspace-write") {
            "read-only" => "readOnly",
            "danger-full-access" => "dangerFullAccess",
            _ => "workspaceWrite",
        };
        let now = self.now_ms() / 1000;
        let home = self.home.clone().unwrap_or_else(std::env::temp_dir);
        let thread = json!({
            "id": id, "sessionId": id, "cwd": cwd, "model": model, "modelProvider": "openai",
            "reasoningEffort": effort, "cliVersion": CLI_VERSION, "originator": self.originator,
            "path": home.join("sessions").join(format!("rollout-{id}.jsonl")),
            "preview": "", "name": null, "source": "vscode", "status": {"type": "idle"},
            "createdAt": now, "updatedAt": now, "recencyAt": now, "historyMode": "paginated",
            "canAcceptDirectInput": true, "ephemeral": false, "extra": null, "gitInfo": null,
            "agentNickname": null, "agentRole": null, "daybreakEnabled": null,
            "forkedFromId": null, "parentThreadId": null, "projectId": null, "section": null,
            "sectionEnteredAt": null, "threadSource": null, "turns": [],
            "environments": [{"cwd": cwd, "environmentId": "local", "runtimeWorkspaceRoots": [cwd]}],
        });
        let settings = json!({
            "approvalPolicy": params["approvalPolicy"].as_str().unwrap_or("on-request"),
            "approvalsReviewer": "user", "cwd": cwd, "model": model, "modelProvider": "openai",
            "reasoningEffort": effort, "multiAgentMode": "explicitRequestOnly",
            "serviceTier": "default", "disabledPluginIds": [], "instructionSources": [],
            "runtimeWorkspaceRoots": [cwd], "sandbox": {"type": sandbox, "networkAccess": false},
        });
        self.threads.insert(
            id,
            ThreadState {
                thread: thread.clone(),
                settings: settings.clone(),
                turns: Vec::new(),
                subscribers: BTreeSet::new(),
                active: None,
                queue: VecDeque::new(),
            },
        );
        self.save();
        let mut result = settings;
        result["activePermissionProfile"] = Value::Null;
        result["thread"] = thread;
        result
    }

    /// The `thread/resume` result: settings plus the thread with all turns.
    fn resume_result(&self, thread_id: &str) -> Value {
        let t = &self.threads[thread_id];
        let mut thread = t.thread.clone();
        thread["turns"] = Value::Array(t.turns.clone());
        let preview = t
            .turns
            .first()
            .and_then(|turn| turn["items"][0]["content"][0]["text"].as_str());
        if let Some(preview) = preview {
            thread["preview"] = json!(preview);
        }
        let cursor = |kind: &str| {
            json!({"requestedThreadId": thread_id, "scope": {"kind": kind}}).to_string()
        };
        let mut result = t.settings.clone();
        result["activePermissionProfile"] = json!({"id": ":read-only", "extends": null});
        result["collaborationMode"] = json!({"mode": "default", "settings": {
            "developer_instructions": null, "model": t.settings["model"],
            "reasoning_effort": t.settings["reasoningEffort"]}});
        result["initialTurnsPage"] = Value::Null;
        result["itemsBackwardsCursor"] = json!(cursor("itemsByCreatedAtOrdinal"));
        result["turnsBackwardsCursor"] = json!(cursor("turns"));
        result["thread"] = thread;
        result
    }

    fn turn_json(
        turn: &Turn,
        status: &str,
        completed_at: Option<u64>,
        items: Value,
        view: &str,
    ) -> Value {
        json!({
            "id": turn.id, "status": status, "items": items, "itemsView": view, "error": null,
            "startedAt": turn.started_at, "completedAt": completed_at,
            "durationMs": completed_at.map(|c| c.saturating_sub(turn.started_at) * 1000),
        })
    }

    fn start_turn(
        &mut self,
        thread_id: &str,
        text: String,
        client_id: Option<String>,
        owner: u64,
        turn: Duration,
    ) -> Value {
        let id = self.id();
        let started_at = self.now_ms() / 1000;
        let t = Turn {
            id,
            inputs: vec![text],
            done: 0,
            client_id,
            started_at,
            deadline: Instant::now() + turn,
            owner,
            asking: None,
            items: Vec::new(),
            agent_items: Vec::new(),
        };
        let pending = json!({"id": t.id, "status": "inProgress", "items": [], "itemsView": "notLoaded",
                             "error": null, "startedAt": null, "completedAt": null, "durationMs": null});
        let started = Self::turn_json(&t, "inProgress", None, json!([]), "notLoaded");
        self.threads
            .get_mut(thread_id)
            .expect("known thread")
            .active = Some(t);
        self.status(thread_id, json!({"type": "active", "activeFlags": []}));
        self.notify(
            thread_id,
            "turn/started",
            json!({"threadId": thread_id, "turn": started}),
        );
        pending
    }

    fn user_item(&mut self, text: &str, client_id: Option<String>) -> Value {
        let id = self.id();
        json!({"type": "userMessage", "id": id, "clientId": client_id,
               "content": [{"type": "text", "text": text, "text_elements": []}]})
    }

    fn token_usage(&mut self, thread_id: &str, turn_id: &str) {
        let usage = json!({"inputTokens": 0, "cachedInputTokens": 0, "cacheWriteInputTokens": 0,
                           "outputTokens": 0, "reasoningOutputTokens": 0, "totalTokens": 0});
        self.notify(
            thread_id,
            "thread/tokenUsage/updated",
            json!({"threadId": thread_id, "turnId": turn_id,
                   "tokenUsage": {"last": usage, "total": usage, "modelContextWindow": 258_400}}),
        );
    }

    fn agent_message(&mut self, thread_id: &str, turn_id: &str, text: &str) {
        let id = format!("msg_fake{:016}", self.next_id + 1);
        self.next_id += 1;
        let mut item = json!({"type": "agentMessage", "id": id, "text": "", "phase": "final_answer",
                              "delivery": null, "memoryCitation": null, "questions": null});
        self.item_event(thread_id, turn_id, "item/started", &item);
        self.notify(
            thread_id,
            "item/agentMessage/delta",
            json!({"threadId": thread_id, "turnId": turn_id, "itemId": id, "delta": text}),
        );
        item["text"] = json!(text);
        self.item_event(thread_id, turn_id, "item/completed", &item);
        self.token_usage(thread_id, turn_id);
        if let Some(turn) = self
            .threads
            .get_mut(thread_id)
            .and_then(|t| t.active.as_mut())
        {
            turn.items.push(item.clone());
            turn.agent_items.push(item);
        }
    }

    /// Answers the inputs not yet answered; stops at an approval request.
    fn run_turn(&mut self, thread_id: &str, turn: Duration) {
        loop {
            let Some(active) = self.threads.get(thread_id).and_then(|t| t.active.as_ref()) else {
                return;
            };
            if active.asking.is_some() {
                return;
            }
            let (turn_id, done) = (active.id.clone(), active.done);
            let Some(text) = active.inputs.get(done).cloned() else {
                self.finish_turn(thread_id, "completed", turn);
                return;
            };
            let client_id = if done == 0 {
                active.client_id.clone()
            } else {
                None
            };
            let user = self.user_item(&text, client_id);
            self.item_event(thread_id, &turn_id, "item/started", &user);
            self.item_event(thread_id, &turn_id, "item/completed", &user);
            let active = self
                .threads
                .get_mut(thread_id)
                .and_then(|t| t.active.as_mut())
                .expect("active");
            active.items.push(user);
            active.done += 1;
            match text.strip_prefix(APPROVAL_PREFIX) {
                Some(command) if done == 0 => {
                    self.ask(
                        thread_id,
                        &turn_id,
                        command.lines().next().unwrap_or_default().trim(),
                    );
                    return;
                }
                _ => self.agent_message(thread_id, &turn_id, &reply_to(&text)),
            }
        }
    }

    fn ask(&mut self, thread_id: &str, turn_id: &str, command: &str) {
        self.status(
            thread_id,
            json!({"type": "active", "activeFlags": ["waitingOnApproval"]}),
        );
        let id = format!("exec-{}", self.id());
        let cwd = self.threads[thread_id].settings["cwd"].clone();
        let item = json!({
            "type": "commandExecution", "id": id, "status": "inProgress", "source": "agent",
            "command": format!("/bin/zsh -lc '{command}'"),
            "commandActions": [{"command": command, "type": "unknown"}], "cwd": cwd,
            "aggregatedOutput": null, "durationMs": null, "exitCode": null, "pluginId": null,
            "processId": null, "scriptPath": null,
        });
        self.item_event(thread_id, turn_id, "item/started", &item);
        let words: Vec<&str> = command.split_whitespace().collect();
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        let started = self.now_ms();
        let request = json!({
            "id": request_id,
            "method": "item/commandExecution/requestApproval",
            "params": {
                "kind": "command", "threadId": thread_id, "turnId": turn_id, "itemId": id,
                "reason": "fake-codex-app-server asks before every `run:` command",
                "command": item["command"], "commandActions": item["commandActions"], "cwd": cwd,
                "environmentId": "local", "startedAtMs": started,
                "availableDecisions": ["accept", {"acceptWithExecpolicyAmendment": {"execpolicy_amendment": words}}, "cancel"],
                "proposedExecpolicyAmendment": words,
            }
        });
        let Some(active) = self
            .threads
            .get_mut(thread_id)
            .and_then(|t| t.active.as_mut())
        else {
            return;
        };
        active.asking = Some(item);
        let owner = active.owner;
        self.pending_approvals.insert(
            (owner, request_id),
            (thread_id.to_owned(), turn_id.to_owned()),
        );
        if let Some(tx) = self.connections.get(&owner) {
            let _ = tx.send(request);
        }
    }

    fn answer(&mut self, thread_id: &str, request_id: i64, decision: &str, turn: Duration) {
        let Some(active) = self
            .threads
            .get_mut(thread_id)
            .and_then(|t| t.active.as_mut())
        else {
            return;
        };
        let Some(mut item) = active.asking.take() else {
            return;
        };
        let turn_id = active.id.clone();
        self.notify(
            thread_id,
            "serverRequest/resolved",
            json!({"threadId": thread_id, "requestId": request_id}),
        );
        let accepted = decision.starts_with("accept");
        item["status"] = json!(if accepted { "completed" } else { "declined" });
        if accepted {
            item["exitCode"] = json!(0);
            item["aggregatedOutput"] = json!("");
        }
        self.item_event(thread_id, &turn_id, "item/completed", &item);
        self.status(thread_id, json!({"type": "active", "activeFlags": []}));
        self.token_usage(thread_id, &turn_id);
        let command = item["commandActions"][0]["command"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let text = if accepted {
            format!("ran `{command}`")
        } else {
            format!("command not run: {decision}")
        };
        if let Some(active) = self
            .threads
            .get_mut(thread_id)
            .and_then(|t| t.active.as_mut())
        {
            active.items.push(item);
        }
        self.agent_message(thread_id, &turn_id, &text);
        self.run_turn(thread_id, turn);
    }

    fn finish_turn(&mut self, thread_id: &str, status: &str, turn: Duration) {
        let thread = self.threads.get_mut(thread_id).expect("known thread");
        let Some(active) = thread.active.take() else {
            return;
        };
        let completed_at = self.now_ms() / 1000;
        let (summary, view) = if status == "completed" {
            (Value::Array(active.agent_items.clone()), "summary")
        } else {
            (json!([]), "notLoaded")
        };
        let full = Self::turn_json(
            &active,
            status,
            Some(completed_at),
            Value::Array(active.items.clone()),
            "full",
        );
        let thread = self.threads.get_mut(thread_id).expect("known thread");
        thread.turns.push(full);
        if thread.thread["preview"] == "" {
            let first = active
                .items
                .first()
                .map(|i| i["content"][0]["text"].clone());
            thread.thread["preview"] = first.unwrap_or_else(|| json!(""));
        }
        let queued = thread.queue.pop_front();
        // Persist before telling anyone: a client may stop the server as
        // soon as it sees `turn/completed`.
        self.save();
        self.status(thread_id, json!({"type": "idle"}));
        let done = Self::turn_json(&active, status, Some(completed_at), summary, view);
        self.notify(
            thread_id,
            "turn/completed",
            json!({"threadId": thread_id, "turn": done}),
        );
        if let Some((text, client_id)) = queued {
            self.notify(
                thread_id,
                "thread/queue/changed",
                json!({"threadId": thread_id}),
            );
            self.start_turn(thread_id, text, Some(client_id), active.owner, turn);
        }
    }
}

fn tick_loop(shared: &Shared) {
    loop {
        std::thread::sleep(Duration::from_millis(5));
        let mut state = lock(&shared.state);
        let now = Instant::now();
        let due: Vec<String> = state
            .threads
            .iter()
            .filter(|(_, t)| {
                t.active
                    .as_ref()
                    .is_some_and(|a| a.done == 0 && a.asking.is_none() && a.deadline <= now)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for thread_id in due {
            state.run_turn(&thread_id, shared.turn);
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
                // Messages for this connection, in order (the reply among them).
                for reply in handle(&message, connection, shared) {
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

/// Handles one client message; returns what to send back on this
/// connection before any queued notification: the response to a request,
/// plus the notifications `thread/resume` sends around it.
fn handle(message: &Value, connection: u64, shared: &Shared) -> Vec<Value> {
    let mut state = lock(&shared.state);
    let Some(method) = message["method"].as_str() else {
        // A response to one of our requests (approval decision).
        let Some(id) = message["id"].as_i64() else {
            return Vec::new();
        };
        if let Some((thread_id, _)) = state.pending_approvals.remove(&(connection, id)) {
            let decision = message["result"]["decision"]
                .as_str()
                .unwrap_or("cancel")
                .to_owned();
            state.answer(&thread_id, id, &decision, shared.turn);
        }
        return Vec::new();
    };
    let Some(id) = message.get("id").cloned() else {
        return Vec::new();
    };
    let params = &message["params"];
    let thread_id = params["threadId"].as_str().unwrap_or_default().to_owned();
    let known = state.threads.contains_key(&thread_id);
    let mut before = Vec::new();
    let mut after = Vec::new();
    let result: Result<Value, (i64, String)> = match method {
        "initialize" => {
            state.originator = params["clientInfo"]["name"]
                .as_str()
                .unwrap_or("fake")
                .to_owned();
            let home = state.home.clone().unwrap_or_else(std::env::temp_dir);
            Ok(
                json!({"codexHome": home, "platformFamily": std::env::consts::FAMILY,
                      "platformOs": std::env::consts::OS,
                      "userAgent": format!("fake-codex-app-server/{CLI_VERSION}")}),
            )
        }
        "thread/start" => {
            let result = state.start_thread(params);
            let new_id = result["thread"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if let Some(t) = state.threads.get_mut(&new_id) {
                t.subscribers.insert(connection);
            }
            state.notify(
                &new_id,
                "thread/started",
                json!({"thread": result["thread"]}),
            );
            Ok(result)
        }
        "thread/resume" if known => {
            if let Some(t) = state.threads.get_mut(&thread_id) {
                t.subscribers.insert(connection);
            }
            if params["excludeTurns"] != true {
                let summary = "Full-history hydration is deprecated for paginated threads; use `excludeTurns: true`, then page with `thread/turns/list` and `thread/items/list`.";
                before.push(state.notification(
                    "deprecationNotice",
                    json!({"summary": summary, "details": null}),
                ));
            }
            let status = if state.threads[&thread_id].active.is_some() {
                json!({"type": "active", "activeFlags": []})
            } else {
                json!({"type": "idle"})
            };
            before.push(state.notification(
                "thread/status/changed",
                json!({"threadId": thread_id, "status": status}),
            ));
            let last_turn = state.threads[&thread_id]
                .turns
                .last()
                .map(|t| t["id"].clone());
            let usage = json!({"inputTokens": 0, "cachedInputTokens": 0, "cacheWriteInputTokens": 0,
                               "outputTokens": 0, "reasoningOutputTokens": 0, "totalTokens": 0});
            after.push(state.notification("thread/tokenUsage/updated", json!({"threadId": thread_id,
                "turnId": last_turn, "tokenUsage": {"last": usage, "total": usage, "modelContextWindow": 258_400}})));
            after.push(state.notification("thread/goal/cleared", json!({"threadId": thread_id})));
            Ok(state.resume_result(&thread_id))
        }
        "thread/resume" | "turn/start" | "turn/steer" | "turn/interrupt" | "thread/queue/add"
            if !known =>
        {
            Err((INVALID_REQUEST, format!("thread not found: {thread_id}")))
        }
        "turn/start" => {
            let text = text_of(params);
            let active = state
                .threads
                .get_mut(&thread_id)
                .and_then(|t| t.active.as_mut());
            match active {
                Some(active) => {
                    active.inputs.push(text);
                    Ok(
                        json!({"turn": {"id": active.id, "status": "inProgress", "items": [], "itemsView": "notLoaded",
                                       "error": null, "startedAt": null, "completedAt": null, "durationMs": null}}),
                    )
                }
                None => Ok(
                    json!({"turn": state.start_turn(&thread_id, text, None, connection, shared.turn)}),
                ),
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
                    active.inputs.push(text);
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
            let matches = state.threads[&thread_id]
                .active
                .as_ref()
                .is_some_and(|a| a.id == turn_id);
            if matches {
                state.pending_approvals.retain(|_, (t, _)| *t != thread_id);
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
            let client_id = params["clientUserMessageId"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let submission = state.id();
            let result = json!({"queuedSubmission": {"id": submission, "clientUserMessageId": client_id,
                                                     "input": params["input"]}});
            let busy = state.threads[&thread_id].active.is_some();
            if busy {
                if let Some(t) = state.threads.get_mut(&thread_id) {
                    t.queue.push_back((text, client_id));
                }
                state.notify(
                    &thread_id,
                    "thread/queue/changed",
                    json!({"threadId": thread_id}),
                );
            } else {
                state.start_turn(&thread_id, text, Some(client_id), connection, shared.turn);
            }
            Ok(result)
        }
        other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
    };
    let reply = match result {
        Ok(result) => json!({"id": id, "result": result}),
        Err((code, message)) => json!({"id": id, "error": {"code": code, "message": message}}),
    };
    before.push(reply);
    before.extend(after);
    before
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempdir::TempDir;

    fn thread_state(id: &str) -> ThreadState {
        ThreadState {
            thread: json!({"id": id}),
            settings: json!({}),
            turns: Vec::new(),
            subscribers: BTreeSet::new(),
            active: None,
            queue: VecDeque::new(),
        }
    }

    /// A state file with no bytes is treated like a missing file.
    #[test]
    fn load_empty_file_is_empty_state() {
        let dir = TempDir::new("codex-empty").unwrap();
        let file = dir.path().join(THREADS_FILE);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "").unwrap();
        let mut state = State {
            home: Some(dir.path().to_path_buf()),
            ..State::default()
        };
        state.load().unwrap();
        assert!(state.threads.is_empty());
    }

    /// A missing file is empty state, not an error.
    #[test]
    fn load_missing_file_is_empty_state() {
        let dir = TempDir::new("codex-missing").unwrap();
        let mut state = State {
            home: Some(dir.path().to_path_buf()),
            ..State::default()
        };
        state.load().unwrap();
        assert!(state.threads.is_empty());
    }

    /// A non-empty file that fails to parse — the shape a process killed
    /// mid-write left behind before `save` became atomic — is a loud,
    /// clearly labelled error instead of a silent empty start.
    #[test]
    fn load_truncated_file_is_a_clear_error() {
        let dir = TempDir::new("codex-truncated").unwrap();
        let file = dir.path().join(THREADS_FILE);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, r#"{"next_id": 2, "threads": [{"thread": {"id"#).unwrap();
        let mut state = State {
            home: Some(dir.path().to_path_buf()),
            ..State::default()
        };
        let err = state.load().unwrap_err();
        assert!(
            err.contains(&file.display().to_string()),
            "error should name the file: {err}"
        );
        assert!(state.threads.is_empty());
    }

    /// `save` goes through the temp-file-then-rename path: no partial file
    /// is ever observable at the final path.
    #[test]
    fn save_is_atomic_no_partial_file_observable() {
        let dir = TempDir::new("codex-atomic").unwrap();
        let file = dir.path().join(THREADS_FILE);
        let mut state = State {
            home: Some(dir.path().to_path_buf()),
            ..State::default()
        };
        state
            .threads
            .insert("t_fake1".to_owned(), thread_state("t_fake1"));
        state.next_id = 1;
        state.save();

        let text = std::fs::read_to_string(&file).unwrap();
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["threads"][0]["thread"]["id"], "t_fake1");
        let leftovers: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temp file left behind: {leftovers:?}");

        let mut reloaded = State {
            home: Some(dir.path().to_path_buf()),
            ..State::default()
        };
        reloaded.load().unwrap();
        assert!(reloaded.threads.contains_key("t_fake1"));

        // Prove the save actually replaces the directory entry rather than
        // truncating the existing inode in place: a hard link taken right
        // before the next save must keep observing the OLD content, and the
        // inode behind `file` must change. `std::fs::write` truncates the
        // same inode, so a plain-write implementation would make the link
        // observe the NEW content and the inode would stay the same — this
        // is what would let this test pass against a non-atomic save.
        let old = file.with_extension("json.old");
        std::fs::hard_link(&file, &old).unwrap();
        let old_ino = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&old).unwrap());

        state
            .threads
            .insert("t_fake2".to_owned(), thread_state("t_fake2"));
        state.next_id = 2;
        state.save();

        let new_ino = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&file).unwrap());
        assert_ne!(
            old_ino, new_ino,
            "save must replace the file's inode via rename, not truncate it in place"
        );

        let old_text = std::fs::read_to_string(&old).unwrap();
        let old_parsed: Value = serde_json::from_str(&old_text).unwrap();
        assert_eq!(old_parsed["threads"].as_array().unwrap().len(), 1);
        assert_eq!(old_parsed["threads"][0]["thread"]["id"], "t_fake1");

        let new_text = std::fs::read_to_string(&file).unwrap();
        let new_parsed: Value = serde_json::from_str(&new_text).unwrap();
        assert_eq!(new_parsed["threads"].as_array().unwrap().len(), 2);
    }
}
