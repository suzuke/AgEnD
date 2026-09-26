//! Recorder backend for `opencode serve`: HTTP on `127.0.0.1:<port>` plus
//! the `/event` server-sent event stream, the transport
//! `fake-opencode-serve` emulates.
//!
//! Run settings (real CLI; env for this run only, nothing written to the
//! user's config): `--pure` (no external plugins), the free model
//! `opencode/space-bunny-free` (also as the small model for titles),
//! `permission: {bash: "ask", edit: "ask"}`, autoupdate off. Permission
//! requests are always answered `reject`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

use super::{Agent, Backend, Log, Scenario, Side, Spawned, make_project, prompts, wait_for_line};

pub struct Opencode;

pub const PROVIDER: &str = "opencode";
pub const MODEL: &str = "space-bunny-free";

fn approval_prompt() -> String {
    format!(
        "run: {}\nUse your bash tool to run exactly that command once, then reply DONE.",
        prompts::APPROVAL_COMMAND
    )
}

impl Backend for Opencode {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn program(&self) -> &'static str {
        "opencode"
    }

    fn fake(&self) -> &'static str {
        crate::fake_agent::OPENCODE_BIN
    }

    fn scenarios(&self) -> &'static [Scenario] {
        Scenario::ALL
    }

    fn run(&self, scenario: Scenario, agent: &Agent, dir: &Path, log: &Log) -> Result<(), String> {
        let project = make_project(dir)?;
        let t = agent.pace.timeout;
        let (mut server, port) = spawn(agent, &project)?;
        let oc = Client { port, log };
        oc.subscribe()?;
        let session = oc.new_session()?;
        match scenario {
            Scenario::OneTurn => {
                let start = log.len();
                oc.prompt(&session, prompts::OK)?;
                oc.wait_idle(&session, start, t)?;
                oc.call("GET", &format!("/session/{session}/message"), None)?;
            }
            Scenario::Interrupt => {
                let start = log.len();
                oc.prompt(&session, prompts::LONG)?;
                oc.wait_busy(&session, start, t)?;
                std::thread::sleep(agent.pace.settle);
                let start = log.len();
                oc.call(
                    "POST",
                    &format!("/session/{session}/abort"),
                    Some(json!({})),
                )?;
                oc.wait_idle(&session, start, t)?;
                oc.call("GET", &format!("/session/{session}/message"), None)?;
            }
            Scenario::Approval => {
                let start = log.len();
                oc.prompt(&session, &approval_prompt())?;
                // The SSE event can be missed (spike O5), so poll too.
                let deadline = std::time::Instant::now() + t;
                let asked =
                    |e: &super::Entry| e.via == "sse" && e.str("type") == Some("permission.asked");
                let id = loop {
                    let seen = log.wait(start, Duration::ZERO, "permission.asked", asked);
                    let (_, pending) = oc.call("GET", "/permission", None)?;
                    if let Some(id) = pending[0]["id"].as_str() {
                        break id.to_owned();
                    }
                    if let Ok((_, e)) = seen
                        && let Some(id) = e.msg["properties"]["id"].as_str()
                    {
                        break id.to_owned();
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err("no permission request".into());
                    }
                    std::thread::sleep(agent.pace.settle / 4);
                };
                let start = log.len();
                oc.call(
                    "POST",
                    &format!("/session/{session}/permissions/{id}"),
                    Some(json!({"response": "reject"})),
                )?;
                oc.wait_idle(&session, start, t)?;
                oc.call("GET", &format!("/session/{session}/message"), None)?;
            }
            Scenario::Busy => {
                let start = log.len();
                oc.prompt(&session, prompts::LONG)?;
                oc.wait_busy(&session, start, t)?;
                std::thread::sleep(agent.pace.settle);
                oc.call("GET", "/session/status", None)?;
                let start = log.len();
                oc.prompt(&session, prompts::OK)?;
                // Idle once both turns ran (the queued prompt may start
                // after an idle event; wait until idle stays idle).
                let mut from = start;
                loop {
                    let (i, _) = oc.wait_idle(&session, from, t)?;
                    match log.wait(i + 1, agent.pace.quiet * 2, "busy", |e| {
                        is_status(e, &session, "busy")
                    }) {
                        Ok((j, _)) => from = j + 1,
                        Err(_) => break,
                    }
                }
                oc.call("GET", &format!("/session/{session}/message"), None)?;
            }
            Scenario::Resume => {
                let start = log.len();
                oc.prompt(&session, prompts::OK)?;
                oc.wait_idle(&session, start, t)?;
                server.stop();
                let (restarted, port) = spawn(agent, &project)?;
                server = restarted;
                let oc = Client { port, log };
                oc.call("GET", &format!("/session/{session}"), None)?;
                oc.call("GET", &format!("/session/{session}/message"), None)?;
                oc.subscribe()?;
                let start = log.len();
                oc.prompt(&session, prompts::OK)?;
                oc.wait_idle(&session, start, t)?;
                oc.call("GET", &format!("/session/{session}/message"), None)?;
            }
        }
        log.wait_quiet(agent.pace.quiet, agent.pace.timeout, |e| e.via == "sse");
        server.stop();
        Ok(())
    }
}

