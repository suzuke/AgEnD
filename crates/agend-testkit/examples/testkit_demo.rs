//! Gate 2 walkthrough (`cargo xtask accept testkit`): start each fake agent
//! binary, show one exchange, stop it cleanly; talk to the fake daemon; run
//! every contract suite against the fakes.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::Duration;

use agend_core::protocol::client::{
    AgentCommand, ClientCommandData, ClientRequest, ClientResponse, ResultIdentity,
};
use agend_testkit::contract::run_all_fakes;
use agend_testkit::fake_agent::claude::{ESC, IDLE_LINE};
use agend_testkit::fake_agent::codex::{Probe, wait_until_listening};
use agend_testkit::fake_agent::http::{EventStream, call};
use agend_testkit::fake_agent::{CLAUDE_BIN, CODEX_BIN, OPENCODE_BIN, locate};
use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};
use agend_testkit::tempdir::TempDir;
use serde_json::{Value, json};

type Step = Result<(), String>;
type Demo = (&'static str, fn() -> Step);

fn main() -> ExitCode {
    let steps: [Demo; 5] = [
        ("fake codex app-server", codex),
        ("fake opencode serve", opencode),
        ("fake claude with hooks", claude),
        ("fake daemon (client protocol v1)", daemon),
        ("contract suites", contracts),
    ];
    for (title, step) in steps {
        println!("\n== {title} ==");
        if let Err(e) = step() {
            println!("FAILED: {e}");
            return ExitCode::FAILURE;
        }
    }
    println!("\ntestkit demo: all fake agents exited cleanly; all contract suites pass");
    ExitCode::SUCCESS
}

fn spawn(binary: &str, args: &[&str], cwd: Option<&Path>) -> Result<Child, String> {
    let path = locate(binary)?;
    let mut command = Command::new(&path);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let child = command
        .spawn()
        .map_err(|e| format!("start {binary}: {e}"))?;
    println!("started {binary} {} (pid {})", args.join(" "), child.id());
    Ok(child)
}

/// Closes stdin (the fakes' shutdown signal) and requires exit status 0.
fn stop(mut child: Child, binary: &str) -> Step {
    drop(child.stdin.take());
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("{binary} exited with {status}"));
    }
    println!("{binary} exited cleanly ({status})");
    Ok(())
}

fn codex() -> Step {
    let dir = TempDir::new("demo-cx").map_err(|e| e.to_string())?;
    let socket = dir.path().join("app.sock");
    let listen = format!("unix://{}", socket.display());
    let child = spawn(
        CODEX_BIN,
        &["app-server", "--listen", &listen, "--turn-ms", "50"],
        None,
    )?;
    wait_until_listening(&socket, Duration::from_secs(5))?;
    let mut probe = Probe::connect(&socket)?;
    let mut show = |method: &str, params: Value| -> Result<Value, String> {
        println!("-> {method} {params}");
        let result = probe.call(method, params)?;
        println!("<- result {result}");
        Ok(result)
    };
    show(
        "initialize",
        json!({"clientInfo": {"name": "testkit-demo"}}),
    )?;
    let thread = show("thread/start", json!({}))?["thread"]["id"].clone();
    let input = json!([{"type": "text", "text": "say hello"}]);
    show("turn/start", json!({"threadId": thread, "input": input}))?;
    loop {
        let message = probe.next_message()?;
        println!(
            "<- {} {}",
            message["method"].as_str().unwrap_or("?"),
            message["params"]
        );
        if message["method"] == "turn/completed" {
            break;
        }
    }
    stop(child, CODEX_BIN)
}

fn opencode() -> Step {
    let mut child = spawn(
        OPENCODE_BIN,
        &["serve", "--port", "0", "--turn-ms", "50"],
        None,
    )?;
    let mut line = String::new();
    let stdout = child.stdout.take().ok_or("no stdout")?;
    BufReader::new(stdout)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    print!("{line}");
    let port: u16 = line
        .trim()
        .rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or("no port")?;
    let mut events = EventStream::open(port).map_err(|e| e.to_string())?;
    let (_, session) = call(port, "POST", "/session", Some("{}")).map_err(|e| e.to_string())?;
    println!("-> POST /session\n<- 200 {session}");
    let id = serde_json::from_str::<Value>(&session).map_err(|e| e.to_string())?["id"]
        .as_str()
        .ok_or("no session id")?
        .to_owned();
    let body = json!({"parts": [{"type": "text", "text": "say hello"}]}).to_string();
    let (status, _) = call(
        port,
        "POST",
        &format!("/session/{id}/prompt_async"),
        Some(&body),
    )
    .map_err(|e| e.to_string())?;
    println!("-> POST /session/{id}/prompt_async {body}\n<- {status}");
    loop {
        let event = events.next_event().map_err(|e| e.to_string())?;
        println!("<- event {event}");
        if event.contains("\"session.idle\"") {
            break;
        }
    }
    stop(child, OPENCODE_BIN)
}

