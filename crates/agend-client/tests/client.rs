//! `agend-client` against the testkit fake daemon (gate 8 P7): retry and
//! its limits, version checks, which requests are sent again after the
//! daemon restarts, daemon errors, events. The fake speaks the real wire
//! format (core types + serde_json), and the CLP contract keeps it honest
//! against the real daemon.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agend_client::{Client, ClientError, Redo};
use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::client::{
    AttentionAction, AttentionRequiredData, ClientRequest, ClientResponse, DaemonEvent,
    RequestIdData, TaskChangedData, V1, V1_1,
};
use agend_testkit::contract::client::proxy::{Direction, Options, Proxy, Transform};
use agend_testkit::fake_daemon::FakeDaemon;
use agend_testkit::tempdir::TempDir;

fn event() -> DaemonEvent {
    DaemonEvent::TaskChanged {
        data: TaskChangedData {
            task_id: "T-1".into(),
            summary: "changed".into(),
            task: None,
        },
    }
}

fn item(id: &str) -> AttentionRequiredData {
    AttentionRequiredData {
        reason: "failed".into(),
        task_id: None,
        ask: None,
        recap: None,
        attention_id: Some(id.into()),
        unblocks: Some(0),
        waiting_since_unix_ms: Some(1),
        if_ignored: None,
        actions: vec![AttentionAction::Retry],
        instance_id: None,
    }
}

fn socket_in(dir: &TempDir) -> PathBuf {
    dir.path().join("daemon.sock")
}

#[test]
fn connect_retries_for_ten_seconds_then_says_what_to_do() {
    let dir = TempDir::new("client-none").unwrap();
    let socket = socket_in(&dir);
    let started = Instant::now();
    let error = Client::connect(&socket, None).err().unwrap();
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(10) && took < Duration::from_secs(11),
        "{took:?}"
    );
    assert_eq!(
        error.to_string(),
        format!(
            "cannot reach the AgEnD daemon at {} after 10 s (No such file or directory (os error 2)). Is it running? Start it with: agend daemon",
            socket.display()
        )
    );
}

#[test]
fn connect_once_does_not_retry() {
    let dir = TempDir::new("client-once").unwrap();
    let started = Instant::now();
    let error = Client::connect_once(&socket_in(&dir), None).err().unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(error, ClientError::Connect { .. }), "{error:?}");
}

#[test]
fn connect_waits_for_a_daemon_that_comes_back() {
    let dir = TempDir::new("client-back").unwrap();
    let socket = socket_in(&dir);
    let path = socket.clone();
    let late = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1200));
        FakeDaemon::start_at(&path).unwrap()
    });
    let mut client = Client::connect(&socket, None).unwrap();
    let _daemon = late.join().unwrap();
    assert!(
        client.retried() >= Duration::from_secs(1),
        "{:?}",
        client.retried()
    );
    assert_eq!(client.selected(), V1_1);
    client.get_fleet().unwrap();
}

#[test]
fn a_1_0_daemon_fails_at_once_with_what_to_do() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.set_supported_versions(&[V1]);
    let started = Instant::now();
    let error = Client::connect(daemon.socket_path(), None).err().unwrap();
    assert!(started.elapsed() < Duration::from_secs(1), "not retried");
    assert_eq!(
        error,
        ClientError::Version(
            "the daemon speaks client protocol 1.0; this agend needs 1.1 — restart the daemon with this binary".into()
        )
    );
}

#[test]
fn a_version_mismatch_is_not_retried() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.set_supported_versions(&[ProtocolVersion::new(2, 0)]);
    let started = Instant::now();
    let error = Client::connect(daemon.socket_path(), None).err().unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        error.to_string(),
        "client protocol version mismatch: local supports 2.0, remote supports 1.1"
    );
}

/// A proxy that swallows the first request line of the first connection
/// (so it is sent but never answered), in front of `upstream`.
fn swallowing_proxy(upstream: &Path, request_type: &'static str) -> Proxy {
    let transform: Transform = Arc::new(move |state, direction, line| {
        let first_request = direction == Direction::ToServer
            && state.number == 1
            && line.contains(&format!(r#""type":"{request_type}""#));
        if first_request {
            return vec![];
        }
        vec![line]
    });
    Proxy::start(upstream.to_path_buf(), Options::rewrite(transform)).unwrap()
}

/// Restarts the fake on `socket` after `after`.
fn restart_later(daemon: FakeDaemon, after: Duration) -> std::thread::JoinHandle<FakeDaemon> {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        let socket = daemon.socket_path().to_path_buf();
        drop(daemon);
        std::thread::sleep(Duration::from_millis(300));
        FakeDaemon::start_at(&socket).unwrap()
    })
}

