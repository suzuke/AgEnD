//! Focused 2.1.284 ACK/Stop fixture, separate from the historical fake.
//! MCP and hook shapes come from the shared native producer and the retained
//! `ack-stop-outcome.json` capture. The initial screen replays the recorded
//! Ready frame through the real holder parser. This does not simulate dialogs,
//! model reasoning, busy routing, permissions or authentication.
use super::*;
use agend_core::protocol::client::ClaudeReceipt;

pub fn main(args: impl IntoIterator<Item = String>) -> ExitCode {
    let args: Vec<_> = args.into_iter().collect();
    if args == ["--version"] {
        println!("2.1.284 (Claude Code)");
        return ExitCode::SUCCESS;
    }
    let result = Args::parse(
        args,
        &[
            "--session-id",
            "--resume",
            "--settings",
            "--setting-sources",
            "--permission-mode",
            "--dangerously-load-development-channels",
        ],
        &[],
        &[],
    )
    .and_then(|args| run(&args));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fake-claude-cli: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let session = args
        .get("--session-id")
        .or(args.get("--resume"))
        .ok_or("missing session identity")?;
    let settings = args.get("--settings").ok_or("missing hook settings")?;
    let hooks = load_hooks(Path::new(settings))?;
    let hook = |event: &str, extra: Value| {
        let payload = hook_payload(&cwd, session, event, extra).to_string();
        for (_, command) in hooks.iter().filter(|(name, _)| name == event) {
            let _ = run_hook(&cwd, command, &payload);
        }
    };
    let name = args
        .get("--dangerously-load-development-channels")
        .and_then(|s| s.strip_prefix("server:"))
        .ok_or("missing channel")?;
    let (tx, rx) = mpsc::channel();
    let (mut child, mut stdin) = start_channel(&cwd, name, tx.clone())?;
    spawn_stdin_reader(tx);
    let result = (|| {
        hook("SessionStart", json!({"source":"startup", "model":"fake"}));
        let ready = include_str!(
            "../../../../agend-core/tests/fixtures/screens/claude-2.1.284-main-100x24-0.txt"
        )
        .trim_end()
        .replace(
            "<rec>/h1/workspace/g12-startup-capture",
            &cwd.display().to_string(),
        );
        print!("\x1b[2J\x1b[H");
        for line in ready.lines() {
            print!("{line}\r\n");
        }
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        let mut turn = 0u64;
        loop {
            // An idle CLI stays alive until its input/channel closes. The
            // harness owns the test deadline; inactivity is not an exit.
            let event = rx.recv().map_err(|e| e.to_string())?;
            match event {
                Event::InputClosed => return Ok(()),
                Event::Channel { content, meta } => {
                    let receipt: ClaudeReceipt = serde_json::from_value(Value::Object(meta))
                        .map_err(|e| format!("native receipt: {e}"))?;
                    if receipt.session_id != session {
                        return Err("foreign channel session".into());
                    }
                    turn += 1;
                    let prompt = format!("00000000-0000-4000-9000-{turn:012}");
                    hook(
                        "UserPromptSubmit",
                        json!({"prompt":content,
                        "permission_mode":"bypassPermissions", "prompt_id":prompt}),
                    );
                    let request = ack_request(turn + 1, &[receipt]);
                    let arguments = request["params"]["arguments"].clone();
                    let tool_id = format!("toolu_fake{turn:016}");
                    let common = json!({"tool_name":"mcp__agend__agend_ack",
                        "tool_input":arguments, "tool_use_id":tool_id,
                        "permission_mode":"bypassPermissions", "prompt_id":prompt});
                    hook("PreToolUse", common.clone());
                    writeln!(stdin, "{request}").map_err(|e| e.to_string())?;
                    let reply = loop {
                        match rx
                            .recv_timeout(Duration::from_secs(5))
                            .map_err(|e| e.to_string())?
                        {
                            Event::Response(reply) if reply["id"] == request["id"] => break reply,
                            Event::Response(_) => {}
                            _ => return Err("unexpected event before ACK result".into()),
                        }
                    };
                    if reply.get("error").is_some() || reply["result"]["isError"] == true {
                        return Err("native ACK rejected".into());
                    }
                    let mut post = common;
                    post["tool_response"] = reply["result"].clone();
                    hook("PostToolUse", post);
                    // Exercise polling between durable ACK and completion.
                    std::thread::sleep(Duration::from_millis(350));
                    let answer = reply_to(&content);
                    println!("{answer}");
                    hook(
                        "Stop",
                        json!({"stop_hook_active":false,
                        "last_assistant_message":answer, "background_tasks":[],
                        "session_crons":[], "permission_mode":"bypassPermissions",
                        "prompt_id":prompt}),
                    );
                }
                Event::Input(_) | Event::Response(_) => {}
            }
        }
    })();
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    result
}
