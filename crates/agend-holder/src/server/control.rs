//! Correlated PTY operations, serialized with all existing PTY writes.
//! The daemon checks caller identity; here the live connection, process
//! generation, owner and size are checked again at the actual queue operation.

use super::*;
use agend_core::protocol::terminal::{
    TerminalControlData, TerminalControlOperation, TerminalControlRequest, TerminalSize,
    TerminalViewport,
};

fn failed(request_id: &str, code: &str, message: impl Into<String>) -> HolderResponse {
    HolderResponse::TerminalOperationError {
        data: TerminalOperationError {
            request_id: request_id.into(),
            code: code.into(),
            message: message.into(),
        },
    }
}

fn validate(
    state: &State,
    conn_id: u64,
    request: &TerminalControlRequest,
) -> Result<(), (&'static str, &'static str)> {
    if state.conn.as_ref().is_none_or(|conn| conn.id != conn_id) {
        return Err((
            "stale_terminal",
            "the holder connection changed; acquire control again",
        ));
    }
    if state.stop.is_some() || state.agent.is_none() || state.exited.is_some() {
        return Err(("agent_exited", "there is no live agent PTY"));
    }
    if request.generation != state.screen.generation() {
        return Err((
            "stale_terminal",
            "the holder generation changed; acquire control again",
        ));
    }
    if let TerminalControlOperation::DaemonKey {
        key,
        expected_revision,
    } = &request.operation
    {
        if state.control.is_some() {
            return Err((
                "control_lost",
                "an operator owns this terminal; daemon key refused",
            ));
        }
        if *expected_revision != state.screen.revision() {
            return Err(("stale_screen", "the screen changed; daemon key refused"));
        }
        if pty::control_key_bytes(*key).is_none() {
            return Err(("unknown_control_key", "unknown daemon control key"));
        }
        return Ok(());
    }
    let attach = request.operation.attach_id();
    if attach.is_empty() || attach.len() > 128 {
        return Err(("invalid_request", "attach id must have 1 to 128 bytes"));
    }
    if !matches!(request.operation, TerminalControlOperation::Acquire { .. })
        && state.control.as_deref() != Some(attach)
    {
        return Err((
            "control_lost",
            "another window controls this terminal; acquire control again",
        ));
    }
    match &request.operation {
        TerminalControlOperation::Acquire { size, .. }
        | TerminalControlOperation::Resize { size, .. } => {
            if !size.is_valid() {
                return Err(("invalid_size", "rows and columns must be 1 to 1000"));
            }
            if Screen::validate_frame_size(*size).is_err() {
                return Err((
                    "frame_too_large",
                    "this PTY size cannot produce a frame within 8 MiB",
                ));
            }
        }
        _ => (),
    }
    Ok(())
}

pub(super) fn enqueue(
    holder: &Arc<Holder>,
    state: &mut State,
    request: TerminalControlRequest,
) -> Option<HolderResponse> {
    let conn = state.conn.as_ref()?;
    if conn.version < agend_core::protocol::holder::V1_1 {
        return Some(failed(
            &request.request_id,
            "not_supported",
            "terminal control requires holder protocol 1.1",
        ));
    }
    let conn_id = conn.id;
    if matches!(
        request.operation,
        TerminalControlOperation::DaemonKey { .. }
    ) && conn.version < agend_core::protocol::holder::V1_2
    {
        return Some(failed(
            &request.request_id,
            "not_supported",
            "daemon key completion requires holder protocol 1.2",
        ));
    }
    // Resize/Input/Release are validated at execution: an earlier Acquire on
    // the same queue may not yet have completed. Generation/size/live checks
    // are repeated there, and no operation can write before owner validation.
    if let TerminalControlOperation::Acquire { .. } = request.operation
        && let Err((code, message)) = validate(state, conn_id, &request)
    {
        return Some(failed(&request.request_id, code, message));
    }
    if state.stop.is_some() || state.agent.is_none() || state.exited.is_some() {
        return Some(failed(
            &request.request_id,
            "agent_exited",
            "there is no live agent PTY",
        ));
    }
    if request.generation != state.screen.generation() {
        return Some(failed(
            &request.request_id,
            "stale_terminal",
            "the holder generation changed; acquire control again",
        ));
    }
    let bytes = match &request.operation {
        TerminalControlOperation::DaemonKey { key, .. } => {
            pty::control_key_bytes(*key).map(Vec::from)
        }
        TerminalControlOperation::Input { bytes_base64, .. } => match BASE64.decode(bytes_base64) {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                return Some(failed(
                    &request.request_id,
                    "bad_request",
                    format!("bytes_base64: {error}"),
                ));
            }
        },
        _ => None,
    };
    let input = state.agent.as_ref()?.input.clone();
    let id = request.request_id.clone();
    let writer_holder = Arc::clone(holder);
    match input.operation(move |out| execute(&writer_holder, conn_id, request, bytes, out)) {
        Ok(()) => {
            if let Some(conn) = &mut state.conn {
                conn.structured = true;
            }
            None
        }
        Err(QueueError::Busy) => Some(failed(
            &id,
            "pty_busy",
            "the PTY queue is full; nothing was written",
        )),
        Err(QueueError::Closed) => Some(failed(
            &id,
            "agent_exited",
            "the PTY writer stopped; nothing was written",
        )),
    }
}