#[test]
fn a_read_is_sent_again_after_the_daemon_restarts() {
    let dir = TempDir::new("client-redo").unwrap();
    let daemon = FakeDaemon::start_at(&socket_in(&dir)).unwrap();
    let proxy = swallowing_proxy(daemon.socket_path(), "get_fleet");
    let mut client = Client::connect(proxy.socket(), None).unwrap();
    let restarted = restart_later(daemon, Duration::from_millis(300));
    let fleet = client.get_fleet().unwrap();
    let daemon = restarted.join().unwrap();
    assert_eq!(fleet, daemon.fleet());
    assert!(client.retried() > Duration::ZERO);
}

#[test]
fn a_request_that_may_change_something_is_not_sent_again() {
    let dir = TempDir::new("client-noredo").unwrap();
    let daemon = FakeDaemon::start_at(&socket_in(&dir)).unwrap();
    let proxy = swallowing_proxy(daemon.socket_path(), "resolve_attention");
    let mut client = Client::connect(proxy.socket(), None).unwrap();
    let restarted = restart_later(daemon, Duration::from_millis(300));
    let error = client
        .resolve_attention("instance-failed:x", AttentionAction::Retry)
        .unwrap_err();
    let daemon = restarted.join().unwrap();
    assert_eq!(error, ClientError::Restarted);
    assert_eq!(
        error.to_string(),
        "daemon restarted during the request; check with agend status"
    );
    let resolves = daemon
        .requests()
        .into_iter()
        .filter(|r| matches!(r, ClientRequest::ResolveAttention { .. }))
        .count();
    assert_eq!(resolves, 0, "never sent to the restarted daemon");
}

#[test]
fn daemon_errors_keep_their_code_and_only_the_operator_resolves() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.add_attention(item("instance-failed:a"));
    let mut agent = Client::connect(daemon.socket_path(), Some("g8-1".into())).unwrap();
    let error = agent
        .resolve_attention("instance-failed:a", AttentionAction::Retry)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "forbidden: only the operator can resolve needs-you items; ask the operator with agend ask"
    );
    let mut operator = Client::connect(daemon.socket_path(), None).unwrap();
    let error = operator
        .resolve_attention("instance-failed:nope", AttentionAction::Retry)
        .unwrap_err();
    assert!(
        matches!(&error, ClientError::Daemon { code, .. } if code == "unknown_attention"),
        "{error:?}"
    );
    operator
        .resolve_attention("instance-failed:a", AttentionAction::Retry)
        .unwrap();
    assert!(daemon.fleet().attention.is_empty());
}

#[test]
fn events_follow_the_fleet_view_and_a_bad_cursor_is_a_gap() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.emit(event());
    let mut client = Client::connect(daemon.socket_path(), None).unwrap();
    let as_of = client.get_fleet().unwrap().as_of_event_id;
    client.subscribe_events(Some(as_of)).unwrap();
    daemon.emit(event());
    daemon.emit(event());
    let ids: Vec<u64> = (0..2)
        .map(|_| client.next_event().unwrap().event_id)
        .collect();
    assert_eq!(ids, [as_of + 1, as_of + 2]);
    // Events that arrive while waiting for a reply are kept for next_event.
    daemon.emit(event());
    let request = ClientRequest::GetFleet {
        data: RequestIdData {
            request_id: client.next_request_id(),
        },
    };
    std::thread::sleep(Duration::from_millis(100));
    let reply = client.request(&request, Redo::Safe).unwrap();
    assert!(matches!(reply, ClientResponse::Fleet { .. }));
    assert_eq!(client.next_event().unwrap().event_id, as_of + 3);

    let mut late = Client::connect(daemon.socket_path(), None).unwrap();
    late.subscribe_events(Some(as_of + 1_000)).unwrap();
    let error = late.next_event().unwrap_err();
    assert!(
        matches!(&error, ClientError::Daemon { code, .. } if code == "event_gap"),
        "{error:?}"
    );
    drop(daemon);
    assert_eq!(
        client.next_event().unwrap_err(),
        ClientError::Disconnected("the daemon closed the connection".into())
    );
}