const HOOK: &str = r#"payload=$(cat)
printf '%s\n' "$payload" >> hooks.log
case "$payload" in *'"stop_hook_active":false'*)
  [ -f queued ] && rm queued && printf '%s' '{"decision":"block","reason":"queued message from dev-2: please also update the README"}' ;;
esac
exit 0
"#;

const CHANNEL: &str = r#"read -r _init
printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-06-18","capabilities":{"experimental":{"claude/channel":{}}},"serverInfo":{"name":"demo-channel","version":"0"}}}'
read -r _initialized
printf '%s\n' '{"jsonrpc":"2.0","method":"notifications/claude/channel","params":{"content":"from: reviewer-2 (your team) . task T-1 . request: summarize the diff","meta":{"delivery_id":"d-1"}}}'
cat > /dev/null
"#;

fn claude() -> Step {
    let dir = TempDir::new("demo-cl").map_err(|e| e.to_string())?;
    let root = dir.path();
    let write =
        |name: &str, text: &str| std::fs::write(root.join(name), text).map_err(|e| e.to_string());
    std::fs::create_dir(root.join(".claude")).map_err(|e| e.to_string())?;
    write("hook.sh", HOOK)?;
    write("channel.sh", CHANNEL)?;
    write("queued", "")?;
    let hook = json!([{"hooks": [{"type": "command", "command": "sh hook.sh"}]}]);
    let settings = json!({"hooks": {"SessionStart": hook, "UserPromptSubmit": hook, "Stop": hook}});
    write(".claude/settings.json", &settings.to_string())?;
    write(
        ".mcp.json",
        &json!({"mcpServers": {"agend": {"command": "sh", "args": ["channel.sh"]}}}).to_string(),
    )?;
    let args = [
        "--dangerously-load-development-channels",
        "server:agend",
        "--turn-ms",
        "500",
    ];
    let mut child = spawn(CLAUDE_BIN, &args, Some(root))?;
    let mut out = BufReader::new(child.stdout.take().ok_or("no stdout")?);
    let next_idle = |out: &mut BufReader<_>| -> Step {
        loop {
            let mut line = String::new();
            if out.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("fake-claude exited early".into());
            }
            print!("   {line}");
            if line.trim_end() == IDLE_LINE {
                return Ok(());
            }
        }
    };
    next_idle(&mut out)?; // startup
    next_idle(&mut out)?; // channel message, Stop hook block, continuation
    let stdin = child.stdin.as_mut().ok_or("no stdin")?;
    println!("-> type \"long task\" + Enter, then Esc");
    stdin.write_all(b"long task\r").map_err(|e| e.to_string())?;
    stdin.write_all(&[ESC]).map_err(|e| e.to_string())?;
    next_idle(&mut out)?;
    stop(child, CLAUDE_BIN)?;
    println!("hook invocations (hooks.log):");
    let log = std::fs::read_to_string(root.join("hooks.log")).map_err(|e| e.to_string())?;
    for line in log.lines() {
        let payload: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let mut detail = String::new();
        if let Some(active) = payload["stop_hook_active"].as_bool() {
            detail = format!(" stop_hook_active={active}");
        }
        if let Some(source) = payload["source"].as_str() {
            detail = format!(" source={source}");
        }
        if let Some(prompt) = payload["prompt"].as_str() {
            detail = format!(" prompt={:?}", prompt.lines().next().unwrap_or_default());
        }
        println!(
            "   {}{detail}",
            payload["hook_event_name"].as_str().unwrap_or("?")
        );
    }
    Ok(())
}

fn daemon() -> Step {
    let daemon = FakeDaemon::start().map_err(|e| e.to_string())?;
    daemon.assign(
        "T-1",
        ResultIdentity {
            stage_id: "work".into(),
            attempt: 2,
        },
    );
    let mut client = ProbeClient::connect(daemon.socket_path()).map_err(|e| e.to_string())?;
    let mut exchange = |request: ClientRequest| -> Result<ClientResponse, String> {
        println!(
            "-> {}",
            serde_json::to_string(&request).map_err(|e| e.to_string())?
        );
        let response = client.request(&request).map_err(|e| e.to_string())?;
        println!(
            "<- {}",
            serde_json::to_string(&response).map_err(|e| e.to_string())?
        );
        Ok(response)
    };
    exchange(ClientRequest::hello())?;
    for (request_id, attempt) in [("r-1", 1), ("r-2", 2)] {
        let command = AgentCommand::Done {
            task_id: "T-1".into(),
            identity: Some(ResultIdentity {
                stage_id: "work".into(),
                attempt,
            }),
        };
        exchange(ClientRequest::Command {
            data: ClientCommandData {
                request_id: request_id.into(),
                command,
            },
        })?;
    }
    println!("(attempt 1 is stale and changes nothing; attempt 2 is the current one)");
    Ok(())
}

fn contracts() -> Step {
    let reports = run_all_fakes();
    for report in &reports {
        println!("{report}");
    }
    let failed = reports.iter().filter(|r| !r.all_passed()).count();
    if failed > 0 {
        return Err(format!("{failed} contract suite(s) failed"));
    }
    Ok(())
}