fn resize(state: &mut State, size: TerminalSize) -> Result<(), String> {
    let agent = state.agent.as_ref().ok_or("there is no live agent PTY")?;
    // Actual PTY success comes first. Output parsing waits on this same lock,
    // so it never sees new dimensions in the PTY with old grid dimensions.
    agent
        .master
        .resize(PtySize {
            rows: size.rows,
            cols: size.columns,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| error.to_string())?;
    state.screen.resize(size.rows, size.columns);
    Ok(())
}

fn execute(
    holder: &Arc<Holder>,
    conn_id: u64,
    request: TerminalControlRequest,
    bytes: Option<Vec<u8>>,
    out: &mut dyn Write,
) {
    let mut state = holder.lock();
    if let Err((code, message)) = validate(&state, conn_id, &request) {
        if state.conn.as_ref().is_some_and(|conn| conn.id == conn_id) {
            push(
                holder,
                &mut state,
                &failed(&request.request_id, code, message),
            );
        }
        return;
    }
    let mut reply = TerminalControlData {
        request_id: request.request_id.clone(),
        generation: request.generation.clone(),
        attach_id: Some(request.operation.attach_id().into()),
        frame: None,
    };
    match request.operation {
        TerminalControlOperation::Acquire { attach_id, size }
        | TerminalControlOperation::Resize { attach_id, size } => {
            if let Err(message) = resize(&mut state, size) {
                push(
                    holder,
                    &mut state,
                    &failed(&request.request_id, "resize_failed", message),
                );
                return;
            }
            let frame = match state.screen.frame(TerminalViewport {
                top: None,
                rows: size.rows,
            }) {
                Ok(frame) => frame,
                Err(error) => {
                    state.control = None;
                    push(
                        holder,
                        &mut state,
                        &failed(&request.request_id, error.code(), error.message()),
                    );
                    return;
                }
            };
            reply.frame = Some(frame);
            let response = HolderResponse::TerminalControl { data: reply };
            // A grant needs a complete frame. Oversized combining text must
            // not leave a controller active after an unsuccessful grant.
            if serde_json::to_writer(&mut FrameLine(Vec::new()), &response).is_err() {
                state.control = None;
                push(
                    holder,
                    &mut state,
                    &failed(
                        &request.request_id,
                        "frame_too_large",
                        "terminal frame exceeds 8 MiB; nothing was truncated",
                    ),
                );
                return;
            }
            state.control = Some(attach_id);
            push(holder, &mut state, &response);
        }
        TerminalControlOperation::Input { .. } | TerminalControlOperation::DaemonKey { .. } => {
            if matches!(
                request.operation,
                TerminalControlOperation::DaemonKey { .. }
            ) {
                reply.attach_id = None;
            }
            drop(state);
            // The same queue cannot execute a new grant until this completes.
            let result = out
                .write_all(&bytes.expect("input decoded before enqueue"))
                .and_then(|()| out.flush());
            let mut state = holder.lock();
            if state.conn.as_ref().is_none_or(|conn| conn.id != conn_id) {
                return;
            }
            match result {
                Ok(()) => push(
                    holder,
                    &mut state,
                    &HolderResponse::TerminalControl { data: reply },
                ),
                Err(error) => {
                    state.control = None;
                    push(
                        holder,
                        &mut state,
                        &failed(&request.request_id, "pty_write_failed", error.to_string()),
                    );
                }
            }
        }
        TerminalControlOperation::Release { .. } => {
            state.control = None;
            reply.attach_id = None;
            push(
                holder,
                &mut state,
                &HolderResponse::TerminalControl { data: reply },
            );
        }
    }
}

/// Legacy operator writes also recheck ownership in the one writer queue;
/// otherwise an input accepted behind a pending Acquire could bypass it.
pub(super) fn legacy_input(
    holder: &Arc<Holder>,
    state: &State,
    bytes: Vec<u8>,
) -> Option<HolderResponse> {
    if state.control.is_some() {
        return Some(error(
            "control_lost",
            "legacy input is refused while a terminal controller exists",
        ));
    }
    let Some(agent) = state.agent.as_ref() else {
        return Some(error("not_spawned", "no agent has been spawned"));
    };
    if state.exited.is_some() {
        return Some(error("agent_exited", "the agent has exited"));
    }
    let conn_id = state.conn.as_ref()?.id;
    let writer_holder = Arc::clone(holder);
    match agent.input.operation(move |out| {
        let mut state = writer_holder.lock();
        if state.conn.as_ref().is_none_or(|conn| conn.id != conn_id) {
            return;
        }
        if state.control.is_some()
            || state.exited.is_some()
            || state.stop.is_some()
            || state.agent.is_none()
        {
            let reason = if state.exited.is_some() {
                "agent_exited"
            } else {
                "control_lost"
            };
            push(
                &writer_holder,
                &mut state,
                &error(reason, "legacy input was invalidated; nothing was written"),
            );
            return;
        }
        drop(state);
        if let Err(write_error) = out.write_all(&bytes).and_then(|()| out.flush()) {
            let mut state = writer_holder.lock();
            if state.conn.as_ref().is_some_and(|conn| conn.id == conn_id) {
                push(
                    &writer_holder,
                    &mut state,
                    &error("pty_write_failed", write_error.to_string()),
                );
            }
        }
    }) {
        Ok(()) => None,
        Err(QueueError::Busy) => Some(error(
            "pty_busy",
            "the agent is not reading its input; nothing was written",
        )),
        Err(QueueError::Closed) => Some(error("agent_exited", "the agent's PTY is closed")),
    }
}
