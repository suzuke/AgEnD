//! Persist first; offline Stop is an empty decision, never a queue claim.
use super::{exchange, request, spool::Spool};
use agend_core::protocol::client::*;
use serde_json::json;
use std::{
    ffi::OsString,
    io::{self},
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) fn run(home: &Path, args: &[OsString]) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let [event] = args else {
        return Err(io::Error::other("usage: agend hook <event>"));
    };
    let event = event
        .to_str()
        .ok_or_else(|| io::Error::other("invalid hook name"))?;
    if !matches!(
        event,
        "SessionStart" | "UserPromptSubmit" | "Stop" | "PreToolUse" | "PostToolUse" | "SessionEnd"
    ) {
        return Err(io::Error::other("unsupported hook event"));
    }
    let instance = super::caller()?;
    // Command hooks receive exactly one JSON input then EOF.
    let mut bytes = vec![];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "hook stdin timed out",
            ));
        }
        let mut poll = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut poll, 1, left.as_millis().max(1) as i32) };
        if result < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if result == 0 {
            continue;
        }
        let mut buf = [0u8; 4096];
        let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n as usize]);
        if bytes.len() > MAX_MESSAGE_BYTES {
            break;
        }
    }
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::other("hook input exceeds 1 MiB"));
    }
    let native: serde_json::Value = serde_json::from_slice(&bytes)?;
    let session = native
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|v| is_uuid_v4(v))
        .ok_or_else(|| io::Error::other("hook needs a UUID v4 session_id"))?;
    if native.get("hook_event_name").and_then(|v| v.as_str()) != Some(event) {
        return Err(io::Error::other("hook event name does not match stdin"));
    }
    let data = request(
        &instance,
        ClaudeOperation::Hook {
            event_id: super::uuid()?,
            session_id: session.into(),
            event: event.into(),
            payload: serde_json::to_string(&native)?,
            occurred_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_millis() as u64,
            replayed: false,
        },
    )?;
    let spool = Spool::open(home, "hooks", deadline)?;
    let mut messages = vec![];
    let received: io::Result<ClaudeReplyData> = spool.locked(|| {
        // No unlock between durable publication and the first live RPC.
        // Publication failure propagates; only transport failure is offline.
        let path = spool.publish_locked(data.clone())?;
        Ok((|| {
            let reply = exchange(home, &data, deadline - Duration::from_millis(200))?;
            if reply.committed {
                spool.remove(&path)?;
            }
            Ok(reply)
        })())
    })?;
    match received {
        Ok(reply) if reply.committed => {
            messages = reply.messages;
        }
        Ok(_) => eprintln!("agend hook: event remains pending; no commit receipt"),
        Err(e) => eprintln!("agend hook: event saved, pending sync: {e}"),
    }
    let output = if event == "Stop" && !messages.is_empty() {
        let mut reason = "AgEnD messages. Call agend_ack with each message_id, delivery_id and session_id before working.\n".to_string();
        for m in &messages {
            reason.push_str(&format!(
                "\nmessage_id={} delivery_id={} session_id={}\n{}\n",
                m.receipt.message_id, m.receipt.delivery_id, m.receipt.session_id, m.content
            ));
        }
        json!({"decision":"block", "reason":reason})
    } else {
        json!({})
    };
    super::json_output(&output, deadline)?;
    // Lost written receipt is conservatively unknown, never replayed as content.
    if !messages.is_empty() {
        let written = request(
            &instance,
            ClaudeOperation::Written {
                receipts: messages.into_iter().map(|m| m.receipt).collect(),
            },
        )?;
        if let Err(e) = exchange(home, &written, deadline) {
            eprintln!("agend hook: write receipt not saved: {e}");
        }
    }
    Ok(())
}