fn is_status(e: &super::Entry, session: &str, status: &str) -> bool {
    e.via == "sse"
        && e.str("type") == Some("session.status")
        && e.msg["properties"]["sessionID"] == session
        && e.msg["properties"]["status"]["type"] == status
}

fn spawn(agent: &Agent, project: &Path) -> Result<(Spawned, u16), String> {
    let model = format!("{PROVIDER}/{MODEL}");
    let config = json!({
        "autoupdate": false,
        "model": model,
        "small_model": model,
        "permission": {"bash": "ask", "edit": "ask", "webfetch": "ask"},
    });
    let mut child = Command::new(&agent.program)
        .args(["serve", "--pure", "--hostname", "127.0.0.1", "--port", "0"])
        .args(agent.fake_args())
        .current_dir(project)
        .env("OPENCODE_CONFIG_CONTENT", config.to_string())
        .env("OPENCODE_DISABLE_AUTOUPDATE", "1")
        .envs(agent.fake_env(project))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", agent.program.display()))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let line = wait_for_line(&mut child, stdout, "listening on", Duration::from_secs(60))?;
    let port = line
        .trim()
        .rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| format!("no port in {line:?}"))?;
    let spawned = Spawned {
        child,
        name: "opencode serve".into(),
    };
    Ok((spawned, port))
}

struct Client<'a> {
    port: u16,
    log: &'a Log,
}

