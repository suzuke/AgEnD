//! `fake-codex-app-server` as a process: WebSocket JSON-RPC over a unix socket.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use agend_testkit::fake_agent::codex::{MAX_DIRECT_SOCKET_PATH, Probe, wait_until_listening};
use agend_testkit::tempdir::TempDir;
use serde_json::{Value, json};

struct Running {
    child: Child,
    socket: PathBuf,
}

impl Running {
    fn start(socket: &Path, turn_ms: u64) -> Running {
        let child = Command::new(env!("CARGO_BIN_EXE_fake-codex-app-server"))
            .args(["app-server", "--listen"])
            .arg(format!("unix://{}", socket.display()))
            .args(["--turn-ms", &turn_ms.to_string()])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        wait_until_listening(socket, Duration::from_secs(5)).unwrap();
        Running {
            child,
            socket: socket.to_path_buf(),
        }
    }

    fn probe(&self) -> Probe {
        let mut probe = Probe::connect(&self.socket).unwrap();
        probe
            .call("initialize", json!({"clientInfo": {"name": "test"}}))
            .unwrap();
        probe
    }

    /// Closes stdin and expects a clean exit that removes the socket.
    fn stop(mut self) {
        drop(self.child.stdin.take());
        let status = self.child.wait().unwrap();
        assert!(status.success(), "{status}");
        assert!(!self.socket.exists(), "socket left behind");
    }
}

fn input(text: &str) -> Value {
    json!([{"type": "text", "text": text}])
}

fn start_thread(probe: &mut Probe) -> String {
    let result = probe.call("thread/start", json!({})).unwrap();
    result["thread"]["id"].as_str().unwrap().to_owned()
}

#[test]
fn turn_start_ends_with_turn_completed_and_echoes_ids() {
    let dir = TempDir::new("cx").unwrap();
    let server = Running::start(&dir.path().join("s.sock"), 50);
    let mut probe = server.probe();
    let thread = start_thread(&mut probe);
    let started = probe
        .call(
            "turn/start",
            json!({"threadId": thread, "input": input("say hi")}),
        )
        .unwrap();
    let turn = started["turn"]["id"].clone();
    let message = probe.next_method("item/completed").unwrap();
    assert_eq!(message["params"]["item"]["type"], "userMessage");
    let reply = probe.next_method("item/completed").unwrap();
    assert_eq!(reply["params"]["item"]["text"], "fake reply: say hi");
    assert_eq!(reply["params"]["turnId"], turn);
    let completed = probe.next_method("turn/completed").unwrap();
    let params = &completed["params"];
    assert_eq!(params["threadId"], thread);
    assert_eq!(params["turn"]["id"], turn);
    assert_eq!(params["turn"]["status"], "completed");
    assert_eq!(params["turn"]["items"][0]["text"], "fake reply: say hi");
    server.stop();
}

#[test]
fn steer_interrupt_and_queue() {
    let dir = TempDir::new("cx").unwrap();
    let server = Running::start(&dir.path().join("s.sock"), 800);
    let mut probe = server.probe();
    let thread = start_thread(&mut probe);
    let turn = probe
        .call(
            "turn/start",
            json!({"threadId": thread, "input": input("first")}),
        )
        .unwrap()["turn"]["id"]
        .clone();
    let wrong = probe.call(
        "turn/steer",
        json!({"threadId": thread, "expectedTurnId": "turn-0", "input": input("x")}),
    );
    assert!(wrong.unwrap_err().contains("-32600"));
    let steered = probe
        .call(
            "turn/steer",
            json!({"threadId": thread, "expectedTurnId": turn, "input": input("steered")}),
        )
        .unwrap();
    assert_eq!(steered["turnId"], turn);
    probe
        .call(
            "thread/queue/add",
            json!({"threadId": thread, "clientUserMessageId": "m-2", "input": input("queued")}),
        )
        .unwrap();
    // Like codex 0.156.1 (transcripts/codex/busy.jsonl): the steer is its
    // own user message and reply inside the same turn.
    let mut replies = Vec::new();
    while replies.len() < 2 {
        let message = probe.next_method("item/completed").unwrap();
        if message["params"]["item"]["type"] == "agentMessage" {
            assert_eq!(message["params"]["turnId"], turn);
            replies.push(message["params"]["item"]["text"].clone());
        }
    }
    assert_eq!(replies, ["fake reply: first", "fake reply: steered"]);
    probe.next_method("turn/completed").unwrap();
    let queued = probe.next_method("turn/started").unwrap();
    let queued_turn = queued["params"]["turn"]["id"].clone();
    assert_ne!(queued_turn, turn, "a queued message runs as its own turn");
    probe
        .call(
            "turn/interrupt",
            json!({"threadId": thread, "turnId": queued_turn}),
        )
        .unwrap();
    let interrupted = probe.next_method("turn/completed").unwrap();
    assert_eq!(interrupted["params"]["turn"]["status"], "interrupted");
    server.stop();
}

