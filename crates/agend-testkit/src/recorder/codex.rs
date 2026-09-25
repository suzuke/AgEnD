//! Recorder backend for `codex app-server`: JSON-RPC over WebSocket over a
//! unix socket (`--listen unix://<dir>/codex.sock`), the transport
//! `fake-codex-app-server` emulates.
//!
//! Run settings (real CLI; all on the command line, nothing written to
//! `~/.codex/config.toml`): model `gpt-6-luna`, reasoning effort `low`,
//! approval policy `untrusted`, sandbox `read-only`; `notify`, MCP servers
//! and hooks from the user's config are switched off for the run.
//!
//! Every server→client request is answered at once by the pump thread:
//! approvals with `decline`, anything else with a JSON-RPC error. Nothing is
//! ever approved.

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use super::{Agent, Backend, Entry, Log, Scenario, Side, Spawned, make_project, prompts};

pub struct Codex;

pub const MODEL: &str = "gpt-6-luna";
pub const EFFORT: &str = "low";
const VIA: &str = "ws";

fn approval_prompt() -> String {
    format!(
        "run: {}\nRun exactly that shell command once. Request escalated sandbox permissions \
         for it (sandbox_permissions: require_escalated, justification: \"agend recorder\"). \
         Then reply DONE.",
        prompts::APPROVAL_COMMAND
    )
}

const STEER: &str = "Also end your reply with the word STEERED.";

impl Backend for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn program(&self) -> &'static str {
        "codex"
    }

    fn fake(&self) -> &'static str {
        crate::fake_agent::CODEX_BIN
    }

    fn scenarios(&self) -> &'static [Scenario] {
        Scenario::ALL
    }

    fn run(&self, scenario: Scenario, agent: &Agent, dir: &Path, log: &Log) -> Result<(), String> {
        let project = make_project(dir)?;
        let socket = dir.join("codex.sock");
        let t = agent.pace.timeout;
        let mut server = spawn(agent, &project, &socket)?;
        let mut ws = Ws::connect(&socket, log)?;
        let thread = ws.start_thread(&project)?;
        match scenario {
            Scenario::OneTurn => {
                let turn = ws.turn(&thread, prompts::OK)?;
                ws.wait_completed(&turn, t)?;
            }
            Scenario::Interrupt => {
                let turn = ws.turn(&thread, prompts::LONG)?;
                std::thread::sleep(agent.pace.settle);
                ws.request(
                    "turn/interrupt",
                    json!({"threadId": thread, "turnId": turn}),
                )?;
                ws.wait_completed(&turn, t)?;
            }
            Scenario::Approval => {
                let start = log.len();
                let turn = ws.turn(&thread, &approval_prompt())?;
                ws.wait_completed(&turn, t)?;
                log.wait(start, Duration::ZERO, "an approval request", |e| {
                    e.from == Side::Backend
                        && e.msg.get("id").is_some()
                        && e.str("method")
                            .is_some_and(|m| m.ends_with("requestApproval"))
                })?;
            }
            Scenario::Busy => {
                let turn = ws.turn(&thread, prompts::LONG)?;
                std::thread::sleep(agent.pace.settle);
                ws.request(
                    "turn/steer",
                    json!({"threadId": thread, "expectedTurnId": turn, "input": input(STEER)}),
                )?;
                let start = log.len();
                ws.request(
                    "thread/queue/add",
                    json!({"threadId": thread, "clientUserMessageId": "agend-rec-queued-1", "input": input(prompts::OK)}),
                )?;
                ws.wait_completed(&turn, t)?;
                let (_, started) = log.wait(start, t, "the queued turn to start", |e| {
                    e.from == Side::Backend && e.str("method") == Some("turn/started")
                })?;
                let queued = started.msg["params"]["turn"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                ws.wait_completed(&queued, t)?;
            }
            Scenario::Resume => {
                let turn = ws.turn(&thread, prompts::OK)?;
                ws.wait_completed(&turn, t)?;
                ws.close();
                server.stop();
                server = spawn(agent, &project, &socket)?;
                ws = Ws::connect(&socket, log)?;
                ws.request("thread/resume", json!({"threadId": thread}))?;
                let turn = ws.turn(&thread, prompts::OK)?;
                ws.wait_completed(&turn, t)?;
            }
        }
        // Let late notifications (token usage, status) arrive before closing.
        log.wait_quiet(agent.pace.quiet, agent.pace.timeout, |e| {
            e.via == VIA && e.from == Side::Backend
        });
        ws.close();
        server.stop();
        Ok(())
    }
}

fn input(text: &str) -> Value {
    json!([{"type": "text", "text": text, "text_elements": []}])
}

fn spawn(agent: &Agent, project: &Path, socket: &Path) -> Result<Spawned, String> {
    let mut args: Vec<String> = vec!["app-server".into()];
    for (key, value) in [
        ("model", format!("{MODEL:?}")),
        ("model_reasoning_effort", format!("{EFFORT:?}")),
        ("notify", "[]".to_owned()),
        ("mcp_servers", "{}".to_owned()),
    ] {
        args.push("-c".into());
        args.push(format!("{key}={value}"));
    }
    args.extend(["--disable".into(), "hooks".into()]);
    args.extend(["--listen".into(), format!("unix://{}", socket.display())]);
    args.extend(agent.fake_args());
    let child = Command::new(&agent.program)
        .args(&args)
        .current_dir(project)
        .envs(agent.fake_env(project))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", agent.program.display()))?;
    let spawned = Spawned {
        child,
        name: "codex app-server".into(),
    };
    crate::fake_agent::codex::wait_until_listening(socket, Duration::from_secs(30))?;
    Ok(spawned)
}

