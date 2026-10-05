//! Native daemon/holder/helper processes. No real Claude or model calls.
#![cfg(unix)]
#[path = "common/claude_contract.rs"]
mod claude_contract;
#[path = "common/claude_control_loss.rs"]
mod claude_control_loss;
#[path = "common/claude_pipeline.rs"]
mod claude_pipeline;
#[path = "common/claude_startup.rs"]
mod claude_startup;
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

fn operator_request(home: &Path, caller: Option<&str>, request: ClientRequest) -> ClientResponse {
    use agend_testkit::fake_daemon::ProbeClient;
    let (mut client, _) = ProbeClient::hello(&home.join(DAEMON_SOCKET), caller).unwrap();
    client.request(&request).unwrap()
}
fn unknown_items(home: &Path) -> Vec<AttentionRequiredData> {
    let response = operator_request(
        home,
        None,
        ClientRequest::GetFleet {
            data: RequestIdData { request_id: id() },
        },
    );
    let ClientResponse::Fleet { data } = response else {
        panic!("{response:?}")
    };
    data.fleet
        .attention
        .into_iter()
        .filter(|a| {
            a.attention_id
                .as_deref()
                .is_some_and(|s| s.starts_with("claude-delivery:"))
        })
        .collect()
}
fn wait_unknown(home: &Path, count: usize) -> Vec<AttentionRequiredData> {
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        let items = unknown_items(home);
        if items.len() == count {
            return items;
        }
        assert!(
            Instant::now() < end,
            "expected {count} unknown deliveries, got {}",
            items.len()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn resolve_unknown(
    home: &Path,
    message: &str,
    caller: Option<&str>,
    action: AttentionAction,
) -> ClientResponse {
    operator_request(
        home,
        caller,
        ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: id(),
                attention_id: format!("claude-delivery:{message}"),
                action,
                note: Some("operator chooses to end this unknown delivery".into()),
            },
        },
    )
}

fn native_driver_send(home: &Path, message: &str, body: &str, to: &str) -> ClientResponse {
    operator_request(
        home,
        Some("claude"),
        ClientRequest::Command {
            data: ClientCommandData {
                request_id: id(),
                command: AgentCommand::Send {
                    to: to.into(),
                    message: body.into(),
                    level: Some(MessageLevel::Queue),
                    message_id: Some(message.into()),
                },
            },
        },
    )
}
fn first_native_driver_write(f: &Fixture, message: &str, body: &str) -> ClaudeReceipt {
    f.hook("SessionStart", json!({"source":"startup"}));
    std::thread::sleep(Duration::from_millis(5100));
    // Report live stable idle before delivery; the real Driver must observe
    // the helper's actual Written transaction, not a synthetic store writer.
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty()
    );
    let mut channel = Channel::new(&f.home);
    let home = f.home.clone();
    let message_id = message.to_owned();
    let content = body.to_owned();
    let sender =
        std::thread::spawn(move || native_driver_send(&home, &message_id, &content, "claude"));
    let notification = loop {
        let v = channel.recv();
        if v["method"] == "notifications/claude/channel" {
            break v;
        }
    };
    assert_eq!(
        notification["params"]["content"],
        format!("From: claude\n\n{body}")
    );
    let receipt = Channel::receipt(&notification);
    assert_eq!(receipt.message_id, message);
    channel.written_barrier();
    assert!(
        matches!(sender.join().unwrap(), ClientResponse::CommandResult {data} if data.result == CommandResult::Accepted)
    );
    channel.close();
    receipt
}