#[test]
fn approval_request_waits_for_the_decision() {
    let dir = TempDir::new("cx").unwrap();
    let server = Running::start(&dir.path().join("s.sock"), 20);
    let mut probe = server.probe();
    let thread = start_thread(&mut probe);
    for (decision, expected) in [
        ("accept", "ran `cargo test`"),
        ("decline", "command not run: decline"),
    ] {
        probe
            .call(
                "turn/start",
                json!({"threadId": thread, "input": input("run: cargo test")}),
            )
            .unwrap();
        let request = probe
            .next_method("item/commandExecution/requestApproval")
            .unwrap();
        assert_eq!(request["params"]["command"], "/bin/zsh -lc 'cargo test'");
        assert_eq!(
            request["params"]["commandActions"][0]["command"],
            "cargo test"
        );
        probe
            .send(json!({"id": request["id"], "result": {"decision": decision}}))
            .unwrap();
        let reply = loop {
            let message = probe.next_method("item/completed").unwrap();
            if message["params"]["item"]["type"] == "agentMessage" {
                break message;
            }
        };
        assert_eq!(reply["params"]["item"]["text"], expected);
        probe.next_method("turn/completed").unwrap();
    }
    server.stop();
}

#[test]
fn only_resumed_threads_stream_to_a_new_connection() {
    let dir = TempDir::new("cx").unwrap();
    let server = Running::start(&dir.path().join("s.sock"), 20);
    let mut first = server.probe();
    let thread = start_thread(&mut first);
    drop(first);
    let mut second = server.probe();
    let missing = second.call("thread/resume", json!({"threadId": "thr-999"}));
    assert!(missing.is_err());
    second
        .call(
            "thread/resume",
            json!({"threadId": thread, "excludeTurns": true}),
        )
        .unwrap();
    second
        .call(
            "turn/start",
            json!({"threadId": thread, "input": input("again")}),
        )
        .unwrap();
    let completed = second.next_method("turn/completed").unwrap();
    assert_eq!(completed["params"]["threadId"], thread);
    assert!(
        second
            .call("thread/fork", json!({}))
            .unwrap_err()
            .contains("-32601")
    );
    server.stop();
}

#[test]
fn long_listen_path_is_a_symlink_to_a_short_socket() {
    let dir = TempDir::new("cx").unwrap();
    let deep = dir.path().join("x".repeat(60)).join("y".repeat(40));
    std::fs::create_dir_all(&deep).unwrap();
    let requested = deep.join("app-server.sock");
    assert!(requested.as_os_str().len() > MAX_DIRECT_SOCKET_PATH);
    let server = Running::start(&requested, 20);
    assert!(
        std::fs::symlink_metadata(&requested)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let real = std::fs::canonicalize(&requested).unwrap();
    assert!(
        real.as_os_str().len() <= MAX_DIRECT_SOCKET_PATH,
        "{}",
        real.display()
    );
    let temp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    assert_eq!(
        real.parent(),
        Some(temp.as_path()),
        "the short socket sits directly in the temp dir, with no directory to leave behind"
    );
    let mut probe = server.probe();
    start_thread(&mut probe);
    server.stop();
    assert!(!real.exists());
}

#[test]
fn a_short_listen_path_is_a_symlink_too() {
    // codex 0.156.1 binds under /private/tmp/codex-daemon-<uid>/ even for a
    // short --listen path (seen while recording); the fake does the same.
    let dir = TempDir::new("cx").unwrap();
    let requested = dir.path().join("s.sock");
    let server = Running::start(&requested, 20);
    assert!(
        std::fs::symlink_metadata(&requested)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    server.stop();
}