/// A recording WebSocket JSON-RPC client. A pump thread owns the socket:
/// it sends queued messages, logs everything, and answers server requests.
struct Ws {
    tx: Sender<Option<Value>>,
    log: Log,
    next_id: i64,
    pump: Option<JoinHandle<()>>,
}

impl Ws {
    fn connect(socket: &Path, log: &Log) -> Result<Ws, String> {
        let real = std::fs::canonicalize(socket).map_err(|e| format!("realpath: {e}"))?;
        let stream = UnixStream::connect(&real).map_err(|e| format!("connect: {e}"))?;
        let (ws, _) = tungstenite::client::client("ws://localhost/", stream)
            .map_err(|e| format!("websocket handshake: {e}"))?;
        ws.get_ref()
            .set_read_timeout(Some(Duration::from_millis(10)))
            .map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        let pump_log = log.clone();
        let pump = std::thread::spawn(move || pump(ws, &rx, &pump_log));
        let mut client = Ws {
            tx,
            log: log.clone(),
            next_id: 0,
            pump: Some(pump),
        };
        client.request(
            "initialize",
            json!({"clientInfo": {"name": "agend-record", "title": null, "version": "0.0.0"},
                   "capabilities": {"experimentalApi": true}}),
        )?;
        client.send(json!({"method": "initialized"}));
        Ok(client)
    }

    fn send(&self, message: Value) {
        let _ = self.tx.send(Some(message));
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let start = self.log.len();
        self.send(json!({"id": id, "method": method, "params": params}));
        let (_, reply) = self.log.wait(start, Duration::from_secs(60), method, |e| {
            e.via == VIA
                && e.from == Side::Backend
                && e.msg["id"] == id
                && e.msg.get("method").is_none()
        })?;
        match reply.msg.get("error") {
            Some(error) => Err(format!("{method}: {error}")),
            None => Ok(reply.msg["result"].clone()),
        }
    }

    fn start_thread(&mut self, project: &Path) -> Result<String, String> {
        let result = self.request(
            "thread/start",
            json!({"model": MODEL, "cwd": project, "approvalPolicy": "untrusted",
                   "sandbox": "read-only", "config": {"model_reasoning_effort": EFFORT}}),
        )?;
        id_at(&result, &["thread", "id"])
    }

    fn turn(&mut self, thread: &str, text: &str) -> Result<String, String> {
        let result = self.request(
            "turn/start",
            json!({"threadId": thread, "input": input(text), "effort": EFFORT}),
        )?;
        id_at(&result, &["turn", "id"])
    }

    fn wait_completed(&self, turn: &str, timeout: Duration) -> Result<Entry, String> {
        self.log
            .wait(0, timeout, &format!("turn/completed for {turn}"), |e| {
                e.via == VIA
                    && e.str("method") == Some("turn/completed")
                    && e.msg["params"]["turn"]["id"] == turn
            })
            .map(|(_, e)| e)
    }

    fn close(&mut self) {
        let _ = self.tx.send(None);
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
    }
}

impl Drop for Ws {
    fn drop(&mut self) {
        self.close();
    }
}

fn id_at(value: &Value, path: &[&str]) -> Result<String, String> {
    let mut v = value;
    for key in path {
        v = &v[*key];
    }
    v.as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("no {} in {value}", path.join(".")))
}

/// The answer to a server→client request: never an approval.
fn answer(request: &Value) -> Value {
    let id = request["id"].clone();
    let method = request["method"].as_str().unwrap_or_default();
    if method.ends_with("requestApproval") {
        json!({"id": id, "result": {"decision": "decline"}})
    } else {
        json!({"id": id, "error": {"code": -32601, "message": "agend-record does not handle this request"}})
    }
}

fn pump(mut ws: WebSocket<UnixStream>, rx: &Receiver<Option<Value>>, log: &Log) {
    let send = |ws: &mut WebSocket<UnixStream>, message: Value| {
        log.push(Side::Client, VIA, message.clone());
        ws.send(Message::text(message.to_string())).is_ok()
    };
    loop {
        loop {
            match rx.try_recv() {
                Ok(Some(message)) => {
                    if !send(&mut ws, message) {
                        return;
                    }
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = ws.close(None);
                    let _ = ws.flush();
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                let Ok(message) = serde_json::from_str::<Value>(text.as_str()) else {
                    log.push(Side::Backend, VIA, Value::String(text.as_str().to_owned()));
                    continue;
                };
                log.push(Side::Backend, VIA, message.clone());
                if message.get("id").is_some() && message.get("method").is_some() {
                    let reply = answer(&message);
                    if !send(&mut ws, reply) {
                        return;
                    }
                }
            }
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
    }
}
