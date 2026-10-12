//! Never reconnect between checking a launch receipt and sending Shutdown.
//! Process quiescence and serialization of replacement starts belong to the
//! caller; this module establishes identity and waits for holder absence.
use super::{Conn, RuntimeError, STOP_WITHIN, err, files};
use agend_core::{
    protocol::holder::{HolderRequest, HolderResponse, V1_3},
    runtime_records::ManagedLaunchIntent,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(super) fn check_holder(home: &Path, id: &str, expected: u32) -> Result<bool, RuntimeError> {
    match files::running(home, id).map_err(|e| err(e.to_string()))? {
        None => Ok(false),
        Some(pid) if pid == expected => Ok(true),
        Some(_) => Err(err("managed holder changed; replacement preserved")),
    }
}

pub(super) fn stop(
    home: &Path,
    intent: &ManagedLaunchIntent,
    holder_pid: u32,
    agent_pid: u32,
) -> Result<(), RuntimeError> {
    let id = &intent.instance_id;
    if !check_holder(home, id, holder_pid)? {
        return Ok(());
    }
    let mut conn = verified_connection(home, intent, holder_pid, agent_pid)?;
    if !check_holder(home, id, holder_pid)? {
        return Ok(());
    }
    conn.send(&HolderRequest::Shutdown)
        .map_err(|e| err(format!("managed Shutdown outcome unknown: {e}")))?;
    let deadline = Instant::now() + STOP_WITHIN;
    while check_holder(home, id, holder_pid)? {
        if Instant::now() >= deadline {
            return Err(err(
                "managed holder still runs after Shutdown; outcome unknown",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// Establish identity on one socket without stopping or replacing a runtime link.
pub(super) fn verified_connection(
    home: &Path,
    intent: &ManagedLaunchIntent,
    holder_pid: u32,
    agent_pid: u32,
) -> Result<Conn, RuntimeError> {
    let id = &intent.instance_id;
    if !check_holder(home, id, holder_pid)? {
        return Err(err("managed holder is absent"));
    }
    // Unlike the generic stop path, do not retry a failed connection: a later
    // peer may be a different holder. The caller reconciles durable state.
    let (mut conn, _) = Conn::connect(&files::socket_path(home, id))
        .map_err(|e| err(format!("managed stop connect: {e}")))?;
    if conn.version < V1_3 {
        return Err(err("managed stop requires holder protocol 1.3"));
    }
    conn.stream()
        .and_then(|stream| stream.set_write_timeout(Some(Duration::from_secs(2))))
        .map_err(|e| err(e.to_string()))?;
    conn.send(&HolderRequest::GetLaunchBinding {
        instance_id: id.clone(),
    })
    .map_err(|e| err(format!("managed stop binding query: {e}")))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(err(
                "managed stop binding query timed out; holder preserved",
            ));
        }
        match conn
            .recv_within(remaining)
            .map_err(|e| err(e.to_string()))?
        {
            HolderResponse::LaunchBinding { data } => {
                if data.instance_id != *id
                    || data.binding.as_deref() != Some(intent.binding.as_str())
                    || data.process_id != Some(agent_pid)
                {
                    return Err(err(
                        "managed stop launch identity differs; holder preserved",
                    ));
                }
                break;
            }
            HolderResponse::Error { data } => {
                return Err(err(format!("managed stop binding refused: {}", data.code)));
            }
            _ => {}
        }
    }
    if !check_holder(home, id, holder_pid)? {
        return Err(err("managed holder disappeared"));
    }
    Ok(conn)
}
