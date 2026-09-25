//! `fake-claude`: the subset of Claude Code 2.1.282 in interactive mode that
//! docs/backends/claude-code.md, research/spike-claude.md, spike-claude-f.md
//! (D16) and the recordings in `transcripts/claude/` show: hooks for state,
//! an MCP channel for messages, `Esc` to interrupt. The conformance test
//! (`tests/conformance.rs`) compares its hooks, MCP messages, keys and
//! screen markers with the recordings by shape.
//!
//! | Covered | Behaviour |
//! |---|---|
//! | hooks from `.claude/settings.json` (and `--settings <file>`) | `{"hooks": {"<Event>": [{"hooks": [{"type": "command", "command": ...}]}]}}`; each command runs with `sh -c` in the project dir, the JSON payload on stdin and `CLAUDE_PROJECT_DIR` set. Every payload has `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `scratchpad_dir` |
//! | `SessionStart` | `source` `startup` (with `model`) or `resume` (with `--resume <id>`; `context_tokens`, `estimated_cache_write_usd`, `prompt_cache_likely_expired`, `seconds_since_last_response` instead of `model`) |
//! | `UserPromptSubmit` | `prompt` holds the text; `permission_mode`, `prompt_id` |
//! | `Stop` | after every finished turn: `stop_hook_active`, `last_assistant_message`, `background_tasks`, `session_crons`, `permission_mode`, `prompt_id`; a hook printing `{"decision":"block","reason":...}` starts one more turn with the reason (the next Stop has `stop_hook_active: true`) |
//! | `SessionEnd` | on `/exit`: `reason: "prompt_input_exit"`, `prompt_id` |
//! | `Esc` (byte 0x1b on stdin) | interrupts the running turn; no Stop hook fires (spike F3) |
//! | typed input | stdin bytes up to `\r` or `\n`; typed while busy → runs after the turn |
//! | `run: <command>` on a line of the prompt | the turn asks to use Bash: `PreToolUse` (`tool_name: "Bash"`, `tool_input {command, description}`, `tool_use_id`), then `PermissionRequest`, then the screen shows `Do you want to proceed?`; `Esc` cancels (no more hooks, no Stop). Allowing is not modelled (never recorded); other input waits as while busy |
//! | channel | with `--dangerously-load-development-channels server:<name>`: starts `mcpServers.<name>` from `.mcp.json`, MCP `initialize` (protocol `2025-11-25`, `capabilities {elicitation, roots}`) over stdio (needs `capabilities.experimental["claude/channel"]`), `notifications/initialized`, `tools/list`; then each `notifications/claude/channel {content, meta}` becomes the prompt `<channel source="<name>" k="v"...>\n<content>\n</channel>` |
//! | channel while busy | **deliberately differs from real claude.** Real 2.1.282 queues the message and, after the turn (and any Stop-hook turn), fires `UserPromptSubmit` and answers it (recording `busy`). The fake fires `UserPromptSubmit` at that same point but never acts on the message (no turn, no Stop): the worst case of spike C1, kept because D16 requires drivers to queue busy messages through the Stop hook (gate-02 A6); the conformance check lists this in `shape::DELIBERATE` |
//!
//! Output lines (stdout): `> <prompt>`, `⏺ fake reply: ...`,
//! `Stop hook error: <reason>` (Claude Code's label for a blocking Stop hook),
//! `Interrupted · What should Claude do instead?`, `Do you want to proceed?`,
//! and the fake-only `fake-claude: idle` after each settled turn so tests can
//! wait for it.
//!
//! Not covered: the TUI screen, startup dialogs, tools other than the Bash
//! permission request, `PostToolUse`, `Notification`, hook exit code 2, hook
//! matchers and timeouts, the channel `reply` tool, the CLAUDE.md
//! source-note effect after `Esc` (spike F: 0/3 vs 3/3), `-p` stream-json.
//!
//! Must NOT: call a model or run the command it asks permission for.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::{Args, reply_to};

pub const ESC: u8 = 0x1b;
pub const IDLE_LINE: &str = "fake-claude: idle";
/// Where transcripts go, relative to the project dir (the real Claude Code
/// writes under `~/.claude/projects/`; the fake keeps them in the project).
pub const TRANSCRIPT_DIR: &str = ".claude/fake-transcripts";
pub const INTERRUPTED_LINE: &str = "Interrupted · What should Claude do instead?";
pub const PERMISSION_LINE: &str = "Do you want to proceed?";
/// Reported as `scratchpad_dir` (`<dir>/<session>`), never created.
pub const SCRATCHPAD_DIR: &str = ".claude/fake-scratchpad";

