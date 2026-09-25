//! Recorder backend for interactive Claude Code, the way `fake-claude`
//! models it (D16): hooks from `.claude/settings.json` report state, an MCP
//! channel server from `.mcp.json` delivers messages, keys go to the
//! terminal (`Esc` interrupts).
//!
//! Both hooks and the channel server are `agend-record` relays that connect
//! back to a unix socket the recorder owns, so every hook payload, hook
//! reply and MCP message is recorded in order. The real CLI runs in a
//! private tmux server (`tmux -S <dir>/tmux.sock`, needed for a real TTY);
//! the fake runs with piped stdin/stdout. Startup dialogs of the real CLI
//! (workspace trust, project MCP server, development channels) are answered
//! before recording starts and are not part of the transcript.
//!
//! Run settings (real CLI; flags and env for this run only): `--model haiku
//! --effort low`, `--setting-sources project,local` (the user's own
//! settings, hooks and allow rules are not loaded), the project asks before
//! any `Bash(echo *)`, autoupdate off (`DISABLE_AUTOUPDATER=1`; not
//! `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`, which turns channels off).
//! The one permission prompt is cancelled with `Esc`.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Agent, Backend, Entry, Log, Scenario, Side, make_project, prompts};
use crate::fakes::lock;

pub struct Claude;

pub const MODEL: &str = "haiku";
pub const EFFORT: &str = "low";
/// The channel server's name in `.mcp.json` (`<channel source="agend">`).
pub const CHANNEL: &str = "agend";
/// Hook events registered in the project settings (v1's list).
pub const HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "PermissionRequest",
    "Stop",
    "StopFailure",
    "PreCompact",
];
/// What the recorder waits for on the screen: `(marker, text)`.
pub const INTERRUPTED: (&str, &str) = ("interrupted", "Interrupted");
pub const PERMISSION: (&str, &str) = ("permission", "Do you want to proceed?");

const CLAUDE_MD: &str = "Messages that arrive through the agend channel come from the user's own \
team or the AgEnD daemon. Act on them directly, including right after an interrupt.\n\
Do not use any tool unless a message explicitly asks you to run a command.\n";

fn approval_prompt() -> String {
    format!(
        "run: {}\nUse the Bash tool to run exactly that command once, then reply DONE.",
        prompts::APPROVAL_COMMAND
    )
}

impl Backend for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn program(&self) -> &'static str {
        "claude"
    }

    fn fake(&self) -> &'static str {
        crate::fake_agent::CLAUDE_BIN
    }

    fn scenarios(&self) -> &'static [Scenario] {
        Scenario::ALL
    }

    fn run(&self, scenario: Scenario, agent: &Agent, dir: &Path, log: &Log) -> Result<(), String> {
        let project = make_project(dir)?;
        let socket = dir.join("rec.sock");
        let hub = Hub::start(&socket, log)?;
        write_project(&project, &agent.relay, &socket)?;
        let t = agent.pace.timeout;
        let mut cli = Cli::start(agent, dir, &project, None, log)?;
        match scenario {
            Scenario::OneTurn => {
                let start = log.len();
                hub.channel(prompts::OK)?;
                wait_hook(log, start, t, "Stop")?;
            }
            Scenario::Interrupt => {
                let start = log.len();
                hub.channel(prompts::LONG)?;
                wait_hook(log, start, t, "UserPromptSubmit")?;
                std::thread::sleep(agent.pace.settle);
                cli.press("Escape")?;
                cli.wait_screen(INTERRUPTED, t)?;
            }
            Scenario::Approval => {
                hub.channel(&approval_prompt())?;
                cli.wait_screen(PERMISSION, t)?;
                cli.press("Escape")?;
            }
            Scenario::Busy => {
                let start = log.len();
                hub.channel(prompts::LONG)?;
                wait_hook(log, start, t, "UserPromptSubmit")?;
                std::thread::sleep(agent.pace.settle);
                hub.queue_for_stop("Reply with exactly: QUEUED");
                hub.channel("Reply with exactly: OK2")?;
                log.wait(start, t, "Stop with stop_hook_active", |e| {
                    is_hook(e, "Stop") && e.msg["stop_hook_active"] == true
                })?;
            }
            Scenario::Resume => {
                let start = log.len();
                hub.channel(prompts::OK)?;
                wait_hook(log, start, t, "Stop")?;
                let (_, started) = wait_hook(log, 0, t, "SessionStart")?;
                let session = started.str("session_id").ok_or("no session_id")?.to_owned();
                cli.quiet(agent);
                cli.exit(t)?;
                cli = Cli::start(agent, dir, &project, Some(&session), log)?;
                let start = log.len();
                cli.type_text(prompts::OK)?;
                cli.press("Enter")?;
                wait_hook(log, start, t, "Stop")?;
            }
        }
        cli.quiet(agent);
        cli.exit(t)?;
        drop(hub);
        Ok(())
    }
}

