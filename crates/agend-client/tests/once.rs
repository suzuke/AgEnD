//! Native socket/proxy failures around the real fake-daemon producer.
#![cfg(unix)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use agend_client::{ClientError, exchange_once};
use agend_core::protocol::client::{ClientRequest, ClientResponse, RequestIdData, V1_3, V1_4};
use agend_testkit::contract::client::proxy::{Direction, Options, Proxy};
use agend_testkit::fake_daemon::FakeDaemon;
use agend_testkit::tempdir::TempDir;

fn request() -> ClientRequest {
    ClientRequest::GetFleet {
        data: RequestIdData {
            request_id: "one-shot".into(),
        },
    }
}

#[test]
fn a_correlated_rpc_uses_the_real_producer() {
    let daemon = FakeDaemon::start().unwrap();
    let response = exchange_once(
        daemon.socket_path(),
        None,
        V1_3,
        &request(),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    match response {
        ClientResponse::Fleet { data } => {
            assert_eq!(data.request_id, "one-shot");
            assert_eq!(data.fleet, daemon.fleet());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_version_failure_does_not_send_the_rpc() {
    let daemon = FakeDaemon::start().unwrap();
    let error = exchange_once(
        daemon.socket_path(),
        None,
        V1_4,
        &request(),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap_err();
    assert!(matches!(error, ClientError::Version(_)), "{error:?}");
    assert!(
        !daemon
            .requests()
            .iter()
            .any(|r| matches!(r, ClientRequest::GetFleet { .. }))
    );
}

#[test]
fn a_missing_daemon_and_expired_deadline_do_not_retry() {
    let dir = TempDir::new("client-once-missing").unwrap();
    for (deadline, expired) in [
        (Instant::now() + Duration::from_secs(5), false),
        (Instant::now(), true),
    ] {
        let start = Instant::now();
        let error = exchange_once(
            &dir.path().join("missing.sock"),
            None,
            V1_3,
            &request(),
            deadline,
        )
        .unwrap_err();
        assert!(
            if expired {
                matches!(error, ClientError::Disconnected(ref message) if message.contains("request was not sent"))
            } else {
                matches!(error, ClientError::Connect { .. })
            },
            "{error:?}"
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}

#[test]
fn hello_and_reply_share_one_deadline() {
    let daemon = FakeDaemon::start().unwrap();
    let proxy = Proxy::start(
        daemon.socket_path().to_owned(),
        Options::rewrite(Arc::new(|_, direction, line| {
            if direction == Direction::ToClient {
                // Keep ample handshake scheduling slack on loaded CI runners.
                // Resetting the deadline for the reply still returns success
                // and is caught by unwrap_err below.
                std::thread::sleep(Duration::from_secs(1));
            }
            vec![line]
        })),
    )
    .unwrap();
    let start = Instant::now();
    let error = exchange_once(
        proxy.socket(),
        None,
        V1_3,
        &request(),
        start + Duration::from_millis(1500),
    )
    .unwrap_err();
    assert!(matches!(error, ClientError::Disconnected(_)), "{error:?}");
    assert!(
        start.elapsed() < Duration::from_millis(2500),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_lost_reply_is_never_replayed() {
    let daemon = FakeDaemon::start().unwrap();
    let proxy = Proxy::start(
        daemon.socket_path().to_owned(),
        Options::rewrite(Arc::new(|_, direction, line| {
            if direction == Direction::ToClient && line.contains("\"type\":\"fleet\"") {
                vec![]
            } else {
                vec![line]
            }
        })),
    )
    .unwrap();
    let error = exchange_once(
        proxy.socket(),
        None,
        V1_3,
        &request(),
        Instant::now() + Duration::from_millis(200),
    )
    .unwrap_err();
    assert!(matches!(error, ClientError::Disconnected(_)), "{error:?}");
    let requests = daemon.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|r| matches!(r, ClientRequest::GetFleet { .. }))
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| matches!(r, ClientRequest::Hello { .. }))
            .count(),
        1
    );
}

#[test]
fn an_uncorrelated_operation_is_rejected_before_connecting() {
    let dir = TempDir::new("client-once-uncorrelated").unwrap();
    let error = exchange_once(
        &dir.path().join("missing.sock"),
        None,
        V1_3,
        &ClientRequest::hello(),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap_err();
    assert!(
        matches!(error, ClientError::Daemon { ref code, .. } if code == "invalid_request"),
        "{error:?}"
    );
}

#[test]
fn a_dripping_hello_cannot_extend_the_deadline() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};

    let daemon = FakeDaemon::start().unwrap();
    let dir = TempDir::new("client-once-drip").unwrap();
    let path = dir.path().join("drip.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let upstream = daemon.socket_path().to_owned();
    // Relay the producer's actual hello bytes, one at a time. A per-read
    // timeout with read_line would permit the entire slow line through.
    let relay = std::thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut server = UnixStream::connect(upstream).unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut hello = String::new();
        BufReader::new(client.try_clone().unwrap())
            .read_line(&mut hello)
            .unwrap();
        server.write_all(hello.as_bytes()).unwrap();
        let mut line = String::new();
        BufReader::new(server).read_line(&mut line).unwrap();
        for byte in line.bytes() {
            if client.write_all(&[byte]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let start = Instant::now();
    let error = exchange_once(
        &path,
        None,
        V1_3,
        &request(),
        start + Duration::from_millis(100),
    )
    .unwrap_err();
    assert!(matches!(error, ClientError::Connect { .. }), "{error:?}");
    assert!(
        start.elapsed() < Duration::from_millis(500),
        "{:?}",
        start.elapsed()
    );
    relay.join().unwrap();
    assert!(
        !daemon
            .requests()
            .iter()
            .any(|r| matches!(r, ClientRequest::GetFleet { .. }))
    );
}

#[test]
fn an_oversized_response_is_refused_before_reading_the_whole_line() {
    use agend_core::protocol::client::MAX_LINE_BYTES;
    let daemon = FakeDaemon::start().unwrap();
    let proxy = Proxy::start(
        daemon.socket_path().to_owned(),
        Options::rewrite(Arc::new(|_, direction, line| {
            if direction == Direction::ToClient && line.contains("\"type\":\"fleet\"") {
                let mut response: serde_json::Value = serde_json::from_str(&line).unwrap();
                response["padding"] = "x".repeat(MAX_LINE_BYTES).into();
                vec![serde_json::to_string(&response).unwrap()]
            } else {
                vec![line]
            }
        })),
    )
    .unwrap();
    let error = exchange_once(
        proxy.socket(),
        None,
        V1_3,
        &request(),
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(
        matches!(error, ClientError::Disconnected(ref message) if message.contains("line limit")),
        "{error:?}"
    );
}

#[test]
fn oversized_strings_are_rejected_before_json_scanning_or_connecting() {
    use agend_core::protocol::client::{
        AgentCommand, ClientCommandData, OperatorCommand, OperatorData,
    };
    let daemon = FakeDaemon::start().unwrap();
    let oversized = "x".repeat(32 << 20);
    let requests = [
        ClientRequest::Command {
            data: ClientCommandData {
                request_id: "send".into(),
                command: AgentCommand::Send {
                    to: "target".into(),
                    message: oversized.clone(),
                    level: None,
                    message_id: None,
                },
            },
        },
        ClientRequest::Command {
            data: ClientCommandData {
                request_id: "ask".into(),
                command: AgentCommand::Ask {
                    question: oversized.clone(),
                    options: vec![],
                },
            },
        },
        ClientRequest::Operator {
            data: OperatorData {
                request_id: "workflow".into(),
                command: OperatorCommand::WorkflowCheck {
                    toml: oversized.clone(),
                },
            },
        },
        ClientRequest::GetFleet {
            data: RequestIdData {
                request_id: oversized.clone(),
            },
        },
    ];
    for request in requests {
        let start = Instant::now();
        let error = exchange_once(
            daemon.socket_path(),
            None,
            V1_3,
            &request,
            start + Duration::from_millis(25),
        )
        .unwrap_err();
        assert!(
            matches!(error, ClientError::Disconnected(ref message) if message.contains("request was not sent")),
            "{error:?}"
        );
        assert!(
            start.elapsed() < Duration::from_millis(250),
            "{:?}",
            start.elapsed()
        );
    }
    let start = Instant::now();
    exchange_once(
        daemon.socket_path(),
        Some(oversized),
        V1_3,
        &request(),
        start + Duration::from_millis(25),
    )
    .unwrap_err();
    assert!(start.elapsed() < Duration::from_millis(250));
    assert!(
        daemon.requests().is_empty(),
        "oversized preparation must not connect"
    );
}

#[test]
fn escaped_encoding_obeys_the_deadline_without_sending_a_partial_request() {
    use agend_core::protocol::client::{AgentCommand, ClientCommandData};
    let daemon = FakeDaemon::start().unwrap();
    let request = ClientRequest::Command {
        data: ClientCommandData {
            request_id: "escape".into(),
            command: AgentCommand::Send {
                to: "target".into(),
                message: "\u{1}".repeat(1 << 20),
                level: None,
                message_id: None,
            },
        },
    };
    let start = Instant::now();
    let error = exchange_once(
        daemon.socket_path(),
        None,
        V1_3,
        &request,
        start + Duration::from_millis(25),
    )
    .unwrap_err();
    assert!(
        matches!(error, ClientError::Disconnected(ref message) if message.contains("request was not sent")),
        "{error:?}"
    );
    assert!(
        start.elapsed() < Duration::from_millis(250),
        "{:?}",
        start.elapsed()
    );
    assert!(daemon.requests().is_empty());
}

#[test]
fn plain_string_encoding_checks_the_deadline_during_the_scan() {
    use agend_core::protocol::client::{MAX_LINE_BYTES, OperatorCommand, OperatorData};
    let daemon = FakeDaemon::start().unwrap();
    let request = ClientRequest::Operator {
        data: OperatorData {
            request_id: "plain".into(),
            command: OperatorCommand::WorkflowCheck {
                toml: "x".repeat(MAX_LINE_BYTES - 1024),
            },
        },
    };
    let start = Instant::now();
    let error = exchange_once(
        daemon.socket_path(),
        None,
        V1_3,
        &request,
        start + Duration::from_millis(20),
    )
    .unwrap_err();
    assert!(
        matches!(error, ClientError::Disconnected(ref message) if message.contains("deadline elapsed")),
        "{error:?}"
    );
    assert!(
        start.elapsed() < Duration::from_millis(120),
        "{:?}",
        start.elapsed()
    );
    assert!(daemon.requests().is_empty());
}

#[test]
fn escaped_line_size_is_bounded_during_preparation() {
    use agend_core::protocol::client::{AgentCommand, ClientCommandData, MAX_LINE_BYTES};
    let daemon = FakeDaemon::start().unwrap();
    let request = ClientRequest::Command {
        data: ClientCommandData {
            request_id: "line-limit".into(),
            command: AgentCommand::Ask {
                question: "\u{1}".repeat(MAX_LINE_BYTES / 6 + 1),
                options: vec![],
            },
        },
    };
    let error = exchange_once(
        daemon.socket_path(),
        None,
        V1_3,
        &request,
        Instant::now() + Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(
        matches!(error, ClientError::Disconnected(ref message) if message.contains("line limit")),
        "{error:?}"
    );
    assert!(daemon.requests().is_empty());
}
