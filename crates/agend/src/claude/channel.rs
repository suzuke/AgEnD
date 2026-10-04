//! MCP stdio channel, with explicit durable ACKs and no content replay.
use super::{exchange, request, spool::Spool};
use agend_core::protocol::client::*;
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    io::{self, BufRead},
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};

const WITHIN: Duration = Duration::from_secs(10);
enum Input {
    Message(Value),
    InvalidJson,
    Failure(io::Error),
}
fn send(value: Value) -> io::Result<()> {
    super::json_output(&value, Instant::now() + WITHIN)
}
fn result(id: Value, value: Value) -> io::Result<()> {
    send(json!({"jsonrpc":"2.0","id":id,"result":value}))
}
fn error(id: Value, code: i32, text: &str) -> io::Result<()> {
    send(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":text}}))
}
fn tools() -> Value {
    json!({"tools":[{"name":"agend_ack","description":"Confirm receipt before working. Offline ACKs are saved pending sync; this is not task completion.","inputSchema":{"type":"object","properties":{"receipts":{"type":"array","minItems":1,"maxItems":32,"items":{"type":"object","properties":{"message_id":{"type":"string"},"delivery_id":{"type":"string"},"session_id":{"type":"string"}},"required":["message_id","delivery_id","session_id"],"additionalProperties":false}}},"required":["receipts"],"additionalProperties":false}}]})
}
fn ack(home: &Path, instance: &str, args: &Value) -> io::Result<bool> {
    if !args
        .as_object()
        .is_some_and(|o| o.len() == 1 && o.contains_key("receipts"))
        || !args
            .get("receipts")
            .and_then(Value::as_array)
            .is_some_and(|rs| {
                rs.iter().all(|r| {
                    r.as_object().is_some_and(|o| {
                        o.len() == 3
                            && ["message_id", "delivery_id", "session_id"]
                                .iter()
                                .all(|k| o.contains_key(*k))
                    })
                })
            })
    {
        return Err(io::Error::other("ACK arguments must match the tool schema"));
    }
    let receipts: Vec<ClaudeReceipt> = serde_json::from_value(
        args.get("receipts")
            .cloned()
            .ok_or_else(|| io::Error::other("missing receipts"))?,
    )?;
    if receipts.is_empty()
        || receipts.len() > CLAUDE_BATCH_COUNT
        || receipts.iter().any(|r| {
            r.message_id.is_empty()
                || r.message_id.len() > MAX_MESSAGE_BYTES
                || !is_uuid_v4(&r.delivery_id)
                || !is_uuid_v4(&r.session_id)
        })
    {
        return Err(io::Error::other(
            "ACK requires 1..32 nonempty message ids and UUID v4 delivery/session ids",
        ));
    }
    let deadline = Instant::now() + WITHIN;
    let spool = Spool::open(home, "acks", deadline)?;
    // Persist the entire accepted batch before any network attempt. A crash
    // during publication reports no success and every published ACK remains.
    for receipt in receipts {
        spool.publish(request(
            instance,
            ClaudeOperation::Ack {
                receipts: vec![receipt],
            },
        )?)?;
    }
    if let Err(e) = super::replay(home, "acks", deadline) {
        if e.kind() == io::ErrorKind::InvalidInput {
            return Err(e);
        }
        eprintln!("agend channel: ACK saved, pending sync: {e}");
        return Ok(true);
    }
    spool.locked(|| Ok(!spool.list()?.is_empty()))
}

pub(super) fn run(home: &Path, args: &[OsString]) -> io::Result<()> {
    let [flag, name] = args else {
        return Err(io::Error::other("usage: agend channel --instance <id>"));
    };
    let instance = super::caller()?;
    if flag != "--instance" || name.to_str() != Some(instance.as_str()) {
        return Err(io::Error::other("--instance must match AGEND_INSTANCE"));
    }
    // Stdin is independent of daemon polling. Bounded messages cannot grow an
    // unbounded mailbox while a peer or local disk is slow.
    let (tx, rx) = mpsc::sync_channel(4);
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        loop {
            let mut bytes = vec![];
            let line = loop {
                let buf = match stdin.fill_buf() {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx.send(Input::Failure(e));
                        return;
                    }
                };
                if buf.is_empty() {
                    if !bytes.is_empty() {
                        let _ = tx.send(Input::Failure(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "partial MCP line",
                        )));
                    }
                    return;
                }
                let end = buf.iter().position(|b| *b == b'\n');
                let n = end.map_or(buf.len(), |i| i + 1);
                if bytes.len() + n > MAX_MESSAGE_BYTES {
                    let _ = tx.send(Input::Failure(io::Error::other("MCP input exceeds 1 MiB")));
                    return;
                }
                bytes.extend_from_slice(&buf[..n]);
                stdin.consume(n);
                if end.is_some() {
                    break bytes;
                }
            };
            let input = match serde_json::from_slice::<Value>(&line) {
                Ok(v) => Input::Message(v),
                Err(_) => Input::InvalidJson,
            };
            if tx.send(input).is_err() {
                return;
            }
        }
    });
    let mut initialized = false;
    let mut ready = false;
    let mut session: Option<String> = None;
    let mut next_poll = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Input::Message(input)) => {
                let id = input.get("id").cloned();
                let method = input.get("method").and_then(Value::as_str).unwrap_or("");
                if !input.is_object()
                    || input.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
                    || method.is_empty()
                    || id
                        .as_ref()
                        .is_some_and(|v| !v.is_string() && !v.is_i64() && !v.is_u64())
                {
                    error(Value::Null, -32600, "invalid JSON-RPC request")?;
                    continue;
                }
                match (method, id) {
                    ("initialize", Some(id)) if !initialized => {
                        let requested = input
                            .pointer("/params/protocolVersion")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let version = match requested {
                            "2025-11-25" | "2025-06-18" | "2025-03-26" | "2024-11-05" => requested,
                            _ => "2025-11-25",
                        };
                        result(
                            id,
                            json!({"protocolVersion":version,"capabilities":{"experimental":{"claude/channel":{}},"tools":{}},"serverInfo":{"name":"agend","version":env!("CARGO_PKG_VERSION")},"instructions":"AgEnD pushes full messages. Call agend_ack using message_id, delivery_id and session_id before working. Pending sync means saved locally, not confirmed; ACK is not task completion."}),
                        )?;
                        initialized = true;
                    }
                    ("notifications/initialized", None) if initialized => ready = true,
                    ("ping", Some(id)) => result(id, json!({}))?,
                    ("tools/list", Some(id)) if ready => result(id, tools())?,
                    ("tools/call", Some(id)) if ready => {
                        if input.pointer("/params/name").and_then(Value::as_str)
                            != Some("agend_ack")
                        {
                            error(id, -32602, "unknown tool")?;
                            continue;
                        }
                        let response = match ack(
                            home,
                            &instance,
                            input.pointer("/params/arguments").unwrap_or(&Value::Null),
                        ) {
                            Ok(true) => {
                                json!({"content":[{"type":"text","text":"ACK saved locally; pending sync. Not yet confirmed by daemon."}]})
                            }
                            Ok(false) => {
                                json!({"content":[{"type":"text","text":"ACK committed by daemon; confirmed."}]})
                            }
                            Err(e) => {
                                json!({"isError":true,"content":[{"type":"text","text":format!("ACK failed: {e}; no confirmation is claimed")}]})
                            }
                        };
                        result(id, response)?;
                    }
                    (_, Some(id)) => error(id, -32601, "method not available")?,
                    _ => {}
                }
            }
            Ok(Input::InvalidJson) => error(Value::Null, -32700, "invalid JSON")?,
            Ok(Input::Failure(e)) => return Err(e),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !ready || Instant::now() < next_poll {
            continue;
        }
        next_poll = Instant::now() + Duration::from_millis(250);
        // Separate deadlines bound background receipts; they never trigger
        // content dispatch and historic hooks never establish idle.
        for kind in ["hooks", "acks"] {
            if let Err(e) = super::replay(home, kind, Instant::now() + Duration::from_millis(200)) {
                eprintln!("agend channel: {kind} pending sync: {e}");
            }
        }
        let operation = match &session {
            Some(s) => ClaudeOperation::Poll {
                session_id: s.clone(),
            },
            None => ClaudeOperation::Attach,
        };
        let data = request(&instance, operation)?;
        let reply = match exchange(home, &data, Instant::now() + Duration::from_millis(2200)) {
            Ok(reply) => reply,
            Err(e) => {
                eprintln!("agend channel: daemon unavailable or refused: {e}");
                continue;
            }
        };
        if session.is_none() {
            session = reply.session_id;
        }
        for message in reply.messages {
            send(
                json!({"jsonrpc":"2.0","method":"notifications/claude/channel","params":{"content":message.content,"meta":{"message_id":message.receipt.message_id,"delivery_id":message.receipt.delivery_id,"session_id":message.receipt.session_id}}}),
            )?;
            let written = request(
                &instance,
                ClaudeOperation::Written {
                    receipts: vec![message.receipt],
                },
            )?;
            if let Err(e) = exchange(home, &written, Instant::now() + WITHIN) {
                eprintln!("agend channel: write receipt not saved: {e}");
            }
        }
    }
}