/// Starts the real CLI in a fresh `/private/tmp/agend-rec-claude-*` dir,
/// answers its startup dialogs, waits for SessionStart and the channel, and
/// exits with `/exit`: no prompt, no tokens. Returns the hooks and MCP
/// messages seen.
pub fn startup_check(relay: &Path) -> Result<Vec<Entry>, String> {
    let dir = super::mktemp_dir("claude")?;
    let agent = Agent {
        program: PathBuf::from("claude"),
        fake: false,
        relay: relay.to_path_buf(),
        pace: super::REAL_PACE,
    };
    let log = Log::default();
    let result = (|| {
        let project = make_project(&dir)?;
        let socket = dir.join("rec.sock");
        let _hub = Hub::start(&socket, &log)?;
        write_project(&project, relay, &socket)?;
        let mut cli = Cli::start(&agent, &dir, &project, None, &log)?;
        cli.exit(Duration::from_secs(30))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.map(|()| log.entries())
}

fn is_hook(e: &Entry, event: &str) -> bool {
    e.via == "hook" && e.from == Side::Backend && e.str("hook_event_name") == Some(event)
}

fn wait_hook(log: &Log, start: usize, t: Duration, event: &str) -> Result<(usize, Entry), String> {
    log.wait(start, t, &format!("hook {event}"), |e| is_hook(e, event))
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// `.claude/settings.json` (hooks, ask before `echo`), `.mcp.json`
/// (the channel relay) and `CLAUDE.md` (D16 source framing).
fn write_project(project: &Path, relay: &Path, socket: &Path) -> Result<(), String> {
    let relay = shell_quote(&relay.to_string_lossy());
    let command = format!(
        "{relay} hook-relay {}",
        shell_quote(&socket.to_string_lossy())
    );
    let mut hooks = serde_json::Map::new();
    for event in HOOK_EVENTS {
        hooks.insert(
            (*event).to_owned(),
            json!([{"hooks": [{"type": "command", "command": command}]}]),
        );
    }
    let settings = json!({"hooks": hooks, "permissions": {"ask": ["Bash(echo *)"]}});
    let mcp = json!({"mcpServers": {CHANNEL: {
        "command": relay.trim_matches('\''),
        "args": ["channel-relay", socket],
    }}});
    let write = |path: PathBuf, text: String| {
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))
    };
    std::fs::create_dir_all(project.join(".claude")).map_err(|e| e.to_string())?;
    write(project.join(".claude/settings.json"), settings.to_string())?;
    write(project.join(".mcp.json"), mcp.to_string())?;
    write(project.join("CLAUDE.md"), CLAUDE_MD.to_owned())
}

/// The recorder's end of the hook and channel relays.
struct Hub {
    log: Log,
    stop_queue: Arc<Mutex<VecDeque<String>>>,
    mcp: Arc<Mutex<Option<UnixStream>>>,
    socket: PathBuf,
    deliveries: Mutex<u64>,
}

impl Hub {
    fn start(socket: &Path, log: &Log) -> Result<Hub, String> {
        let listener =
            UnixListener::bind(socket).map_err(|e| format!("bind {}: {e}", socket.display()))?;
        let hub = Hub {
            log: log.clone(),
            stop_queue: Arc::default(),
            mcp: Arc::default(),
            socket: socket.to_path_buf(),
            deliveries: Mutex::new(0),
        };
        let (log, queue, mcp) = (
            log.clone(),
            Arc::clone(&hub.stop_queue),
            Arc::clone(&hub.mcp),
        );
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (log, queue, mcp) = (log.clone(), Arc::clone(&queue), Arc::clone(&mcp));
                std::thread::spawn(move || serve(stream, &log, &queue, &mcp));
            }
        });
        Ok(hub)
    }

    /// The next Stop hook blocks with `message` as the reason (D16 queue).
    fn queue_for_stop(&self, message: &str) {
        lock(&self.stop_queue).push_back(message.to_owned());
    }

    /// Sends `notifications/claude/channel` through the connected channel server.
    fn channel(&self, content: &str) -> Result<(), String> {
        let n = {
            let mut n = lock(&self.deliveries);
            *n += 1;
            *n
        };
        let message = json!({"jsonrpc": "2.0", "method": "notifications/claude/channel", "params": {
            "content": content,
            "meta": {"sender_id": "agend-record", "delivery_id": format!("d{n}")},
        }});
        let mut mcp = lock(&self.mcp);
        let stream = mcp.as_mut().ok_or("the channel server is not connected")?;
        self.log.push(Side::Client, "mcp", message.clone());
        writeln!(stream, "{message}").map_err(|e| format!("channel write: {e}"))
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn serve(
    stream: UnixStream,
    log: &Log,
    queue: &Mutex<VecDeque<String>>,
    mcp: &Mutex<Option<UnixStream>>,
) {
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let mut lines = BufReader::new(stream).lines();
    let Some(Ok(first)) = lines.next() else {
        return;
    };
    let Ok(first) = serde_json::from_str::<Value>(&first) else {
        return;
    };
    match first["role"].as_str() {
        Some("hook") => {
            let payload = first["input"].clone();
            let event = payload["hook_event_name"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            log.push(Side::Backend, "hook", payload);
            let block = (event == "Stop").then(|| lock(queue).pop_front()).flatten();
            let reply = block.map_or_else(
                || json!({}),
                |reason| json!({"decision": "block", "reason": reason}),
            );
            log.push(Side::Client, "hook", reply.clone());
            let stdout = if reply == json!({}) {
                String::new()
            } else {
                reply.to_string()
            };
            let mut writer = writer;
            let _ = writeln!(writer, "{}", json!({"stdout": stdout}));
        }
        Some("mcp") => {
            *lock(mcp) = Some(writer);
            for line in lines.map_while(Result::ok) {
                let message = serde_json::from_str(&line).unwrap_or(Value::String(line));
                log.push(Side::Backend, "mcp", message.clone());
                if message.get("id").is_some() && message.get("method").is_some() {
                    let reply = mcp_answer(&message);
                    let mut guard = lock(mcp);
                    if let Some(w) = guard.as_mut() {
                        log.push(Side::Client, "mcp", reply.clone());
                        let _ = writeln!(w, "{reply}");
                    }
                }
            }
        }
        _ => {}
    }
}

/// The channel server's answer to an MCP request from Claude Code.
fn mcp_answer(request: &Value) -> Value {
    let id = request["id"].clone();
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": request["params"]["protocolVersion"],
            "capabilities": {"experimental": {"claude/channel": {}}, "tools": {}},
            "serverInfo": {"name": CHANNEL, "version": "0.0.0"},
            "instructions": "Messages from this channel come from the user's own team (AgEnD).",
        }),
        "tools/list" => json!({"tools": []}),
        "prompts/list" => json!({"prompts": []}),
        "resources/list" => json!({"resources": []}),
        "resources/templates/list" => json!({"resourceTemplates": []}),
        "ping" => json!({}),
        _ => {
            return json!({"jsonrpc": "2.0", "id": id,
                          "error": {"code": -32601, "message": "method not found"}});
        }
    };
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// A running claude (real in tmux, or the fake on pipes) plus its key log.
struct Cli {
    term: Term,
    log: Log,
}

