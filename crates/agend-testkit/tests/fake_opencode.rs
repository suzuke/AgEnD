//! `fake-opencode-serve` as a process: HTTP + server-sent events.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use agend_testkit::fake_agent::http::{EventStream, call};
use serde_json::{Value, json};

struct Running {
    child: Child,
    port: u16,
}

impl Running {
    fn start(turn_ms: u64) -> Running {
        let mut child = Command::new(env!("CARGO_BIN_EXE_fake-opencode-serve"))
            .args(["serve", "--hostname", "127.0.0.1", "--port", "0"])
            .args(["--turn-ms", &turn_ms.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let port = line
            .trim()
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("no port in {line:?}"));
        Running { child, port }
    }

    fn json(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let body = body.map(|b| b.to_string());
        let (status, text) = call(self.port, method, path, body.as_deref()).unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    fn stop(mut self) {
        drop(self.child.stdin.take());
        assert!(self.child.wait().unwrap().success());
    }
}

fn prompt(text: &str) -> Value {
    json!({"parts": [{"type": "text", "text": text}]})
}

fn event_type(event: &str) -> String {
    serde_json::from_str::<Value>(event).unwrap()["type"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn sync_prompt_streams_events_and_answers() {
    let server = Running::start(30);
    assert_eq!(
        server.json("GET", "/global/health", None).1["healthy"],
        true
    );
    let session = server.json("POST", "/session", Some(json!({}))).1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut events = EventStream::open(server.port).unwrap();
    assert_eq!(
        event_type(&events.next_event().unwrap()),
        "server.connected"
    );
    let (status, reply) = server.json(
        "POST",
        &format!("/session/{session}/message"),
        Some(prompt("hello")),
    );
    assert_eq!(status, 200);
    assert_eq!(reply["info"]["role"], "assistant");
    assert_eq!(reply["parts"][0]["text"], "fake reply: hello");
    let kinds: Vec<String> = (0..7)
        .map(|_| event_type(&events.next_event().unwrap()))
        .collect();
    assert_eq!(
        kinds,
        [
            "message.updated",
            "session.status",
            "message.updated",
            "message.part.updated",
            "message.updated",
            "session.status",
            "session.idle"
        ]
    );
    assert_eq!(server.json("GET", "/session/ses_missing", None).0, 404);
    server.stop();
}

#[test]
fn prompt_async_queues_and_abort_marks_the_message() {
    let server = Running::start(800);
    let session = server.json("POST", "/session", Some(json!({}))).1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("/session/{session}/prompt_async");
    assert_eq!(server.json("POST", &path, Some(prompt("A"))).0, 204);
    assert_eq!(server.json("POST", &path, Some(prompt("B"))).0, 204);
    let status = server.json("GET", "/session/status", None).1;
    assert_eq!(status[&session]["type"], "busy");
    let (code, body) = server.json(
        "POST",
        &format!("/session/{session}/abort"),
        Some(json!({})),
    );
    assert_eq!((code, body), (200, json!(true)));
    let messages = server
        .json("GET", &format!("/session/{session}/message"), None)
        .1;
    assert_eq!(messages[1]["info"]["error"]["name"], "MessageAbortedError");
    let (code, reply) = server.json(
        "POST",
        &format!("/session/{session}/message"),
        Some(prompt("C")),
    );
    assert_eq!(code, 200);
    assert_eq!(reply["parts"][0]["text"], "fake reply: C");
    let messages = server
        .json("GET", &format!("/session/{session}/message"), None)
        .1;
    let texts: Vec<&str> = messages
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["parts"][0]["text"].as_str())
        .collect();
    assert_eq!(texts, ["A", "B", "fake reply: B", "C", "fake reply: C"]);
    assert_eq!(server.json("GET", "/session/status", None).1, json!({}));
    server.stop();
}
