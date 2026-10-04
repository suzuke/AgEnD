//! One RPC, one connection, one deadline (D40 P1/P7). A failed write may
//! have written a prefix, so neither writes nor lost replies are replayed.
//!
//! Must NOT: reconnect, retry a request, or create a detached timeout thread.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Instant;

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::client::{ClientRequest, ClientResponse, MAX_LINE_BYTES, error_code};
use socket2::{Domain, SockAddr, Socket, Type};

use crate::{ClientError, connection::request_id, version};

fn remaining(deadline: Instant) -> io::Result<std::time::Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "one-shot deadline elapsed",
        ));
    }
    Ok(left)
}

fn write(stream: &mut UnixStream, request: &ClientRequest, deadline: Instant) -> io::Result<()> {
    let mut line = serde_json::to_vec(request).expect("protocol types serialize");
    line.push(b'\n');
    if line.len() > MAX_LINE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "request exceeds protocol line limit",
        ));
    }
    let mut sent = 0;
    while sent < line.len() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(&line[sent..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => sent += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn read(reader: &mut BufReader<UnixStream>, deadline: Instant) -> io::Result<ClientResponse> {
    let mut line = Vec::new();
    loop {
        reader
            .get_ref()
            .set_read_timeout(Some(remaining(deadline)?))?;
        let buf = match reader.fill_buf() {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if buf.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let end = buf.iter().position(|b| *b == b'\n').map(|n| n + 1);
        let take = end.unwrap_or(buf.len());
        if line.len() + take > MAX_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "response exceeds protocol line limit",
            ));
        }
        line.extend_from_slice(&buf[..take]);
        reader.consume(take);
        if end.is_some() {
            if line.iter().all(u8::is_ascii_whitespace) {
                line.clear();
                continue;
            }
            return serde_json::from_slice(&line)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
        }
    }
}

fn daemon_error(response: &ClientResponse) -> Option<ClientError> {
    match response {
        ClientResponse::Error { data } => Some(ClientError::Daemon {
            code: data.code.clone(),
            message: data.message.clone(),
        }),
        _ => None,
    }
}

/// Connect, negotiate at least `needed`, send `request` once and receive its
/// correlated reply by `deadline`. No automatic reconnect or replay, even
/// on partial writes. After an error the caller must reconcile durable
/// state before deciding whether another body delivery is safe.
///
/// Only RPCs carrying request ids are accepted; subscriptions and terminal
/// operations use their existing connection APIs. Events are discarded on
/// this short-lived connection. `deadline` covers connect and hello too.
pub fn exchange_once(
    socket: &Path,
    caller: Option<String>,
    needed: ProtocolVersion,
    request: &ClientRequest,
    deadline: Instant,
) -> Result<ClientResponse, ClientError> {
    let id = request_id(request).ok_or_else(|| ClientError::Daemon {
        code: error_code::INVALID_REQUEST.into(),
        message: "one-shot exchange requires a correlated RPC".into(),
    })?;
    let connect_error = |e: io::Error| ClientError::Connect {
        socket: socket.to_owned(),
        cause: e.to_string(),
    };
    let transport_error = |e: io::Error| {
        ClientError::Disconnected(format!(
            "one-shot exchange ended; request outcome may be unknown: {e}"
        ))
    };
    let conn = Socket::new(Domain::UNIX, Type::STREAM, None).map_err(connect_error)?;
    conn.connect_timeout(
        &SockAddr::unix(socket).map_err(connect_error)?,
        remaining(deadline).map_err(connect_error)?,
    )
    .map_err(connect_error)?;
    let mut stream = UnixStream::from(std::os::fd::OwnedFd::from(conn));
    let mut reader = BufReader::new(stream.try_clone().map_err(connect_error)?);
    write(&mut stream, &ClientRequest::hello_as(caller), deadline).map_err(connect_error)?;
    match read(&mut reader, deadline).map_err(connect_error)? {
        ClientResponse::Hello { data } => {
            version::check_at_least(data.selected, needed).map_err(ClientError::Version)?;
        }
        ClientResponse::Error { data } if data.code == error_code::VERSION_MISMATCH => {
            return Err(ClientError::Version(data.message));
        }
        response => {
            return Err(daemon_error(&response).unwrap_or_else(|| {
                ClientError::Disconnected("unexpected one-shot hello reply".into())
            }));
        }
    }
    write(&mut stream, request, deadline).map_err(transport_error)?;
    loop {
        let response = read(&mut reader, deadline).map_err(transport_error)?;
        let matches = match &response {
            ClientResponse::CommandResult { data } => data.request_id == id,
            ClientResponse::Fleet { data } => data.request_id == id,
            ClientResponse::Error { data } => {
                data.request_id.as_deref().is_none_or(|reply| reply == id)
            }
            _ => false,
        };
        if matches {
            return daemon_error(&response).map_or(Ok(response), Err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::protocol::client::{AgentCommand, ClientCommandData};
    use std::time::Duration;

    #[test]
    fn a_partial_write_stops_at_the_deadline() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        let request = ClientRequest::Command {
            data: ClientCommandData {
                request_id: "partial".into(),
                command: AgentCommand::Send {
                    to: "target".into(),
                    message: "x".repeat(1 << 20),
                    level: None,
                    message_id: None,
                },
            },
        };
        // A tiny native send buffer makes a real prefix write succeed, then
        // blocks the remaining bytes while the peer deliberately does not read.
        socket2::SockRef::from(&stream)
            .set_send_buffer_size(4096)
            .unwrap();
        let start = Instant::now();
        let error = write(&mut stream, &request, start + Duration::from_millis(100)).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ),
            "{error}"
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        peer.set_nonblocking(true).unwrap();
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut peer, &mut bytes);
        let encoded = serde_json::to_vec(&request).unwrap();
        assert!(!bytes.is_empty());
        assert!(bytes.len() < encoded.len());
        assert_eq!(bytes, encoded[..bytes.len()]);
    }
}