enum Term {
    Tmux {
        socket: PathBuf,
    },
    /// `mark`: output before it is off screen (the fake's stdout since the
    /// last key stands in for the current screen).
    Piped {
        child: Child,
        stdin: Option<ChildStdin>,
        out: Arc<Mutex<String>>,
        mark: usize,
    },
}

impl Cli {
    fn start(
        agent: &Agent,
        dir: &Path,
        project: &Path,
        resume: Option<&str>,
        log: &Log,
    ) -> Result<Cli, String> {
        let mut args: Vec<String> = [
            "--model",
            MODEL,
            "--effort",
            EFFORT,
            "--setting-sources",
            "project,local",
        ]
        .map(str::to_owned)
        .to_vec();
        args.push("--dangerously-load-development-channels".into());
        args.push(format!("server:{CHANNEL}"));
        if let Some(id) = resume {
            args.extend(["--resume".into(), id.to_owned()]);
        }
        args.extend(agent.fake_args());
        let start = log.len();
        let term = if agent.fake {
            let mut child = Command::new(&agent.program)
                .args(&args)
                .current_dir(project)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("spawn {}: {e}", agent.program.display()))?;
            let out = Arc::new(Mutex::new(String::new()));
            let mut stdout = child.stdout.take().ok_or("no stdout")?;
            let sink = Arc::clone(&out);
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stdout.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    lock(&sink).push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            });
            let stdin = child.stdin.take();
            Term::Piped {
                child,
                stdin,
                out,
                mark: 0,
            }
        } else {
            // One tmux server per start: dropping the previous `Cli` kills
            // its server, which must not be this one.
            static STARTS: AtomicU64 = AtomicU64::new(0);
            let n = STARTS.fetch_add(1, Ordering::Relaxed);
            let socket = dir.join(format!("tmux-{n}.sock"));
            let mut argv: Vec<String> =
                ["env", "DISABLE_AUTOUPDATER=1"].map(str::to_owned).to_vec();
            argv.push(agent.program.to_string_lossy().into_owned());
            argv.extend(args);
            let status = Command::new("tmux")
                .arg("-S")
                .arg(&socket)
                .args([
                    "-f",
                    "/dev/null",
                    "new-session",
                    "-d",
                    "-s",
                    "rec",
                    "-x",
                    "200",
                    "-y",
                    "50",
                    "-c",
                ])
                .arg(project)
                .args(&argv)
                .status()
                .map_err(|e| format!("tmux: {e}"))?;
            if !status.success() {
                return Err(format!("tmux new-session failed: {status}"));
            }
            Term::Tmux { socket }
        };
        let mut cli = Cli {
            term,
            log: log.clone(),
        };
        cli.ready(start, agent.pace.timeout)?;
        Ok(cli)
    }

    /// Answers startup dialogs (real CLI only, not recorded) until the
    /// SessionStart hook ran and the channel server is initialized.
    fn ready(&mut self, start: usize, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let screen = self.screen();
            if matches!(self.term, Term::Tmux { .. }) {
                let dialog = [
                    "Yes, I trust this folder",
                    "I am using this for local development",
                    "Use this MCP server",
                ]
                .into_iter()
                .find(|option| screen.contains(option));
                if let Some(option) = dialog {
                    self.select(option)?;
                    std::thread::sleep(Duration::from_millis(1500));
                    continue;
                }
            }
            let session = self
                .log
                .wait(start, Duration::ZERO, "", |e| is_hook(e, "SessionStart"));
            let channel = self.log.wait(start, Duration::ZERO, "", |e| {
                e.via == "mcp" && e.str("method") == Some("notifications/initialized")
            });
            if session.is_ok() && channel.is_ok() {
                // Let the input box settle before typing.
                std::thread::sleep(Duration::from_millis(500));
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "claude not ready (SessionStart {}, channel {}); screen:\n{screen}",
                    session.is_ok(),
                    channel.is_ok()
                ));
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    /// Moves the dialog cursor (`❯`) to the line containing `option` and
    /// presses Enter only once the screen shows the cursor there (a key
    /// sent before the dialog takes input is lost, and Enter on the default
    /// "No, exit" would quit).
    fn select(&mut self, option: &str) -> Result<(), String> {
        std::thread::sleep(Duration::from_millis(1000));
        for _ in 0..12 {
            let screen = self.screen();
            let lines: Vec<&str> = screen.lines().collect();
            let cursor = lines.iter().position(|l| l.contains('❯'));
            let target = lines.iter().position(|l| l.contains(option));
            let (Some(cursor), Some(target)) = (cursor, target) else {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            };
            if cursor == target {
                return self.send_key("Enter");
            }
            self.send_key(if target > cursor { "Down" } else { "Up" })?;
            std::thread::sleep(Duration::from_millis(500));
        }
        Err(format!(
            "could not move the dialog cursor to {option:?}:\n{}",
            self.screen()
        ))
    }

    fn screen(&self) -> String {
        match &self.term {
            Term::Tmux { socket } => Command::new("tmux")
                .arg("-S")
                .arg(socket)
                .args(["capture-pane", "-p", "-J", "-t", "rec"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default(),
            Term::Piped { out, mark, .. } => lock(out).get(*mark..).unwrap_or_default().to_owned(),
        }
    }

    fn send_key(&mut self, key: &str) -> Result<(), String> {
        match &mut self.term {
            Term::Tmux { socket } => tmux_keys(socket, &[key]),
            Term::Piped {
                stdin, out, mark, ..
            } => {
                *mark = lock(out).len();
                let bytes: &[u8] = match key {
                    "Enter" => b"\r",
                    "Escape" => b"\x1b",
                    "Up" => b"\x1b[A",
                    "Down" => b"\x1b[B",
                    other => return Err(format!("unknown key {other}")),
                };
                let stdin = stdin.as_mut().ok_or("stdin closed")?;
                stdin
                    .write_all(bytes)
                    .and_then(|()| stdin.flush())
                    .map_err(|e| e.to_string())
            }
        }
    }

    /// Presses a key (recorded). Never Enter while a permission dialog is
    /// open: its default option approves.
    fn press(&mut self, key: &str) -> Result<(), String> {
        if key == "Enter" && self.screen().contains(PERMISSION.1) {
            self.send_key("Escape")?;
            return Err(
                "a permission dialog was open; cancelled it instead of pressing Enter".into(),
            );
        }
        self.log.push(Side::Client, "key", json!({"key": key}));
        self.send_key(key)
    }

    /// Types text without submitting it (recorded).
    fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.log.push(Side::Client, "key", json!({"text": text}));
        match &mut self.term {
            Term::Tmux { socket } => tmux_keys(socket, &["-l", text])?,
            Term::Piped {
                stdin, out, mark, ..
            } => {
                *mark = lock(out).len();
                let stdin = stdin.as_mut().ok_or("stdin closed")?;
                stdin
                    .write_all(text.as_bytes())
                    .map_err(|e| e.to_string())?;
            }
        }
        // Keep typed text and the following key apart (spike: Enter races).
        std::thread::sleep(Duration::from_millis(400));
        Ok(())
    }

    fn wait_screen(&self, (marker, text): (&str, &str), timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.screen().contains(text) {
                self.log
                    .push(Side::Backend, "screen", json!({"marker": marker}));
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("{text:?} not on screen:\n{}", self.screen()));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Waits until no hook fires for `quiet`.
    fn quiet(&self, agent: &Agent) {
        self.log
            .wait_quiet(agent.pace.quiet, agent.pace.timeout, |e| e.via == "hook");
    }

    fn alive(&mut self) -> bool {
        match &mut self.term {
            Term::Tmux { socket } => Command::new("tmux")
                .arg("-S")
                .arg(socket.as_path())
                .args(["has-session", "-t", "rec"])
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success()),
            Term::Piped { child, .. } => matches!(child.try_wait(), Ok(None)),
        }
    }

    /// `/exit`, then waits for the process to end and late hooks to arrive.
    fn exit(&mut self, timeout: Duration) -> Result<(), String> {
        for _ in 0..5 {
            if !self.screen().contains(PERMISSION.1) {
                break;
            }
            self.send_key("Escape")?;
            std::thread::sleep(Duration::from_millis(500));
        }
        self.type_text("/exit")?;
        self.press("Enter")?;
        let deadline = Instant::now() + timeout.min(Duration::from_secs(30));
        while self.alive() {
            if Instant::now() >= deadline {
                return Err(format!(
                    "claude did not exit after /exit; screen:\n{}",
                    self.screen()
                ));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        std::thread::sleep(Duration::from_millis(500));
        Ok(())
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        match &mut self.term {
            Term::Tmux { socket } => {
                let _ = Command::new("tmux")
                    .arg("-S")
                    .arg(socket.as_path())
                    .arg("kill-server")
                    .stderr(Stdio::null())
                    .status();
            }
            Term::Piped { child, stdin, .. } => {
                drop(stdin.take());
                let deadline = Instant::now() + Duration::from_secs(3);
                while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(20));
                }
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn tmux_keys(socket: &Path, keys: &[&str]) -> Result<(), String> {
    let status = Command::new("tmux")
        .arg("-S")
        .arg(socket)
        .args(["send-keys", "-t", "rec"])
        .args(keys)
        .status()
        .map_err(|e| format!("tmux send-keys: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("tmux send-keys {keys:?}: {status}"))
    }
}

/// `agend-record hook-relay <socket>`: the hook command. Sends the payload
/// from stdin to the recorder and prints the recorder's reply.
pub fn hook_relay(socket: &Path) -> ExitCode {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload = serde_json::from_str(&input).unwrap_or(Value::String(input));
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return ExitCode::SUCCESS;
    };
    let _ = writeln!(stream, "{}", json!({"role": "hook", "input": payload}));
    let mut reply = String::new();
    let _ = BufReader::new(stream).read_line(&mut reply);
    let reply: Value = serde_json::from_str(&reply).unwrap_or_default();
    if let Some(stdout) = reply["stdout"].as_str()
        && !stdout.is_empty()
    {
        println!("{stdout}");
    }
    ExitCode::SUCCESS
}

/// `agend-record channel-relay <socket>`: the MCP channel server. Pipes
/// stdio lines to and from the recorder, which speaks MCP.
pub fn channel_relay(socket: &Path) -> ExitCode {
    let Ok(mut stream) = UnixStream::connect(socket) else {
        eprintln!(
            "agend-record channel-relay: cannot connect to {}",
            socket.display()
        );
        return ExitCode::FAILURE;
    };
    let _ = writeln!(stream, "{}", json!({"role": "mcp"}));
    let Ok(reader) = stream.try_clone() else {
        return ExitCode::FAILURE;
    };
    std::thread::spawn(move || {
        let mut stdout = std::io::stdout();
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if writeln!(stdout, "{line}")
                .and_then(|()| stdout.flush())
                .is_err()
            {
                break;
            }
        }
        std::process::exit(0);
    });
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        if writeln!(stream, "{line}").is_err() {
            break;
        }
    }
    ExitCode::SUCCESS
}