#[test]
fn actual_driver_routes_native_content_once_across_four_daemons_and_new_home_is_independent() {
    use agend_core::traits::{Driver, DriverEventKind};
    use agend_daemon::driver::claude::ClaudeDriver;
    use std::sync::Arc;
    let message = id();
    let body = "Driver native body: 繁中 é\nsecond line";
    let mut f = Fixture::new(0);
    f.start();
    let receipt = first_native_driver_write(&f, &message, body);
    f.stop();
    {
        let store = f.store();
        assert_eq!(
            block_on(store.message(&message)).unwrap().unwrap().state,
            DeliveryState::Sent
        );
        assert_eq!(block_on(store.messages_to("claude")).unwrap().len(), 1);
        assert!(
            block_on(store.claude_delivery(&message))
                .unwrap()
                .unwrap()
                .attempt
                .unwrap()
                .confirmed_at_unix_ms
                .is_none()
        );
    }
    for boot in 2..=4 {
        f.start();
        assert!(
            matches!(native_driver_send(&f.home, &message, body, "claude"), ClientResponse::CommandResult {data} if data.result == CommandResult::Accepted)
        );
        assert!(
            matches!(native_driver_send(&f.home, &message, "different content", "claude"), ClientResponse::Error {data} if data.code == error_code::INVALID_REQUEST)
        );
        assert!(
            matches!(native_driver_send(&f.home, &id(), body, "missing"), ClientResponse::Error {data} if data.code == error_code::UNKNOWN_INSTANCE)
        );
        assert!(
            f.rpc(ClaudeOperation::Poll {
                session_id: SESSION.into()
            })
            .messages
            .is_empty()
        );
        if boot == 2 {
            let mut channel = Channel::new(&f.home);
            assert_ne!(
                channel.ack(std::slice::from_ref(&receipt))["result"]["isError"],
                true
            );
            channel.close();
            assert_eq!(f.hook("Stop", json!({"stop_hook_active":false})), json!({}));
        }
        f.stop();
        let store = Arc::new(f.store());
        let driver = ClaudeDriver::new(store.clone());
        let events = block_on(driver.events("claude", None)).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e.kind, DriverEventKind::MessageConfirmed { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e.kind, DriverEventKind::TurnCompleted { .. }))
                .count(),
            1
        );
        for (index, event) in events.iter().enumerate() {
            assert_eq!(
                block_on(driver.events("claude", Some(&event.cursor))).unwrap(),
                events[index + 1..]
            );
        }
        let attempt = block_on(store.claude_delivery(&message))
            .unwrap()
            .unwrap()
            .attempt
            .unwrap();
        assert_eq!(attempt.delivery_id, receipt.delivery_id);
        assert_eq!(block_on(store.messages_to("claude")).unwrap().len(), 1);
    }
    let mut control = Fixture::new(0);
    control.start();
    let independent = first_native_driver_write(&control, &message, body);
    assert_ne!(independent.delivery_id, receipt.delivery_id);
    control.stop();
    assert_eq!(
        block_on(control.store().message(&message))
            .unwrap()
            .unwrap()
            .state,
        DeliveryState::Sent
    );
}

#[test]
fn native_ack_accepts_the_original_opaque_pipeline_dispatch_identity() {
    let mut f = Fixture::new(0);
    let message = "dispatch:t-1/work/1";
    block_on(f.store().claim_message(
        &NewMessage {
            id: message.into(),
            from_instance: "daemon".into(),
            to_instance: "claude".into(),
            task_id: None,
            body: "Work ticket t-1/work/1".into(),
            level: BusyLevel::Queue,
        },
        0,
    ))
    .unwrap();
    f.start();
    f.hook("SessionStart", json!({"source":"startup"}));
    let mut channel = Channel::new(&f.home);
    let notification = loop {
        let v = channel.recv();
        if v["method"] == "notifications/claude/channel" {
            break v;
        }
    };
    let receipt = Channel::receipt(&notification);
    assert_eq!(receipt.message_id, message);
    channel.written_barrier();
    let ack = channel.ack(std::slice::from_ref(&receipt));
    assert!(ack["error"].is_null(), "opaque dispatch id rejected: {ack}");
    assert_ne!(ack["result"]["isError"], true);
    channel.close();
    f.stop();
    assert_eq!(
        block_on(f.store().message(message)).unwrap().unwrap().state,
        DeliveryState::Confirmed
    );
}

