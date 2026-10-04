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
    for deadline in [Instant::now() + Duration::from_secs(5), Instant::now()] {
        let start = Instant::now();
        let error = exchange_once(
            &dir.path().join("missing.sock"),
            None,
            V1_3,
            &request(),
            deadline,
        )
        .unwrap_err();
        assert!(matches!(error, ClientError::Connect { .. }), "{error:?}");
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
                std::thread::sleep(Duration::from_millis(150));
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
        start + Duration::from_millis(250),
    )
    .unwrap_err();
    assert!(matches!(error, ClientError::Disconnected(_)), "{error:?}");
    assert!(
        start.elapsed() < Duration::from_millis(400),
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