/// The command of the first `run: <command>` line in a prompt.
fn run_command(prompt: &str) -> Option<String> {
    prompt
        .lines()
        .find_map(|l| l.trim().strip_prefix("run: "))
        .map(|c| c.trim().to_owned())
}

enum Event {
    Input(Vec<u8>),
    InputClosed,
    Channel {
        content: String,
        meta: Map<String, Value>,
    },
}

struct Turn {
    deadline: Instant,
    prompt: String,
    prompt_id: String,
    stop_hook_active: bool,
    /// `Some(command)` while the turn waits for a Bash permission answer.
    asking: Option<String>,
}

struct Claude {
    cwd: PathBuf,
    session_id: String,
    transcript: PathBuf,
    hooks: Vec<(String, String)>,
    turn_length: Duration,
    turn: Option<Turn>,
    typed: VecDeque<String>,
    /// Channel messages that arrived while busy (see the module docs).
    busy_channel: VecDeque<String>,
    channel_name: Option<String>,
    next_id: u64,
}

pub fn main(args: impl IntoIterator<Item = String>) -> ExitCode {
    let known = [
        "--resume",
        "--session-id",
        "--dangerously-load-development-channels",
        "--settings",
        "--turn-ms",
        "--model",
        "--permission-mode",
        "--effort",
        "--setting-sources",
    ];
    let switches = ["--dangerously-skip-permissions"];
    let args = match Args::parse(args, &known, &switches, &[]) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("fake-claude: {e}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fake-claude: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| format!("current dir: {e}"))?;
    let resumed = args.get("--resume");
    let session_id = resumed.or(args.get("--session-id")).map_or_else(
        || format!("00000000-0000-4000-8000-{:012}", std::process::id()),
        str::to_owned,
    );
    // Inside the project, so a test's temp project takes it along on drop.
    let transcript = cwd.join(TRANSCRIPT_DIR).join(format!("{session_id}.jsonl"));
    let mut hooks = load_hooks(&cwd.join(".claude/settings.json"))?;
    if let Some(extra) = args.get("--settings") {
        hooks.extend(load_hooks(Path::new(extra))?);
    }
    let (tx, rx) = mpsc::channel();
    let channel_name = args
        .get("--dangerously-load-development-channels")
        .map(|spec| spec.strip_prefix("server:").unwrap_or(spec).to_owned());
    let mut channel = None;
    if let Some(name) = &channel_name {
        channel = Some(start_channel(&cwd, name, tx.clone())?);
    }
    spawn_stdin_reader(tx);
    let mut claude = Claude {
        cwd,
        session_id,
        transcript,
        hooks,
        turn_length: Duration::from_millis(args.turn_ms()?),
        turn: None,
        typed: VecDeque::new(),
        busy_channel: VecDeque::new(),
        channel_name,
        next_id: 0,
    };
    println!(
        "fake-claude: session {} in {}",
        claude.session_id,
        claude.cwd.display()
    );
    let start = if resumed.is_some() {
        json!({"source": "resume", "context_tokens": 0, "estimated_cache_write_usd": 0.0,
               "prompt_cache_likely_expired": false, "seconds_since_last_response": 0})
    } else {
        json!({"source": "startup", "model": "fake"})
    };
    claude.hook("SessionStart", start);
    println!("{IDLE_LINE}");
    let result = claude.event_loop(&rx);
    if let Some((mut child, stdin)) = channel {
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

impl Claude {
    fn event_loop(&mut self, rx: &Receiver<Event>) -> Result<(), String> {
        let mut line = Vec::new();
        loop {
            let event = match self.turn.as_ref().map(|t| t.deadline) {
                Some(deadline) => {
                    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(event) => event,
                        Err(RecvTimeoutError::Timeout) => {
                            self.finish_turn();
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => return Ok(()),
                    }
                }
                None => match rx.recv() {
                    Ok(event) => event,
                    Err(_) => return Ok(()),
                },
            };
            match event {
                Event::InputClosed => return Ok(()),
                Event::Input(bytes) => {
                    for byte in bytes {
                        match byte {
                            ESC => self.interrupt(),
                            b'\r' | b'\n' => {
                                let text = String::from_utf8_lossy(&line).trim().to_owned();
                                line.clear();
                                if text == "/exit" {
                                    let id = self.prompt_id();
                                    self.hook(
                                        "SessionEnd",
                                        json!({"reason": "prompt_input_exit", "prompt_id": id}),
                                    );
                                    return Ok(());
                                }
                                if !text.is_empty() {
                                    self.typed.push_back(text);
                                }
                                // Enter submits at once, so a following Esc
                                // in the same read interrupts this turn.
                                if self.turn.is_none() {
                                    self.next_typed();
                                }
                            }
                            _ => line.push(byte),
                        }
                    }
                }
                Event::Channel { content, meta } => self.channel_message(&content, &meta),
            }
        }
    }

    fn channel_message(&mut self, content: &str, meta: &Map<String, Value>) {
        let source = self.channel_name.clone().unwrap_or_default();
        let mut open = format!("<channel source=\"{source}\"");
        for (key, value) in meta {
            let value = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned);
            open.push_str(&format!(" {key}=\"{value}\""));
        }
        let prompt = format!("{open}>\n{content}\n</channel>");
        if self.turn.is_some() {
            self.busy_channel.push_back(prompt);
            return;
        }
        self.start_turn(prompt, false, true);
    }

    fn next_typed(&mut self) {
        if let Some(text) = self.typed.pop_front() {
            self.start_turn(text, false, true);
        }
    }

    fn prompt_id(&mut self) -> String {
        self.next_id += 1;
        format!("00000000-0000-4000-9000-{:012}", self.next_id)
    }

    fn start_turn(&mut self, prompt: String, stop_hook_active: bool, submit_hook: bool) {
        let prompt_id = self.prompt_id();
        if submit_hook {
            self.hook(
                "UserPromptSubmit",
                json!({"prompt": prompt, "permission_mode": "default", "prompt_id": prompt_id}),
            );
        }
        println!("> {}", prompt.replace('\n', "\\n"));
        self.record("user", &prompt);
        self.turn = Some(Turn {
            deadline: Instant::now() + self.turn_length,
            prompt,
            prompt_id,
            stop_hook_active,
            asking: None,
        });
    }

    fn interrupt(&mut self) {
        if self.turn.take().is_some() {
            println!("{INTERRUPTED_LINE}");
            self.settle();
        }
    }

    fn finish_turn(&mut self) {
        let Some(mut turn) = self.turn.take() else {
            return;
        };
        if turn.asking.is_some() {
            // Waiting for a permission answer: no deadline applies.
            turn.deadline = Instant::now() + Duration::from_secs(3600);
            self.turn = Some(turn);
            return;
        }
        if let Some(command) = run_command(&turn.prompt) {
            let input = json!({"command": command, "description": format!("Run {command}")});
            let common = json!({"permission_mode": "default", "prompt_id": turn.prompt_id,
                                "tool_name": "Bash", "tool_input": input});
            let mut pre = common.clone();
            pre["tool_use_id"] = json!(format!("toolu_fake{:016}", self.next_id));
            self.hook("PreToolUse", pre);
            self.hook("PermissionRequest", common);
            println!("{PERMISSION_LINE}");
            turn.asking = Some(command);
            turn.deadline = Instant::now() + Duration::from_secs(3600);
            self.turn = Some(turn);
            return;
        }
        let reply = reply_to(&turn.prompt);
        println!("⏺ {reply}");
        self.record("assistant", &reply);
        let outputs = self.hook(
            "Stop",
            json!({"stop_hook_active": turn.stop_hook_active, "last_assistant_message": reply,
                   "background_tasks": [], "session_crons": [], "permission_mode": "default",
                   "prompt_id": turn.prompt_id}),
        );
        let block = outputs
            .iter()
            .find(|o| o["decision"] == "block")
            .and_then(|o| o["reason"].as_str().map(str::to_owned));
        match block {
            Some(reason) => {
                println!("Stop hook error: {}", reason.replace('\n', "\\n"));
                self.start_turn(reason, true, false);
            }
            None => self.settle(),
        }
    }

    fn settle(&mut self) {
        while let Some(prompt) = self.busy_channel.pop_front() {
            let id = self.prompt_id();
            self.hook(
                "UserPromptSubmit",
                json!({"prompt": prompt, "permission_mode": "default", "prompt_id": id}),
            );
            println!("(channel message arrived while busy; not acted on)");
        }
        self.next_typed();
        if self.turn.is_none() {
            println!("{IDLE_LINE}");
        }
    }

    fn record(&self, role: &str, text: &str) {
        if let Some(dir) = self.transcript.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let line = json!({"type": role, "text": text}).to_string();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.transcript);
        if let Ok(mut file) = file {
            let _ = writeln!(file, "{line}");
        }
    }

    /// Runs every hook for `event`; returns the JSON each one printed.
    fn hook(&self, event: &str, extra: Value) -> Vec<Value> {
        let scratchpad = self.cwd.join(SCRATCHPAD_DIR).join(&self.session_id);
        let mut payload = json!({
            "session_id": self.session_id,
            "transcript_path": self.transcript,
            "cwd": self.cwd,
            "hook_event_name": event,
            "scratchpad_dir": scratchpad,
        });
        if let (Some(payload), Value::Object(extra)) = (payload.as_object_mut(), extra) {
            payload.extend(extra);
        }
        let input = payload.to_string();
        self.hooks
            .iter()
            .filter(|(name, _)| name == event)
            .filter_map(|(_, command)| run_hook(&self.cwd, command, &input))
            .collect()
    }
}

fn run_hook(cwd: &Path, command: &str, input: &str) -> Option<Value> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env("CLAUDE_PROJECT_DIR", cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

/// `(event, command)` pairs from a settings file; a missing file has none.
fn load_hooks(path: &Path) -> Result<Vec<(String, String)>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let settings: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hooks = Vec::new();
    for (event, groups) in settings["hooks"].as_object().into_iter().flatten() {
        for group in groups.as_array().into_iter().flatten() {
            for hook in group["hooks"].as_array().into_iter().flatten() {
                if hook["type"] == "command"
                    && let Some(command) = hook["command"].as_str()
                {
                    hooks.push((event.clone(), command.to_owned()));
                }
            }
        }
    }
    Ok(hooks)
}

fn spawn_stdin_reader(tx: Sender<Event>) {
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0u8; 1024];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(Event::InputClosed);
                    return;
                }
                Ok(n) => {
                    if tx.send(Event::Input(buffer[..n].to_vec())).is_err() {
                        return;
                    }
                }
            }
        }
    });
}

