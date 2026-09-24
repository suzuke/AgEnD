//! `fake-claude`: the subset of Claude Code 2.1.281 in interactive mode that
//! docs/backends/claude-code.md, research/spike-claude.md and
//! spike-claude-f.md record (D16): hooks for state, an MCP channel for
//! messages, `Esc` to interrupt.
//!
//! | Covered | Behaviour |
//! |---|---|
//! | hooks from `.claude/settings.json` (and `--settings <file>`) | `{"hooks": {"<Event>": [{"hooks": [{"type": "command", "command": ...}]}]}}`; each command runs with `sh -c` in the project dir, the JSON payload on stdin and `CLAUDE_PROJECT_DIR` set |
//! | `SessionStart` | `source` `startup` or `resume` (with `--resume <id>`) |
//! | `UserPromptSubmit` | for typed prompts and channel messages, `prompt` holds the text |
//! | `Stop` | after every finished turn, with `stop_hook_active`; a hook printing `{"decision":"block","reason":...}` starts one more turn with the reason (the next Stop has `stop_hook_active: true`) |
//! | `Esc` (byte 0x1b on stdin) | interrupts the running turn; no Stop hook fires (spike F3) |
//! | typed input | stdin bytes up to `\r` or `\n`; typed while busy → runs after the turn |
//! | channel | with `--dangerously-load-development-channels server:<name>`: starts `mcpServers.<name>` from `.mcp.json`, MCP `initialize` over stdio (needs `capabilities.experimental["claude/channel"]`), then each `notifications/claude/channel {content, meta}` becomes the prompt `<channel source="<name>" k="v"...>\n<content>\n</channel>` |
//! | channel while busy | `UserPromptSubmit` fires but the message is not acted on (spike C1: it may be dropped; the fake always drops it), so drivers must queue with the Stop hook |
//!
//! Output lines (stdout): `> <prompt>`, `⏺ fake reply: ...`,
//! `Stop hook error: <reason>` (Claude Code's label for a blocking Stop hook),
//! `Interrupted · What should Claude do instead?`, and the fake-only
//! `fake-claude: idle` after each settled turn so tests can wait for it.
//!
//! Not covered: the TUI screen, startup prompts, tools and
//! `PreToolUse`/`PostToolUse`, permissions, hook exit code 2, hook matchers
//! and timeouts, the channel `reply` tool, the CLAUDE.md source-note effect
//! after `Esc` (spike F: 0/3 vs 3/3), `-p` stream-json.
//!
//! Must NOT: call a model.

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
pub const INTERRUPTED_LINE: &str = "Interrupted · What should Claude do instead?";

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
    stop_hook_active: bool,
}

struct Claude {
    cwd: PathBuf,
    session_id: String,
    transcript: PathBuf,
    hooks: Vec<(String, String)>,
    turn_length: Duration,
    turn: Option<Turn>,
    typed: VecDeque<String>,
    channel_name: Option<String>,
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
    let transcript = std::env::temp_dir()
        .join("fake-claude")
        .join(format!("{session_id}.jsonl"));
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
        channel_name,
    };
    println!(
        "fake-claude: session {} in {}",
        claude.session_id,
        claude.cwd.display()
    );
    let source = if resumed.is_some() {
        "resume"
    } else {
        "startup"
    };
    claude.hook("SessionStart", json!({"source": source, "model": "fake"}));
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
            self.hook("UserPromptSubmit", json!({"prompt": prompt}));
            println!("(channel message arrived while busy; not acted on)");
            return;
        }
        self.start_turn(prompt, false, true);
    }

    fn next_typed(&mut self) {
        if let Some(text) = self.typed.pop_front() {
            self.start_turn(text, false, true);
        }
    }

    fn start_turn(&mut self, prompt: String, stop_hook_active: bool, submit_hook: bool) {
        if submit_hook {
            self.hook("UserPromptSubmit", json!({"prompt": prompt}));
        }
        println!("> {}", prompt.replace('\n', "\\n"));
        self.record("user", &prompt);
        self.turn = Some(Turn {
            deadline: Instant::now() + self.turn_length,
            prompt,
            stop_hook_active,
        });
    }

    fn interrupt(&mut self) {
        if self.turn.take().is_some() {
            println!("{INTERRUPTED_LINE}");
            self.settle();
        }
    }

    fn finish_turn(&mut self) {
        let Some(turn) = self.turn.take() else { return };
        let reply = reply_to(&turn.prompt);
        println!("⏺ {reply}");
        self.record("assistant", &reply);
        let outputs = self.hook("Stop", json!({"stop_hook_active": turn.stop_hook_active}));
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
        let mut payload = json!({
            "session_id": self.session_id,
            "transcript_path": self.transcript,
            "cwd": self.cwd,
            "hook_event_name": event,
            "permission_mode": "default",
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
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "fake-claude", "version": "2.1.281-fake"}}
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
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .map_err(|e| format!("channel initialized: {e}"))?;
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
