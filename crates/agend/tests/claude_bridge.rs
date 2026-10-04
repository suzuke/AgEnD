//! Native daemon/holder/helper processes. No real Claude or model calls.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::{
    model::{Backend, DeliveryState},
    policy::busy::BusyLevel,
    protocol::client::*,
    runtime_records::*,
};
use agend_daemon::store::SqliteStore;
use agend_testkit::{
    block_on,
    fake_agent::claude::{ack_request, hook_payload, initialize_request},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
const BIN: &str = env!("CARGO_BIN_EXE_agend");
const SESSION: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";
fn id() -> String {
    agend_daemon::store::instances::new_session_id().unwrap()
}
struct Fixture {
    daemon: Option<lab::Daemon>,
    lab: lab::Lab,
    home: PathBuf,
    ids: Vec<String>,
}
impl Fixture {
    fn new(messages: usize) -> Self {
        Self::with_body(messages, None)
    }
    fn with_body(messages: usize, body: Option<&str>) -> Self {
        let lab = lab::Lab::with_prefix(Path::new(BIN), "g12b");
        let home = lab.home(0);
        let store = SqliteStore::open(&home, 0).unwrap();
        block_on(store.add_instance(&Instance {
            id: "claude".into(),
            backend: Backend::Claude,
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "printf 'native ready\\n'; exec sleep 600".into(),
                "fake-claude".into(),
            ],
            working_directory: home.display().to_string(),
            session_id: Some(SESSION.into()),
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }))
        .unwrap();
        let ids: Vec<_> = (0..messages).map(|_| id()).collect();
        for (i, id) in ids.iter().enumerate() {
            block_on(store.claim_message(
                &NewMessage {
                    id: id.clone(),
                    from_instance: "operator".into(),
                    to_instance: "claude".into(),
                    task_id: None,
                    body:
                        body.map_or_else(|| format!("message {i}: 完整內容 é\nend"), str::to_owned),
                    level: BusyLevel::Queue,
                },
                0,
            ))
            .unwrap();
        }
        drop(store);
        Self {
            daemon: None,
            lab,
            home,
            ids,
        }
    }
    fn start(&mut self) {
        let mut daemon = lab::Daemon::start(&self.lab, &self.home, &[]).unwrap();
        daemon.ready().unwrap();
        self.daemon = Some(daemon);
    }
    fn stop(&mut self) {
        if let Some(mut d) = self.daemon.take() {
            d.interrupt().unwrap();
        }
    }
    fn kill9(&mut self) {
        if let Some(mut d) = self.daemon.take() {
            d.kill9().unwrap();
        }
    }
    fn store(&self) -> SqliteStore {
        assert!(self.daemon.is_none());
        SqliteStore::open(&self.home, 0).unwrap()
    }
    fn hook(&self, event: &str, extra: Value) -> Value {
        let payload = hook_payload(&self.home, SESSION, event, extra);
        let mut child = Command::new(BIN)
            .args(["hook", event])
            .env("AGEND_HOME", &self.home)
            .env("AGEND_INSTANCE", "claude")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).unwrap()
    }
    fn rpc(&self, operation: ClaudeOperation) -> ClaudeReplyData {
        let req = ClientRequest::Claude {
            data: ClaudeRequestData {
                request_id: id(),
                instance_id: "claude".into(),
                operation,
            },
        };
        match agend_client::exchange_once(
            &self.home.join(DAEMON_SOCKET),
            Some("claude".into()),
            V1_5,
            &req,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap()
        {
            ClientResponse::Claude { data } => data,
            r => panic!("{r:?}"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop();
        self.lab.stop_all_holders();
        assert!(self.lab.running_holders().is_empty());
    }
}
struct Channel {
    child: Child,
    input: Option<ChildStdin>,
    output: mpsc::Receiver<Value>,
}
impl Channel {
    fn new(home: &Path) -> Self {
        let mut child = Command::new(BIN)
            .args(["channel", "--instance", "claude"])
            .env("AGEND_HOME", home)
            .env("AGEND_INSTANCE", "claude")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(serde_json::from_str(&line).unwrap()).is_err() {
                    return;
                }
            }
        });
        let mut c = Self {
            child,
            input,
            output,
        };
        c.send(initialize_request());
        let init = c.recv();
        assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
        assert!(
            init.pointer("/result/capabilities/experimental/claude~1channel")
                .is_some()
        );
        c.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        c
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input.as_mut().unwrap(), "{value}").unwrap();
    }
    fn recv(&self) -> Value {
        self.output
            .recv_timeout(Duration::from_secs(15))
            .expect("helper output deadline")
    }
    fn receipt(value: &Value) -> ClaudeReceipt {
        serde_json::from_value(value["params"]["meta"].clone()).unwrap()
    }
    fn written_barrier(&mut self) {
        // The helper cannot answer this stdin ping until its preceding native
        // written RPC completes. stdout arrival alone is not that DB receipt.
        self.send(json!({"jsonrpc":"2.0","id":9,"method":"ping"}));
        loop {
            let value = self.recv();
            if value["id"] == 9 {
                break;
            }
        }
    }
    fn ack(&mut self, receipts: &[ClaudeReceipt]) -> Value {
        self.send(ack_request(7, receipts));
        loop {
            let v = self.recv();
            if v["id"] == 7 {
                return v;
            }
        }
    }
    fn close(&mut self) {
        self.input.take();
        let end = Instant::now() + Duration::from_secs(12);
        loop {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            if Instant::now() >= end {
                self.child.kill().unwrap();
                self.child.wait().unwrap();
                panic!("helper did not exit after EOF");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Channel {
    fn drop(&mut self) {
        self.input.take();
        if self.child.try_wait().unwrap().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
fn pending(home: &Path, kind: &str) -> usize {
    fs::read_dir(home.join("spool").join(kind))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .count()
}
fn wait_empty(home: &Path, kind: &str) {
    let end = Instant::now() + Duration::from_secs(5);
    while pending(home, kind) > 0 {
        assert!(Instant::now() < end, "pending {kind} not committed");
        std::thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn idle_channel_sends_full_native_content_and_only_explicit_ack_confirms() {
    let mut f = Fixture::new(2);
    f.start();
    let mut channel = Channel::new(&f.home);
    assert_eq!(
        f.hook("SessionStart", json!({"source":"startup","model":"fake"})),
        json!({})
    );
    let before = Instant::now();
    let first = channel.recv();
    assert!(before.elapsed() >= Duration::from_secs(4));
    let second = channel.recv();
    assert_eq!(
        first["params"]["content"],
        "From: operator\n\nmessage 0: 完整內容 é\nend"
    );
    let receipts = [Channel::receipt(&first), Channel::receipt(&second)];
    channel.written_barrier();
    assert_eq!(receipts[0].message_id, f.ids[0]);
    assert_eq!(receipts[1].message_id, f.ids[1]);
    f.stop();
    {
        let s = f.store();
        for id in &f.ids {
            assert_eq!(
                block_on(s.message(id)).unwrap().unwrap().state,
                DeliveryState::Sent
            );
        }
    }
    f.start();
    let ack = channel.ack(&receipts);
    assert!(
        ack["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("confirmed")
    );
    channel.close();
    f.stop();
    let s = f.store();
    for id in &f.ids {
        assert_eq!(
            block_on(s.message(id)).unwrap().unwrap().state,
            DeliveryState::Confirmed
        );
    }
}

#[test]
fn busy_stop_blocks_one_complete_batch_active_stop_never_blocks_or_confirms() {
    let mut f = Fixture::new(2);
    f.start();
    assert_eq!(
        f.hook("UserPromptSubmit", json!({"prompt":"work"})),
        json!({})
    );
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty()
    );
    let blocked = f.hook("Stop", json!({"stop_hook_active":false}));
    assert_eq!(blocked["decision"], "block");
    let text = blocked["reason"].as_str().unwrap();
    for id in &f.ids {
        assert!(text.contains(id));
    }
    assert!(text.contains("完整內容 é\nend"));
    assert_eq!(f.hook("Stop", json!({"stop_hook_active":true})), json!({}));
    assert_eq!(f.hook("Stop", json!({"stop_hook_active":false})), json!({}));
    f.stop();
    let s = f.store();
    for id in &f.ids {
        let m = block_on(s.message(id)).unwrap().unwrap();
        assert_eq!(m.state, DeliveryState::Sent);
        let d = block_on(s.claude_delivery(id)).unwrap().unwrap();
        assert_eq!(d.attempt.unwrap().route, ClaudeRoute::Stop);
    }
}

#[test]
fn offline_stop_spool_replays_after_helper_exits_without_claiming_queue_or_idle() {
    let mut f = Fixture::new(1);
    assert_eq!(f.hook("Stop", json!({"stop_hook_active":false})), json!({}));
    assert_eq!(pending(&f.home, "hooks"), 1);
    f.start();
    wait_empty(&f.home, "hooks");
    std::thread::sleep(Duration::from_millis(5100));
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty()
    );
    f.stop();
    let s = f.store();
    let m = block_on(s.message(&f.ids[0])).unwrap().unwrap();
    assert_eq!(m.state, DeliveryState::Queued);
    assert!(m.attempted_at_unix_ms.is_none());
    let events = block_on(s.driver_events_after(0, 32)).unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].event.replayed);
}

#[test]
fn offline_ack_survives_channel_exit_and_daemon_replays_receipt_only() {
    let mut f = Fixture::new(1);
    f.start();
    let mut channel = Channel::new(&f.home);
    f.hook("SessionStart", json!({"source":"startup"}));
    let delivered = channel.recv();
    let receipt = Channel::receipt(&delivered);
    channel.written_barrier();
    f.stop();
    let reply = channel.ack(std::slice::from_ref(&receipt));
    assert!(!reply["result"]["isError"].as_bool().unwrap_or(false));
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("pending sync")
    );
    assert!(pending(&f.home, "acks") > 0);
    channel.close();
    f.start();
    wait_empty(&f.home, "acks");
    f.stop();
    let s = f.store();
    assert_eq!(
        block_on(s.message(&f.ids[0])).unwrap().unwrap().state,
        DeliveryState::Confirmed
    );
    assert_eq!(
        block_on(s.claude_delivery(&f.ids[0]))
            .unwrap()
            .unwrap()
            .attempt
            .unwrap()
            .delivery_id,
        receipt.delivery_id
    );
}

#[test]
fn committed_intent_lost_before_stdout_is_not_replayed_across_four_boots() {
    let mut f = Fixture::new(1);
    f.start();
    f.hook("SessionStart", json!({"source":"startup"}));
    std::thread::sleep(Duration::from_millis(5100));
    // Native service commits the intent. Simulate a dead helper before it
    // writes a channel notification or records a write; save only its tuple.
    let push = f
        .rpc(ClaudeOperation::Poll {
            session_id: SESSION.into(),
        })
        .messages
        .pop()
        .unwrap();
    f.kill9();
    f.start();
    f.hook("SessionStart", json!({"source":"resume"}));
    std::thread::sleep(Duration::from_millis(5100));
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty()
    );
    f.stop();
    {
        let s = f.store();
        let m = block_on(s.message(&f.ids[0])).unwrap().unwrap();
        let d = block_on(s.claude_delivery(&f.ids[0])).unwrap().unwrap();
        assert!(d.outcome_unknown(&m));
    }
    f.start();
    assert!(
        f.rpc(ClaudeOperation::Ack {
            receipts: vec![push.receipt.clone()]
        })
        .committed
    );
    f.stop();
    f.start();
    assert!(
        f.rpc(ClaudeOperation::Ack {
            receipts: vec![push.receipt.clone()]
        })
        .committed
    );
    assert_eq!(f.hook("Stop", json!({"stop_hook_active":false})), json!({}));
    f.stop();
    let s = f.store();
    assert_eq!(
        block_on(s.message(&f.ids[0])).unwrap().unwrap().state,
        DeliveryState::Confirmed
    );
}

#[test]
fn mismatched_identity_version_session_and_unstarted_ack_are_refused() {
    let mut f = Fixture::new(1);
    f.start();
    let data = ClaudeRequestData {
        request_id: id(),
        instance_id: "claude".into(),
        operation: ClaudeOperation::Attach,
    };
    let result = agend_client::exchange_once(
        &f.home.join(DAEMON_SOCKET),
        Some("other".into()),
        V1_5,
        &ClientRequest::Claude { data: data.clone() },
        Instant::now() + Duration::from_secs(2),
    );
    assert!(result.unwrap_err().to_string().contains("forbidden"));
    let socket = std::os::unix::net::UnixStream::connect(f.home.join(DAEMON_SOCKET)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut r = BufReader::new(socket);
    writeln!(
        r.get_mut(),
        "{}",
        serde_json::to_string(&ClientRequest::Hello {
            data: ClientHello {
                supported: vec![V1_4],
                caller: Some("claude".into())
            }
        })
        .unwrap()
    )
    .unwrap();
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    writeln!(
        r.get_mut(),
        "{}",
        serde_json::to_string(&ClientRequest::Claude { data }).unwrap()
    )
    .unwrap();
    line.clear();
    r.read_line(&mut line).unwrap();
    let response: ClientResponse = serde_json::from_str(&line).unwrap();
    assert!(
        matches!(response,ClientResponse::Error {data} if data.code==error_code::NOT_SUPPORTED)
    );
    for op in [
        ClaudeOperation::Poll {
            session_id: OTHER.into(),
        },
        ClaudeOperation::Ack {
            receipts: vec![ClaudeReceipt {
                message_id: f.ids[0].clone(),
                delivery_id: id(),
                session_id: SESSION.into(),
            }],
        },
    ] {
        let req = ClientRequest::Claude {
            data: ClaudeRequestData {
                request_id: id(),
                instance_id: "claude".into(),
                operation: op,
            },
        };
        assert!(
            agend_client::exchange_once(
                &f.home.join(DAEMON_SOCKET),
                Some("claude".into()),
                V1_5,
                &req,
                Instant::now() + Duration::from_secs(2)
            )
            .is_err()
        );
    }
    f.stop();
    let s = f.store();
    assert!(
        block_on(s.message(&f.ids[0]))
            .unwrap()
            .unwrap()
            .attempted_at_unix_ms
            .is_none()
    );
}

#[test]
fn refused_ack_is_a_tool_error_retained_without_confirming_any_message() {
    let mut f = Fixture::new(1);
    f.start();
    let mut channel = Channel::new(&f.home);
    let reply = channel.ack(&[ClaudeReceipt {
        message_id: f.ids[0].clone(),
        delivery_id: id(),
        session_id: SESSION.into(),
    }]);
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(pending(&f.home, "acks"), 1);
    channel.close();
    f.stop();
    let s = f.store();
    assert_eq!(
        block_on(s.message(&f.ids[0])).unwrap().unwrap().state,
        DeliveryState::Queued
    );
}

#[test]
fn failed_local_ack_publication_is_a_tool_error_and_does_not_write_outside_home() {
    let f = Fixture::new(0);
    let outside = f.lab.home(1);
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(f.home.join("spool")).unwrap();
    std::os::unix::fs::symlink(&outside, f.home.join("spool/acks")).unwrap();
    let mut channel = Channel::new(&f.home);
    let reply = channel.ack(&[ClaudeReceipt {
        message_id: id(),
        delivery_id: id(),
        session_id: SESSION.into(),
    }]);
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    channel.close();
}

#[test]
fn corrupt_spool_prefix_does_not_starve_a_later_native_ack_after_helper_exit() {
    let mut f = Fixture::new(1);
    f.start();
    let blocked = f.hook("Stop", json!({"stop_hook_active":false}));
    assert_eq!(blocked["decision"], "block");
    f.stop();
    let delivery = block_on(f.store().claude_delivery(&f.ids[0]))
        .unwrap()
        .unwrap();
    let attempt = delivery.attempt.unwrap();
    let mut channel = Channel::new(&f.home);
    let receipt = ClaudeReceipt {
        message_id: f.ids[0].clone(),
        delivery_id: attempt.delivery_id,
        session_id: attempt.session_id,
    };
    let reply = channel.ack(&[receipt]);
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("pending sync")
    );
    channel.close();
    let dir = f.home.join("spool/acks");
    for n in 0..32 {
        fs::write(
            dir.join(format!("00000000000000000000-{n:03}.json")),
            b"broken",
        )
        .unwrap();
    }
    f.start();
    let end = Instant::now() + Duration::from_secs(6);
    while pending(&f.home, "acks") != 32 {
        assert!(
            Instant::now() < end,
            "valid ACK was starved by corrupt prefix"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
    f.stop();
    let s = f.store();
    assert_eq!(
        block_on(s.message(&f.ids[0])).unwrap().unwrap().state,
        DeliveryState::Confirmed
    );
}

#[test]
fn native_hook_stdout_backpressure_leaves_unknown_intent_and_never_resends() {
    let body = "x".repeat(MAX_MESSAGE_BYTES);
    let mut f = Fixture::with_body(1, Some(&body));
    f.start();
    let payload = hook_payload(&f.home, SESSION, "Stop", json!({"stop_hook_active":false}));
    let mut child = Command::new(BIN)
        .args(["hook", "Stop"])
        .env("AGEND_HOME", &f.home)
        .env("AGEND_INSTANCE", "claude")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let end = Instant::now() + Duration::from_secs(12);
    // Keep the real pipe unread, rather than mocking the helper's writer.
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= end {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("Stop helper exceeded its stdout deadline");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let result = child.wait_with_output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("stdout write timed out"));
    assert!(!result.stdout.is_empty());
    assert_eq!(f.hook("Stop", json!({"stop_hook_active":false})), json!({}));
    f.stop();
    let s = f.store();
    let m = block_on(s.message(&f.ids[0])).unwrap().unwrap();
    let d = block_on(s.claude_delivery(&f.ids[0])).unwrap().unwrap();
    assert!(d.outcome_unknown(&m));
    assert_eq!(m.state, DeliveryState::Queued);
}

#[test]
fn mcp_parse_and_schema_errors_do_not_confirm_or_break_the_next_native_request() {
    let f = Fixture::new(0);
    let mut channel = Channel::new(&f.home);
    writeln!(channel.input.as_mut().unwrap(), "broken JSON").unwrap();
    assert_eq!(channel.recv()["error"]["code"], -32700);
    channel.send(json!({"jsonrpc":"2.0","id":true,"method":"ping"}));
    assert_eq!(channel.recv()["error"]["code"], -32600);
    let mut request = ack_request(
        7,
        &[ClaudeReceipt {
            message_id: id(),
            delivery_id: id(),
            session_id: SESSION.into(),
        }],
    );
    request["params"]["arguments"]["extra"] = json!(true);
    channel.send(request);
    assert_eq!(channel.recv()["result"]["isError"], true);
    assert_eq!(pending(&f.home, "acks"), 0);
    channel.written_barrier();
    channel.close();
}

#[test]
fn stop_queue_is_not_starved_by_a_full_prefix_of_pending_steer_messages() {
    let mut f = Fixture::new(0);
    let queue_id = id();
    {
        let s = f.store();
        for n in 0..33 {
            block_on(s.claim_message(
                &NewMessage {
                    id: if n == 32 { queue_id.clone() } else { id() },
                    from_instance: "operator".into(),
                    to_instance: "claude".into(),
                    task_id: None,
                    body: format!("prefix-{n}"),
                    level: if n == 32 {
                        BusyLevel::Queue
                    } else {
                        BusyLevel::Steer
                    },
                },
                0,
            ))
            .unwrap();
        }
    }
    f.start();
    let blocked = f.hook("Stop", json!({"stop_hook_active":false}));
    assert_eq!(blocked["decision"], "block");
    assert!(blocked["reason"].as_str().unwrap().contains(&queue_id));
    assert!(!blocked["reason"].as_str().unwrap().contains("prefix-0"));
    f.stop();
    let s = f.store();
    assert_eq!(
        block_on(s.message(&queue_id)).unwrap().unwrap().state,
        DeliveryState::Sent
    );
}

#[test]
fn delayed_live_session_start_cannot_undo_a_newer_busy_hook() {
    let mut f = Fixture::new(1);
    f.start();
    let old = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 1000;
    f.hook("UserPromptSubmit", json!({"prompt":"busy"}));
    let payload = hook_payload(
        &f.home,
        SESSION,
        "SessionStart",
        json!({"source":"startup"}),
    );
    assert!(
        f.rpc(ClaudeOperation::Hook {
            event_id: id(),
            session_id: SESSION.into(),
            event: "SessionStart".into(),
            payload: payload.to_string(),
            occurred_at_unix_ms: old,
            replayed: false,
        })
        .committed
    );
    std::thread::sleep(Duration::from_millis(5100));
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty()
    );
}