/// Starts the MCP channel server `name` from `.mcp.json` and completes the
/// MCP handshake; notifications are forwarded as `Event::Channel`.
fn start_channel(cwd: &Path, name: &str, tx: Sender<Event>) -> Result<(Child, ChildStdin), String> {
    let config_path = cwd.join(".mcp.json");
    let text = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("{}: {e}", config_path.display()))?;
    let config: Value = serde_json::from_str(&text).map_err(|e| format!(".mcp.json: {e}"))?;
    let server = &config["mcpServers"][name];
    let command = server["command"]
        .as_str()
        .ok_or_else(|| format!("server:{name} · no MCP server configured with that name"))?;
    let mut child = Command::new(command)
        .args(
            server["args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        )
        .envs(
            server["env"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(k, v)| v.as_str().map(|v| (k, v))),
        )
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("start channel server {name}: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    let initialize = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-11-25",
                   "capabilities": {"elicitation": {}, "roots": {"listChanged": true}},
                   "clientInfo": {"name": "fake-claude", "title": "fake-claude", "version": "2.1.282-fake",
                                  "description": "agend-testkit fake of Claude Code",
                                  "websiteUrl": "https://github.com/suzuke/AgEnD"}}
    });
    writeln!(stdin, "{initialize}").map_err(|e| format!("channel initialize: {e}"))?;
    let mut line = String::new();
    stdout
        .read_line(&mut line)
        .map_err(|e| format!("channel initialize: {e}"))?;
    let response: Value =
        serde_json::from_str(&line).map_err(|e| format!("channel initialize response: {e}"))?;
    if response["result"]["capabilities"]["experimental"]["claude/channel"].is_null() {
        return Err(format!(
            "server:{name} does not declare the claude/channel capability"
        ));
    }
    for message in [
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    ] {
        writeln!(stdin, "{message}").map_err(|e| format!("channel initialized: {e}"))?;
    }
    std::thread::spawn(move || {
        for line in stdout.lines().map_while(Result::ok) {
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if message["method"] != "notifications/claude/channel" {
                continue;
            }
            let params = &message["params"];
            let event = Event::Channel {
                content: params["content"].as_str().unwrap_or_default().to_owned(),
                meta: params["meta"].as_object().cloned().unwrap_or_default(),
            };
            if tx.send(event).is_err() {
                return;
            }
        }
    });
    Ok((child, stdin))
}
