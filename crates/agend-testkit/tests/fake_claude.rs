//! `fake-claude` as a process: hooks, Stop-hook queueing, Esc, MCP channel.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

use agend_testkit::fake_agent::claude::{ESC, IDLE_LINE, INTERRUPTED_LINE};
use agend_testkit::tempdir::TempDir;
use serde_json::Value;

/// Hook that logs every payload; the first `Stop` with
/// `stop_hook_active: false` blocks once while `queued` exists (D16).
const HOOK: &str = r#"payload=$(cat)
printf '%s\n' "$payload" >> hooks.log
case "$payload" in
  *'"hook_event_name":"Stop"'*'"stop_hook_active":false'*|*'"stop_hook_active":false'*'"hook_event_name":"Stop"'*)
    if [ -f queued ]; then rm queued; printf '%s' '{"decision":"block","reason":"queued: run the checks"}'; fi ;;
esac
"#;

/// Minimal MCP channel server: answers initialize, then pushes one message.
const CHANNEL: &str = r#"read -r _init
printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-06-18","capabilities":{"experimental":{"claude/channel":{}}},"serverInfo":{"name":"test-channel","version":"0"}}}'
read -r _initialized
printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/claude/channel","params":{"content":"from: reviewer-2 (your team)","meta":{"chat_id":"c-1","delivery_id":"d-1"}}}'
cat > /dev/null
"#;

fn project(with_channel: bool) -> TempDir {
    let dir = TempDir::new("cl").unwrap();
    let root = dir.path();
    std::fs::create_dir(root.join(".claude")).unwrap();
    std::fs::write(root.join("hook.sh"), HOOK).unwrap();
    let hooks: Value = serde_json::json!({"hooks": {
        "SessionStart": [{"hooks": [{"type": "command", "command": "sh hook.sh"}]}],
        "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "sh hook.sh"}]}],
        "Stop": [{"hooks": [{"type": "command", "command": "sh hook.sh"}]}],
    }});
    std::fs::write(root.join(".claude/settings.json"), hooks.to_string()).unwrap();
    if with_channel {
        std::fs::write(root.join("channel.sh"), CHANNEL).unwrap();
        let mcp =
            serde_json::json!({"mcpServers": {"agend": {"command": "sh", "args": ["channel.sh"]}}});
        std::fs::write(root.join(".mcp.json"), mcp.to_string()).unwrap();
    }
    dir
}

fn start(root: &Path, extra: &[&str]) -> (Child, BufReader<ChildStdout>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .args(extra)
        .args(["--turn-ms", "600"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let out = BufReader::new(child.stdout.take().unwrap());
    (child, out)
}

/// Lines up to and including the next idle line.
fn until_idle(out: &mut BufReader<ChildStdout>) -> Vec<String> {
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        assert!(
            out.read_line(&mut line).unwrap() > 0,
            "exited early: {lines:?}"
        );
        let line = line.trim_end().to_owned();
        let idle = line == IDLE_LINE;
        lines.push(line);
        if idle {
            return lines;
        }
    }
}

fn hook_log(root: &Path) -> Vec<Value> {
    std::fs::read_to_string(root.join("hooks.log"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn events(log: &[Value]) -> Vec<String> {
    log.iter()
        .map(|p| {
            let name = p["hook_event_name"].as_str().unwrap().to_owned();
            match p["stop_hook_active"].as_bool() {
                Some(active) => format!("{name}(active={active})"),
                None => name,
            }
        })
        .collect()
}

#[test]
fn stop_hook_block_runs_one_more_turn_and_esc_skips_stop() {
    let dir = project(false);
    let root = dir.path();
    let (mut child, mut out) = start(root, &["--session-id", "s-1"]);
    until_idle(&mut out);
    std::fs::write(root.join("queued"), "").unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    stdin.write_all(b"fix the bug\r").unwrap();
    let lines = until_idle(&mut out);
    assert_eq!(
        lines,
        [
            "> fix the bug",
            "⏺ fake reply: fix the bug",
            "Stop hook error: queued: run the checks",
            "> queued: run the checks",
            "⏺ fake reply: queued: run the checks",
            IDLE_LINE
        ]
    );
    stdin.write_all(b"long task\r").unwrap();
    stdin.write_all(&[ESC]).unwrap();
    let lines = until_idle(&mut out);
    assert_eq!(lines, ["> long task", INTERRUPTED_LINE, IDLE_LINE]);
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    let log = hook_log(root);
    assert_eq!(
        events(&log),
        [
            "SessionStart",
            "UserPromptSubmit",
            "Stop(active=false)",
            "Stop(active=true)",
            "UserPromptSubmit"
        ],
        "Esc must not fire Stop"
    );
    assert_eq!(log[0]["source"], "startup");
    assert_eq!(log[0]["session_id"], "s-1");
    assert_eq!(log[1]["prompt"], "fix the bug");
}

#[test]
fn channel_messages_become_wrapped_prompts() {
    let dir = project(true);
    let root = dir.path();
    let (mut child, mut out) = start(
        root,
        &[
            "--resume",
            "s-9",
            "--dangerously-load-development-channels",
            "server:agend",
        ],
    );
    let mut lines = until_idle(&mut out);
    if !lines.iter().any(|l| l.starts_with("> <channel")) {
        lines = until_idle(&mut out);
    }
    let wrapped = "<channel source=\"agend\" chat_id=\"c-1\" delivery_id=\"d-1\">\nfrom: reviewer-2 (your team)\n</channel>";
    assert!(
        lines.contains(&format!("> {}", wrapped.replace('\n', "\\n"))),
        "{lines:?}"
    );
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    let log = hook_log(root);
    assert_eq!(log[0]["source"], "resume");
    let submitted: Vec<&Value> = log
        .iter()
        .filter(|p| p["hook_event_name"] == "UserPromptSubmit")
        .collect();
    assert_eq!(submitted[0]["prompt"], wrapped);
}

#[test]
fn unknown_channel_server_fails_like_claude() {
    let dir = project(false);
    std::fs::write(dir.path().join(".mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_fake-claude"))
        .args(["--dangerously-load-development-channels", "server:agend"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no MCP server configured with that name")
    );
}