impl Client<'_> {
    /// One request; logs `{method, path, body}` and `{status, body}`.
    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, Value), String> {
        self.log.push(
            Side::Client,
            "http",
            json!({"method": method, "path": path, "body": body}),
        );
        let (status, text) = request(self.port, method, path, body.as_ref())
            .map_err(|e| format!("{method} {path}: {e}"))?;
        let body = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        self.log.push(
            Side::Backend,
            "http",
            json!({"status": status, "body": body}),
        );
        if status >= 400 {
            return Err(format!("{method} {path}: HTTP {status} {body}"));
        }
        Ok((status, body))
    }

    fn new_session(&self) -> Result<String, String> {
        let (_, body) = self.call("POST", "/session", Some(json!({})))?;
        body["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("POST /session gave {body}"))
    }

    fn prompt(&self, session: &str, text: &str) -> Result<(), String> {
        let body = json!({
            "parts": [{"type": "text", "text": text}],
            "model": {"providerID": PROVIDER, "modelID": MODEL},
        });
        self.call(
            "POST",
            &format!("/session/{session}/prompt_async"),
            Some(body),
        )
        .map(|_| ())
    }

    fn wait_idle(
        &self,
        session: &str,
        start: usize,
        t: Duration,
    ) -> Result<(usize, super::Entry), String> {
        self.log.wait(start, t, "session.idle", |e| {
            e.via == "sse"
                && e.str("type") == Some("session.idle")
                && e.msg["properties"]["sessionID"] == session
        })
    }

    fn wait_busy(
        &self,
        session: &str,
        start: usize,
        t: Duration,
    ) -> Result<(usize, super::Entry), String> {
        self.log.wait(start, t, "session.status busy", |e| {
            is_status(e, session, "busy")
        })
    }

    /// Opens `GET /event`; a thread logs every event until the server goes.
    fn subscribe(&self) -> Result<(), String> {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).map_err(|e| e.to_string())?;
        write!(
            stream,
            "GET /event HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept: text/event-stream\r\n\r\n",
            self.port
        )
        .map_err(|e| e.to_string())?;
        let start = self.log.len();
        self.log.push(
            Side::Client,
            "http",
            json!({"method": "GET", "path": "/event", "body": null}),
        );
        let mut reader = BufReader::new(stream);
        let (status, chunked, _) = read_head(&mut reader).map_err(|e| e.to_string())?;
        self.log.push(
            Side::Backend,
            "http",
            json!({"status": status, "body": null}),
        );
        let log = self.log.clone();
        std::thread::spawn(move || {
            let body: Box<dyn Read + Send> = if chunked {
                Box::new(Chunked::new(reader))
            } else {
                Box::new(reader)
            };
            let mut data = String::new();
            for line in BufReader::new(body).lines().map_while(Result::ok) {
                if let Some(rest) = line.strip_prefix("data:") {
                    data.push_str(rest.trim_start());
                } else if line.is_empty() && !data.is_empty() {
                    let event = serde_json::from_str(&data).unwrap_or(Value::String(data.clone()));
                    log.push(Side::Backend, "sse", event);
                    data.clear();
                }
            }
        });
        self.log
            .wait(start, Duration::from_secs(10), "server.connected", |e| {
                e.via == "sse" && e.str("type") == Some("server.connected")
            })
            .map(|_| ())
    }
}

/// Sends one `Connection: close` request; returns status and body text.
fn request(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> std::io::Result<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(120)))?;
    let body = body.map(Value::to_string).unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut reader = BufReader::new(stream);
    let (status, chunked, length) = read_head(&mut reader)?;
    let mut bytes = Vec::new();
    if chunked {
        Chunked::new(reader).read_to_end(&mut bytes)?;
    } else if let Some(n) = length {
        bytes.resize(n, 0);
        reader.read_exact(&mut bytes)?;
    } else {
        reader.read_to_end(&mut bytes)?;
    }
    Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
}

/// Status line and headers: `(status, chunked, content-length)`.
fn read_head(reader: &mut impl BufRead) -> std::io::Result<(u16, bool, Option<usize>)> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad status line"))?;
    let (mut chunked, mut length) = (false, None);
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
            if name == "transfer-encoding" && value.eq_ignore_ascii_case("chunked") {
                chunked = true;
            } else if name == "content-length" {
                length = value.parse().ok();
            }
        }
    }
    Ok((status, chunked, length))
}

/// Decodes a chunked transfer-encoded body.
struct Chunked<R> {
    inner: R,
    left: usize,
    done: bool,
}

impl<R: BufRead> Chunked<R> {
    fn new(inner: R) -> Self {
        Chunked {
            inner,
            left: 0,
            done: false,
        }
    }
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let mut size = String::new();
            if self.inner.read_line(&mut size)? == 0 {
                self.done = true;
                return Ok(0);
            }
            if size.trim().is_empty() {
                // The CRLF after the previous chunk.
                size.clear();
                if self.inner.read_line(&mut size)? == 0 {
                    self.done = true;
                    return Ok(0);
                }
            }
            let hex = size.trim().split(';').next().unwrap_or("0");
            self.left = usize::from_str_radix(hex, 16)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            if self.left == 0 {
                self.done = true;
                return Ok(0);
            }
        }
        let n = buf.len().min(self.left);
        let n = self.inner.read(&mut buf[..n])?;
        if n == 0 {
            self.done = true;
        }
        self.left -= n;
        Ok(n)
    }
}