#[test]
fn unknown_delivery_attention_pages_retains_and_only_operator_abandonment_terminates() {
    let mut f = Fixture::new(43);
    let old = agend_daemon::log::now_unix_ms() - 30_000;
    let mut receipts = Vec::new();
    {
        let s = f.store();
        block_on(s.set_instance_status("claude", InstanceStatus::Running)).unwrap();
        // Unattempted prefix/suffix rows must not consume the bounded page.
        for message in &f.ids[1..41] {
            let receipt = ClaudeReceipt {
                message_id: message.clone(),
                delivery_id: id(),
                session_id: SESSION.into(),
            };
            block_on(s.reserve_claude_delivery(
                NewClaudeDelivery {
                    message_id: message.clone(),
                    delivery_id: receipt.delivery_id.clone(),
                    instance_id: "claude".into(),
                    session_id: SESSION.into(),
                    route: ClaudeRoute::Channel,
                },
                old,
            ))
            .unwrap();
            receipts.push(receipt);
        }
    }
    f.start();
    let items = wait_unknown(&f.home, 40);
    assert!(
        items.iter().all(
            |a| a.actions == [AttentionAction::Abandon] && a.waiting_since_unix_ms == Some(old)
        )
    );
    let refused = resolve_unknown(&f.home, &f.ids[1], Some("claude"), AttentionAction::Abandon);
    assert!(matches!(refused, ClientResponse::Error {data} if data.code == error_code::FORBIDDEN));
    let refused = resolve_unknown(&f.home, &f.ids[1], None, AttentionAction::Retry);
    assert!(
        matches!(refused, ClientResponse::Error {data} if data.code == error_code::UNKNOWN_ATTENTION)
    );
    wait_unknown(&f.home, 40);
    let accepted = resolve_unknown(&f.home, &f.ids[1], None, AttentionAction::Abandon);
    assert!(
        matches!(accepted, ClientResponse::CommandResult {data} if data.result == CommandResult::Accepted)
    );
    // A valid late ACK wins over a stale attention snapshot. It must never
    // become failed even if the operator resolves before the next refresh.
    f.rpc(ClaudeOperation::Ack {
        receipts: vec![receipts[1].clone()],
    });
    assert!(matches!(
        resolve_unknown(&f.home, &f.ids[2], None, AttentionAction::Abandon),
        ClientResponse::Error { .. }
    ));
    wait_unknown(&f.home, 38);
    f.stop();
    for _ in 0..3 {
        f.start();
        wait_unknown(&f.home, 38);
        f.stop();
    }
    let s = f.store();
    let ended = block_on(s.claude_delivery(&f.ids[1])).unwrap().unwrap();
    assert_eq!(
        ended.abandonment_reason.as_deref(),
        Some("operator chooses to end this unknown delivery")
    );
    assert_eq!(
        block_on(s.message(&f.ids[1])).unwrap().unwrap().state,
        DeliveryState::Failed
    );
    assert_eq!(
        block_on(s.message(&f.ids[2])).unwrap().unwrap().state,
        DeliveryState::Confirmed
    );
    for receipt in &receipts[2..] {
        let m = block_on(s.message(&receipt.message_id)).unwrap().unwrap();
        let d = block_on(s.claude_delivery(&receipt.message_id))
            .unwrap()
            .unwrap();
        assert!(d.outcome_unknown(&m));
        assert_eq!(d.attempt.unwrap().delivery_id, receipt.delivery_id);
    }
    assert!(
        block_on(s.message(&f.ids[0]))
            .unwrap()
            .unwrap()
            .attempted_at_unix_ms
            .is_none()
    );
    assert!(
        block_on(s.message(&f.ids[42]))
            .unwrap()
            .unwrap()
            .attempted_at_unix_ms
            .is_none()
    );
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
        Self::with_script(
            messages,
            body,
            "/bin/sh",
            "printf 'native ready\\r\\n'; exec sleep 600",
        )
    }
    fn with_script(messages: usize, body: Option<&str>, program: &str, script: &str) -> Self {
        // Actual holder/parser producer renders the recorded main UI, scoped
        // to this fixture's canonical cwd. Synthetic 'ready' is no evidence.
        let ready = include_str!(
            "../../agend-core/tests/fixtures/screens/claude-2.1.284-main-100x24-0.txt"
        )
        .trim_end()
        .replace("<rec>/h1/workspace/g12-startup-capture", "$PWD");
        let producer = format!(
            "cat <<AGEND_READY_FRAME | awk '{{printf \"%s\\r\\n\",$0}}'\n{ready}\nAGEND_READY_FRAME\n"
        );
        let script = script.replace("printf 'native ready\\r\\n';", &producer);
        let lab = lab::Lab::with_prefix(Path::new(BIN), "g12b");
        let home = lab.home(0);
        let store = SqliteStore::open(&home, 0).unwrap();
        block_on(store.add_instance(&Instance {
            id: "claude".into(),
            backend: Backend::Claude,
            program: program.into(),
            args: vec!["-c".into(), script, "fake-claude".into()],
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
        assert!(
            daemon
                .log
                .iter()
                .any(|line| line.contains("holder pid=") || line.contains("reconnected to holder")),
            "Claude fixture never started: {}",
            daemon.log.join("\n")
        );
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
        Self::native_hook(&self.home, event, extra)
    }
    fn native_hook(home: &Path, event: &str, extra: Value) -> Value {
        Self::hook_as(home, "claude", SESSION, event, extra)
    }
    fn hook_as(home: &Path, instance: &str, session: &str, event: &str, extra: Value) -> Value {
        let payload = hook_payload(home, session, event, extra);
        let mut child = Command::new(BIN)
            .args(["hook", event])
            .env("AGEND_HOME", home)
            .env("AGEND_INSTANCE", instance)
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
        Self::for_instance(home, "claude")
    }
    fn for_instance(home: &Path, instance: &str) -> Self {
        let mut child = Command::new(BIN)
            .args(["channel", "--instance", instance])
            .env("AGEND_HOME", home)
            .env("AGEND_INSTANCE", instance)
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
fn busy_steer_writes_one_esc_then_channels_without_stop_or_ack_and_skips_queue_prefix() {
    let mut f = Fixture::with_script(
        40,
        None,
        "/bin/bash",
        r#"stty raw min 1 time 0 -echo; printf 'native ready\r\n'; while IFS= read -r -n 1 byte; do printf '%d\n' "'$byte" >> "$AGEND_HOME/keys.log"; done"#,
    );
    let interrupt = id();
    let store = f.store();
    block_on(store.claim_message(
        &NewMessage {
            id: interrupt.clone(),
            from_instance: "operator".into(),
            to_instance: "claude".into(),
            task_id: None,
            body: "steer after interrupt: 完整內容 é".into(),
            level: BusyLevel::Steer,
        },
        1,
    ))
    .unwrap();
    drop(store);
    f.start();
    assert_eq!(
        f.hook("UserPromptSubmit", json!({"prompt":"busy native turn"})),
        json!({})
    );
    let before = Instant::now();
    let mut channel = Channel::new(&f.home);
    let delivered = channel.recv();
    assert!(
        before.elapsed() < Duration::from_secs(4),
        "busy interrupt waited for idle or Stop"
    );
    let receipt = Channel::receipt(&delivered);
    assert_eq!(receipt.message_id, interrupt);
    assert_eq!(
        delivered["params"]["content"],
        "From: operator\n\nsteer after interrupt: 完整內容 é"
    );
    channel.written_barrier();
    assert!(
        channel
            .output
            .recv_timeout(Duration::from_millis(700))
            .is_err(),
        "busy Queue was sent or interrupt replayed"
    );
    channel.close();
    f.stop();
    assert_eq!(fs::read_to_string(f.home.join("keys.log")).unwrap(), "27\n");
    let store = f.store();
    assert_eq!(
        block_on(store.message(&interrupt)).unwrap().unwrap().state,
        DeliveryState::Sent
    );
    assert!(
        block_on(store.claude_delivery(&interrupt))
            .unwrap()
            .unwrap()
            .attempt
            .unwrap()
            .confirmed_at_unix_ms
            .is_none()
    );
    for queued in &f.ids {
        assert_eq!(
            block_on(store.message(queued)).unwrap().unwrap().state,
            DeliveryState::Queued
        );
        assert!(
            block_on(store.claude_delivery(queued))
                .unwrap()
                .unwrap()
                .attempt
                .is_none()
        );
    }
    assert!(
        !block_on(store.driver_events_after(0, 1024))
            .unwrap()
            .iter()
            .any(|e| e.event.kind == "Stop")
    );
}

#[test]
fn busy_interrupt_keeps_queued_without_writing_or_stealing_operator_control() {
    use agend_core::protocol::terminal::{TerminalSize, TerminalViewport};
    use agend_testkit::fake_daemon::ProbeClient;
    let mut f = Fixture::with_script(
        0,
        None,
        "/bin/bash",
        r#"stty raw min 1 time 0 -echo; printf 'native ready\r\n'; while IFS= read -r -n 1 byte; do printf '%d\n' "'$byte" >> "$AGEND_HOME/keys.log"; done"#,
    );
    let message = id();
    let store = f.store();
    block_on(store.claim_message(
        &NewMessage {
            id: message.clone(),
            from_instance: "operator".into(),
            to_instance: "claude".into(),
            task_id: None,
            body: "interrupt".into(),
            level: BusyLevel::Interrupt,
        },
        0,
    ))
    .unwrap();
    drop(store);
    f.start();
    f.hook("UserPromptSubmit", json!({"prompt":"busy"}));
    let (mut owner, version) = ProbeClient::hello(&f.home.join(DAEMON_SOCKET), None).unwrap();
    assert_eq!(version, V1_5);
    owner
        .send(&ClientRequest::SubscribeTerminalFrames {
            data: TerminalSubscribeData {
                request_id: "owner-view".into(),
                instance_id: "claude".into(),
                viewport: TerminalViewport { top: None, rows: 1 },
            },
        })
        .unwrap();
    let Some(ClientResponse::TerminalFrame { data: frame }) =
        owner.recv_within(Duration::from_secs(5)).unwrap()
    else {
        panic!("owner frame")
    };
    owner
        .send(&ClientRequest::TerminalControl {
            data: ClientTerminalControlData {
                request_id: "take".into(),
                instance_id: "claude".into(),
                view_id: frame.view_id.clone(),
                generation: frame.frame.generation.clone(),
                operation: ClientTerminalOperation::Acquire {
                    size: TerminalSize {
                        rows: 24,
                        columns: 80,
                    },
                },
            },
        })
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    let attach = loop {
        assert!(Instant::now() < end, "owner grant deadline");
        if let Some(ClientResponse::TerminalControlAck { data }) = owner
            .recv_within(end.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            assert_eq!(data.request_id, "take");
            let TerminalControlState::Controlled { attach_id } = data.control else {
                panic!("owner refused")
            };
            break attach_id;
        }
    };
    for _ in 0..3 {
        assert!(
            f.rpc(ClaudeOperation::Poll {
                session_id: SESSION.into()
            })
            .messages
            .is_empty()
        );
    }
    assert!(
        !f.home.join("keys.log").exists(),
        "daemon wrote a key under human ownership"
    );
    // The original human's token still writes through the native PTY.
    owner
        .send(&ClientRequest::TerminalControl {
            data: ClientTerminalControlData {
                request_id: "human-marker".into(),
                instance_id: "claude".into(),
                view_id: frame.view_id,
                generation: frame.frame.generation,
                operation: ClientTerminalOperation::Input {
                    attach_id: attach,
                    bytes_base64: "eQ==".into(),
                },
            },
        })
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < end, "human marker response deadline");
        match owner
            .recv_within(end.saturating_duration_since(Instant::now()))
            .unwrap()
        {
            Some(ClientResponse::TerminalControlAck { data })
                if data.request_id == "human-marker" =>
            {
                break;
            }
            Some(ClientResponse::Error { data }) => panic!("human marker refused: {data:?}"),
            _ => (),
        }
    }
    let end = Instant::now() + Duration::from_secs(5);
    while !f.home.join("keys.log").exists() {
        assert!(Instant::now() < end, "human owner lost control");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fs::read_to_string(f.home.join("keys.log")).unwrap(),
        "121\n"
    );
    drop(owner);
    f.stop();
    let store = f.store();
    let m = block_on(store.message(&message)).unwrap().unwrap();
    assert_eq!(m.state, DeliveryState::Queued);
    assert!(m.attempted_at_unix_ms.is_none());
    assert!(
        block_on(store.claude_delivery(&message))
            .unwrap()
            .unwrap()
            .attempt
            .is_none()
    );
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
    let deadline = Instant::now() + Duration::from_secs(8);
    let push = loop {
        if let Some(push) = f
            .rpc(ClaudeOperation::Poll {
                session_id: SESSION.into(),
            })
            .messages
            .pop()
        {
            break push;
        }
        assert!(
            Instant::now() < deadline,
            "no native startup-gated delivery intent"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
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

#[test]
fn hook_publication_and_live_rpc_hold_one_lock_so_ingest_cannot_steal_busy() {
    use agend_testkit::contract::client::proxy::{Direction, Options, Proxy};
    use std::os::fd::AsRawFd;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let mut f = Fixture::new(1);
    f.start();
    f.hook("SessionStart", json!({"source":"startup"}));
    std::thread::sleep(Duration::from_millis(5100));
    let socket = f.home.join(DAEMON_SOCKET);
    let upstream = f.home.join("run/upstream.sock");
    fs::rename(&socket, &upstream).unwrap();
    let waiting = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let (seen, resume) = (waiting.clone(), release.clone());
    let proxy = Proxy::start_at(&socket, upstream.clone(), Options::rewrite(Arc::new(move |_, direction, line| {
        if direction == Direction::ToServer && matches!(serde_json::from_str::<ClientRequest>(&line),
            Ok(ClientRequest::Claude { data: ClaudeRequestData { operation: ClaudeOperation::Hook { ref event, .. }, .. } }) if event == "UserPromptSubmit") {
            seen.store(true, Ordering::SeqCst);
            let end = Instant::now() + Duration::from_secs(6);
            while !resume.load(Ordering::SeqCst) && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        vec![line]
    }))).unwrap();
    std::thread::scope(|scope| {
        let home = f.home.clone();
        let helper = scope.spawn(move || {
            Fixture::native_hook(&home, "UserPromptSubmit", json!({"prompt":"busy"}))
        });
        let end = Instant::now() + Duration::from_secs(5);
        while !waiting.load(Ordering::SeqCst) {
            assert!(Instant::now() < end, "native Hook RPC did not reach proxy");
            std::thread::sleep(Duration::from_millis(5));
        }
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(f.home.join("spool/lock"))
            .unwrap();
        let held = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0;
        std::thread::sleep(Duration::from_millis(1250));
        let retained = pending(&f.home, "hooks");
        release.store(true, Ordering::SeqCst);
        assert_eq!(helper.join().unwrap(), json!({}));
        assert!(held, "helper released publication lock before live RPC");
        assert_eq!(retained, 1, "daemon stole fresh hook as historical");
    });
    drop(proxy);
    fs::rename(upstream, socket).unwrap();
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty()
    );
}

#[test]
fn newer_published_historical_busy_invalidates_cached_idle_without_dispatch() {
    let mut f = Fixture::new(1);
    f.start();
    f.hook("SessionStart", json!({"source":"startup"}));
    std::thread::sleep(Duration::from_millis(5100));
    // The real disk envelope models a helper that died after publication.
    let payload = hook_payload(
        &f.home,
        SESSION,
        "UserPromptSubmit",
        json!({"prompt":"busy"}),
    );
    let record = ClaudePendingRecord {
        version: 1,
        request: ClaudeRequestData {
            request_id: id(),
            instance_id: "claude".into(),
            operation: ClaudeOperation::Hook {
                event_id: id(),
                session_id: SESSION.into(),
                event: "UserPromptSubmit".into(),
                payload: payload.to_string(),
                occurred_at_unix_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
                replayed: false,
            },
        },
    };
    let path = f
        .home
        .join("spool/hooks")
        .join(format!("00000000000000000002-{}.json", id()));
    let mut file = fs::File::create(path).unwrap();
    file.write_all(&serde_json::to_vec(&record).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    wait_empty(&f.home, "hooks");
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty()
    );
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn first_native_hook_unlock_occurs_after_busy_is_committed_live() {
    let mut f = Fixture::new(1);
    f.start();
    f.hook("SessionStart", json!({"source":"startup"}));
    std::thread::sleep(Duration::from_millis(5100));
    let library = f.home.join("unlock-probe.so");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude_unlock_probe.c");
    let mut compiler = Command::new("cc");
    #[cfg(target_os = "macos")]
    compiler.arg("-dynamiclib");
    #[cfg(target_os = "linux")]
    compiler.args(["-shared", "-fPIC"]);
    compiler.arg(source).arg("-o").arg(&library);
    #[cfg(target_os = "linux")]
    compiler.arg("-ldl");
    assert!(
        compiler.status().unwrap().success(),
        "native unlock probe compilation"
    );
    let marker = f.home.join("unlocked");
    let mut command = Command::new(BIN);
    command
        .args(["hook", "UserPromptSubmit"])
        .env("AGEND_HOME", &f.home)
        .env("AGEND_INSTANCE", "claude")
        .env("AGEND_PROBE_UNLOCK_PATH", &marker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "macos")]
    command.env("DYLD_INSERT_LIBRARIES", &library);
    #[cfg(target_os = "linux")]
    command.env("LD_PRELOAD", &library);
    let mut child = command.spawn().unwrap();
    let payload = hook_payload(
        &f.home,
        SESSION,
        "UserPromptSubmit",
        json!({"prompt":"busy"}),
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while !marker.exists() {
        if Instant::now() >= end || child.try_wait().unwrap().is_some() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("native first-unlock interposition did not pause helper");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // SIGSTOP happens only after the real syscall released flock. An old
    // two-acquisition helper now lets the real ingester steal its publication.
    std::thread::sleep(Duration::from_millis(1250));
    let poll = f.rpc(ClaudeOperation::Poll {
        session_id: SESSION.into(),
    });
    unsafe {
        libc::kill(child.id() as i32, libc::SIGCONT);
    }
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        poll.messages.is_empty(),
        "cached idle delivered after native busy hook"
    );
    f.stop();
    let s = f.store();
    let events = block_on(s.driver_events_after(0, 32)).unwrap();
    let busy = events
        .iter()
        .find(|e| e.event.kind == "UserPromptSubmit")
        .unwrap();
    assert!(
        !busy.event.replayed,
        "fresh busy hook was stolen by historical ingest"
    );
}
