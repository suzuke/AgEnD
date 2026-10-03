//! Deterministic inbox worker; uses real CLI and git shim, never a backend.
use serde_json::Value;
use std::path::Path;
use std::process::Command;
fn cli(args: &[&str]) -> Result<Value, String> {
    let out = Command::new("agend")
        .args(args)
        .arg("--json")
        .output()
        .map_err(|e| e.to_string())?;
    let value = serde_json::from_slice::<Value>(&out.stdout)
        .map_err(|e| format!("invalid CLI output: {e}"))?;
    if !out.status.success() {
        return Err(value.to_string());
    }
    Ok(value)
}
fn git(wt: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(wt)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into())
    }
}
fn handle(body: &str, flags: &[String]) -> Result<(), String> {
    let Some(ticket) = body
        .lines()
        .next()
        .and_then(|s| s.strip_prefix("dispatch "))
    else {
        return Ok(());
    };
    if flags.iter().any(|s| s == "--hold") {
        return Ok(());
    }
    let status = cli(&["status"])?;
    let identity = status.pointer("/data/identity");
    let mut parts = ticket.split('/');
    let (task, stage, attempt) = (
        parts.next(),
        parts.next(),
        parts.next().and_then(|s| s.parse::<u64>().ok()),
    );
    if status.pointer("/data/task_id").and_then(Value::as_str) != task
        || identity
            .and_then(|v| v.get("stage_id"))
            .and_then(Value::as_str)
            != stage
        || identity
            .and_then(|v| v.get("attempt"))
            .and_then(Value::as_u64)
            != attempt
    {
        return Ok(());
    }
    let wt = body
        .lines()
        .find_map(|s| s.strip_prefix("worktree: "))
        .ok_or("dispatch has no worktree")?;
    if body.lines().any(|l| l == "kind: review") {
        if flags.iter().any(|s| s == "--changes-once") && !Path::new(".changes-sent").exists() {
            cli(&["review", "changes", ticket, "Please revise hello.txt"])?;
            std::fs::write(".changes-sent", "").map_err(|e| e.to_string())?;
        } else {
            cli(&["review", "approve", ticket])?;
        }
    } else if body.contains("next: agend result") {
        cli(&["result", ticket, "A deterministic research result"])?;
    } else {
        let wt = Path::new(wt);
        let fail = flags.iter().any(|s| s == "--fail-checks-once")
            && !Path::new(".failed-checks-once").exists();
        let name = if fail { "forgotten.txt" } else { "hello.txt" };
        let old = std::fs::read_to_string(wt.join(name)).unwrap_or_default();
        std::fs::write(wt.join(name), format!("{old}{ticket}\n")).map_err(|e| e.to_string())?;
        git(wt, &["add", name])?;
        git(wt, &["commit", "-m", &format!("Implement {ticket}")])?;
        if flags.iter().any(|s| s == "--leave-wip") || Path::new(".leave-wip").exists() {
            std::fs::write(wt.join("unfinished.txt"), "uncommitted work\n")
                .map_err(|e| e.to_string())?;
        }
        cli(&["done", ticket])?;
        if fail {
            std::fs::write(".failed-checks-once", "").map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn main() {
    let flags = std::env::args().skip(1).collect::<Vec<_>>();
    loop {
        let after = std::fs::read_to_string(".inbox-cursor").ok();
        let args = after
            .as_ref()
            .map_or(vec!["inbox"], |id| vec!["inbox", "--after", id.as_str()]);
        match cli(&args) {
            Ok(value) => {
                if let Some(messages) = value.pointer("/data/messages").and_then(Value::as_array) {
                    for message in messages {
                        let Some(id) = message.get("message_id").and_then(Value::as_str) else {
                            continue;
                        };
                        let body = message
                            .get("body")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        match handle(body, &flags) {
                            Ok(()) => {
                                let _ = std::fs::write(".inbox-cursor", id);
                            }
                            Err(e) => {
                                eprintln!("fake-worker: {e}");
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                if e.contains("unknown_message") {
                    let _ = std::fs::remove_file(".inbox-cursor");
                } else {
                    eprintln!("fake-worker: {e}");
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
