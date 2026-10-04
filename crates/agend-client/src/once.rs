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

mod decode;
mod prepare;
mod sliced_json;

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

struct Encoding {
    line: Vec<u8>,
    deadline: Instant,
}

impl Write for Encoding {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        remaining(self.deadline)?;
        if bytes.len() > MAX_LINE_BYTES.saturating_sub(self.line.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "request exceeds protocol line limit",
            ));
        }
        self.line.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        remaining(self.deadline).map(|_| ())
    }
}

fn encode(request: &ClientRequest, deadline: Instant) -> io::Result<Vec<u8>> {
    prepare::check(request, deadline)?;
    let mut encoding = Encoding {
        line: Vec::new(),
        deadline,
    };
    serde_json::to_writer(&mut encoding, &sliced_json::Checked(request))
        .map_err(|e| io::Error::new(e.io_error_kind().unwrap_or(io::ErrorKind::InvalidData), e))?;
    encoding.write_all(b"\n")?;
    Ok(encoding.line)
}

fn write(stream: &mut UnixStream, line: &[u8], deadline: Instant) -> io::Result<()> {
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
            let response = decode::response(&line, deadline)?;
            remaining(deadline)?;
            return Ok(response);
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
    let preparation_error = |e: io::Error| {
        ClientError::Disconnected(format!(
            "one-shot preparation failed; request was not sent: {e}"
        ))
    };
    // Prepare bounded lines before opening a socket. Oversized strings
    // cannot spend the deadline scanning JSON or allocate an unbounded Vec.
    let line = encode(request, deadline).map_err(preparation_error)?;
    let hello = encode(&ClientRequest::hello_as(caller), deadline).map_err(preparation_error)?;
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
    write(&mut stream, &hello, deadline).map_err(connect_error)?;
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
    write(&mut stream, &line, deadline).map_err(transport_error)?;
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
    fn sliced_strings_keep_the_native_json_wire_format() {
        let text = format!("{}繁中é\"\\\n\u{1}{}", "a".repeat(4095), "界".repeat(5000));
        let requests = [
            ClientRequest::hello_as(Some(text.clone())),
            ClientRequest::Command {
                data: ClientCommandData {
                    request_id: "unicode".into(),
                    command: AgentCommand::Ask {
                        question: text.clone(),
                        options: vec![text, String::new()],
                    },
                },
            },
        ];
        for request in requests {
            let mut native = serde_json::to_vec(&request).unwrap();
            native.push(b'\n');
            let actual = encode(&request, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(actual, native);
        }
    }

    #[test]
    fn parsing_a_large_real_producer_response_checks_the_deadline() {
        use agend_core::protocol::client::RequestIdData;
        use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};
        use std::io::Read;
        let daemon = FakeDaemon::start().unwrap();
        let (mut source, _) = ProbeClient::hello(daemon.socket_path(), None).unwrap();
        let mut response = source
            .request(&ClientRequest::GetFleet {
                data: RequestIdData {
                    request_id: "parse".into(),
                },
            })
            .unwrap();
        if let ClientResponse::Fleet { data } = &mut response {
            data.fleet.teams = vec![data.fleet.teams[0].clone(); 340_000];
        } else {
            panic!("producer did not return fleet");
        }
        let mut line = serde_json::to_vec(&response).unwrap();
        line.push(b'\n');
        assert!(line.len() < MAX_LINE_BYTES);
        // Exercise the CPU deadline without depending on socket scheduling.
        let start = Instant::now();
        let error = decode::response(&line, start + Duration::from_millis(80)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("one-shot deadline elapsed"));
        assert!(start.elapsed() < Duration::from_millis(180));
        let (stream, mut peer) = UnixStream::pair().unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let writer = std::thread::spawn(move || {
            let _ = peer.write_all(&line);
            let _ = peer.read(&mut [0]);
        });
        let start = Instant::now();
        let mut reader = BufReader::new(stream);
        let error = read(&mut reader, start + Duration::from_millis(80)).unwrap_err();
        // Native I/O may expire before parsing starts; both paths must stop.
        assert!(matches!(
            error.kind(),
            io::ErrorKind::InvalidData | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
        if error.kind() == io::ErrorKind::InvalidData {
            assert!(error.to_string().contains("one-shot deadline elapsed"));
        }
        assert!(
            start.elapsed() < Duration::from_millis(180),
            "{:?}",
            start.elapsed()
        );
        drop(reader);
        writer.join().unwrap();
    }

    #[test]
    fn data_before_type_does_not_leave_an_unchecked_content_conversion() {
        use agend_core::protocol::client::{RequestIdData, TaskView};
        use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};
        let daemon = FakeDaemon::start().unwrap();
        daemon.set_task(TaskView {
            task_id: "max".into(),
            title: "max".into(),
            team_id: "general".into(),
            status: "open".into(),
            assignee: None,
            stages: vec![String::new(); 2_790_000],
            current_stage: None,
            pipeline: None,
        });
        let (mut source, _) = ProbeClient::hello(daemon.socket_path(), None).unwrap();
        let response = source
            .request(&ClientRequest::GetFleet {
                data: RequestIdData {
                    request_id: "max-stage-tail".into(),
                },
            })
            .unwrap();
        // The real producer's response, reordered by serde_json's native map
        // serializer. JSON field order must not select a slower unchecked path.
        let mut line = serde_json::to_vec(&serde_json::to_value(&response).unwrap()).unwrap();
        line.push(b'\n');
        assert!(line.len() < MAX_LINE_BYTES);
        drop(source);
        drop(daemon);
        drop(response);
        for budget in [650, 665, 680] {
            let (stream, mut peer) = UnixStream::pair().unwrap();
            peer.set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let bytes = line.clone();
            let writer = std::thread::spawn(move || {
                let _ = peer.write_all(&bytes);
                std::thread::sleep(Duration::from_millis(100));
            });
            let start = Instant::now();
            let mut reader = BufReader::new(stream);
            let _ = read(&mut reader, start + Duration::from_millis(budget));
            assert!(
                start.elapsed() < Duration::from_millis(budget + 100),
                "budget={budget}, elapsed={:?}",
                start.elapsed()
            );
            drop(reader);
            writer.join().unwrap();
        }
    }

    #[test]
    fn staged_decoder_matches_native_producer_replies_in_either_field_order() {
        use agend_core::protocol::client::{InstanceView, RequestIdData};
        use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};
        let daemon = FakeDaemon::start().unwrap();
        daemon.set_instance(InstanceView {
            instance_id: "g-1".into(),
            team_id: "general".into(),
            backend: "claude".into(),
            state: agend_core::protocol::client::AgentState::Idle,
            working_directory: None,
        });
        let (mut source, _) = ProbeClient::hello(daemon.socket_path(), Some("g-1")).unwrap();
        let mut responses = Vec::new();
        for command in [
            AgentCommand::Status,
            AgentCommand::Inbox {
                after_message_id: None,
            },
            AgentCommand::TaskCreate {
                title: "繁中é".into(),
                role: "dev".into(),
                team_id: None,
                workflow_id: None,
            },
            AgentCommand::Ask {
                question: "q\"\\\n繁中é".into(),
                options: vec!["a".into()],
            },
            AgentCommand::Block {
                task_id: "t-1".into(),
                reason: "blocked".into(),
            },
            AgentCommand::Unknown,
        ] {
            responses.push(
                source
                    .request(&ClientRequest::Command {
                        data: ClientCommandData {
                            request_id: "decode".into(),
                            command,
                        },
                    })
                    .unwrap(),
            );
        }
        responses.push(
            source
                .request(&ClientRequest::GetFleet {
                    data: RequestIdData {
                        request_id: "fleet".into(),
                    },
                })
                .unwrap(),
        );
        for response in responses {
            for bytes in [
                serde_json::to_vec(&response).unwrap(),
                serde_json::to_vec(&serde_json::to_value(&response).unwrap()).unwrap(),
            ] {
                let actual =
                    decode::response(&bytes, Instant::now() + Duration::from_secs(2)).unwrap();
                assert_eq!(actual, response);
            }
        }
    }

    #[test]
    fn nested_ask_options_do_not_leave_an_unchecked_content_conversion() {
        use agend_core::protocol::{ask::*, client::RequestIdData};
        use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};
        let daemon = FakeDaemon::start().unwrap();
        daemon.open_ask(
            AskThread {
                ask_id: "large-options".into(),
                task_id: None,
                entries: vec![AskEntry::Question {
                    from: "dev".into(),
                    text: "options".into(),
                    options: vec![String::new(); 2_785_000],
                }],
            },
            None,
        );
        let (mut source, _) = ProbeClient::hello(daemon.socket_path(), None).unwrap();
        let response = source
            .request(&ClientRequest::GetFleet {
                data: RequestIdData {
                    request_id: "nested".into(),
                },
            })
            .unwrap();
        let bytes = serde_json::to_vec(&serde_json::to_value(&response).unwrap()).unwrap();
        assert!(bytes.len() < MAX_LINE_BYTES);
        drop(response);
        drop(source);
        drop(daemon);
        for budget in [875, 890, 900, 925] {
            let start = Instant::now();
            let result = decode::response(&bytes, start + Duration::from_millis(budget));
            assert!(
                start.elapsed() < Duration::from_millis(budget + 100),
                "budget={budget}, elapsed={:?}",
                start.elapsed()
            );
            drop(result);
        }
    }

    #[test]
    fn staged_nested_ask_decoder_matches_the_real_producer() {
        use agend_core::protocol::{ask::*, client::RequestIdData};
        use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};
        let daemon = FakeDaemon::start().unwrap();
        daemon.open_ask(
            AskThread {
                ask_id: "conversation".into(),
                task_id: Some("task".into()),
                entries: vec![
                    AskEntry::Question {
                        from: "dev".into(),
                        text: "繁中é".into(),
                        options: vec!["choice".into()],
                    },
                    AskEntry::Answer {
                        from: "operator".into(),
                        source: AnswerSource::Cli,
                        reply: AskReply::Choice {
                            option: "choice".into(),
                        },
                    },
                    AskEntry::FollowUp {
                        from: "dev".into(),
                        text: "follow up".into(),
                        options: vec![],
                    },
                    AskEntry::Answer {
                        from: "operator".into(),
                        source: AnswerSource::Tui,
                        reply: AskReply::Text {
                            text: "答覆".into(),
                        },
                    },
                    AskEntry::Answer {
                        from: "operator".into(),
                        source: AnswerSource::Unknown,
                        reply: AskReply::Unknown,
                    },
                    AskEntry::Resolution {
                        from: "dev".into(),
                        summary: "resolved".into(),
                    },
                    AskEntry::Unknown,
                ],
            },
            None,
        );
        let (mut source, _) = ProbeClient::hello(daemon.socket_path(), None).unwrap();
        let response = source
            .request(&ClientRequest::GetFleet {
                data: RequestIdData {
                    request_id: "ask".into(),
                },
            })
            .unwrap();
        for bytes in [
            serde_json::to_vec(&response).unwrap(),
            serde_json::to_vec(&serde_json::to_value(&response).unwrap()).unwrap(),
        ] {
            assert_eq!(
                decode::response(&bytes, Instant::now() + Duration::from_secs(2)).unwrap(),
                response
            );
        }
    }

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
        let encoded = encode(&request, Instant::now() + Duration::from_secs(2)).unwrap();
        let start = Instant::now();
        let error = write(&mut stream, &encoded, start + Duration::from_millis(100)).unwrap_err();
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
        assert!(!bytes.is_empty());
        assert!(bytes.len() < encoded.len());
        assert_eq!(bytes, encoded[..bytes.len()]);
    }
}
