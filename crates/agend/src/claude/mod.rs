//! Synchronous Claude helper paths. Never open SQLite or a Tokio runtime.
mod channel;
mod hook;
mod spool;
use agend_core::protocol::client::*;
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read},
    path::Path,
    time::Instant,
};

pub fn run(args: &[OsString], is_channel: bool) -> i32 {
    let result = (|| {
        let home = crate::home::resolve().map_err(|e| io::Error::other(e.message))?;
        if is_channel {
            channel::run(&home, args)
        } else {
            hook::run(&home, args)
        }
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("agend Claude helper: {e}");
            1
        }
    }
}
fn uuid() -> io::Result<String> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(uuid_v4(bytes))
}
fn caller() -> io::Result<String> {
    let id = std::env::var("AGEND_INSTANCE")
        .map_err(|_| io::Error::other("AGEND_INSTANCE is required"))?;
    agend_core::runtime_records::validate_id(&id).map_err(io::Error::other)?;
    Ok(id)
}
fn request(instance: &str, operation: ClaudeOperation) -> io::Result<ClaudeRequestData> {
    Ok(ClaudeRequestData {
        request_id: uuid()?,
        instance_id: instance.into(),
        operation,
    })
}
fn exchange(
    home: &Path,
    data: &ClaudeRequestData,
    deadline: Instant,
) -> io::Result<ClaudeReplyData> {
    let response = agend_client::exchange_once(
        &home.join(DAEMON_SOCKET),
        Some(data.instance_id.clone()),
        V1_5,
        &ClientRequest::Claude { data: data.clone() },
        deadline,
    )
    .map_err(|e| match e {
        agend_client::ClientError::Daemon { .. } => io::Error::new(io::ErrorKind::InvalidInput, e),
        _ => io::Error::other(e),
    })?;
    match response {
        ClientResponse::Claude { data } => Ok(data),
        _ => Err(io::Error::other("unexpected Claude reply")),
    }
}
fn replay(home: &Path, kind: &str, deadline: Instant) -> io::Result<usize> {
    let spool = spool::Spool::open(home, kind, deadline)?;
    spool.locked(|| {
        let mut count = 0;
        let mut refused = None;
        for path in spool.list()? {
            if Instant::now() >= deadline {
                break;
            }
            let mut pending = spool.read(&path)?;
            match (kind, &mut pending.request.operation) {
                ("hooks", ClaudeOperation::Hook { replayed, .. }) => *replayed = true,
                ("acks", ClaudeOperation::Ack { .. }) => {}
                _ => {
                    return Err(io::Error::other(
                        "pending file is not a replayable hook or ACK",
                    ));
                }
            }
            let reply = match exchange(home, &pending.request, deadline) {
                Ok(reply) => reply,
                Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                    refused = Some(e);
                    continue;
                }
                Err(e) => return Err(e),
            };
            if !reply.committed {
                return Err(io::Error::other(
                    "daemon did not commit the pending receipt",
                ));
            }
            spool.remove(&path)?;
            count += 1;
        }
        match refused {
            Some(e) => Err(e),
            None => Ok(count),
        }
    })
}
/// A nonblocking pipe write with one fixed deadline, including stdout flush.
fn output(bytes: &[u8], deadline: Instant) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let stdout = io::stdout();
    let fd = stdout.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        let mut sent = 0;
        while sent < bytes.len() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "stdout write timed out; delivery outcome may be unknown",
                ));
            }
            let mut poll = libc::pollfd {
                fd,
                events: libc::POLLOUT,
                revents: 0,
            };
            let n = unsafe {
                libc::poll(
                    &mut poll,
                    1,
                    left.as_millis().min(i32::MAX as u128).max(1) as i32,
                )
            };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if n == 0 {
                continue;
            }
            let n = unsafe { libc::write(fd, bytes[sent..].as_ptr().cast(), bytes.len() - sent) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    continue;
                }
                return Err(e);
            }
            if n == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            sent += n as usize;
        }
        Ok(())
    })();
    unsafe {
        libc::fcntl(fd, libc::F_SETFL, flags);
    }
    result
}
fn json_output(value: &serde_json::Value, deadline: Instant) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    if bytes.len() >= MAX_LINE_BYTES {
        return Err(io::Error::other(
            "stdout JSON exceeds line limit; nothing written",
        ));
    }
    bytes.push(b'\n');
    output(&bytes, deadline)
}
