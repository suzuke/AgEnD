//! Opt-in fake native TUI: raw PTY input submits to the same remote thread.
//! This exercises AgEnD integration, not the real Codex CLI implementation.
use crate::fake_agent::codex::Probe;
use serde_json::json;
use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;

struct Raw(String);
impl Raw {
    fn enter() -> Result<Self, String> {
        let original = Command::new("stty")
            .arg("-g")
            .stdin(std::process::Stdio::inherit())
            .output()
            .map_err(|e| e.to_string())?;
        if !original.status.success() {
            return Err("stty -g failed".into());
        }
        let original = String::from_utf8(original.stdout).map_err(|e| e.to_string())?;
        let guard = Self(original.trim().into());
        if !Command::new("stty")
            .args(["raw", "-echo"])
            .status()
            .map_err(|e| e.to_string())?
            .success()
        {
            return Err("stty raw failed".into());
        }
        Ok(guard)
    }
}
impl Drop for Raw {
    fn drop(&mut self) {
        let _ = Command::new("stty").arg(&self.0).status();
        let _ = std::io::stdout().write_all(b"\x1b[?2004l");
    }
}
pub(super) fn run(thread: &str, remote: &Path) -> Result<(), String> {
    let mut probe = Probe::connect(remote)?;
    probe.call(
        "initialize",
        json!({"clientInfo":{"name":"agend-fake-manual-tui","version":"1"}}),
    )?;
    probe.call(
        "thread/resume",
        json!({"threadId":thread,"excludeTurns":true}),
    )?;
    let _raw = Raw::enter()?;
    let mut stdout = std::io::stdout().lock();
    stdout
        .write_all(format!("\x1b[2J\x1b[HFAKE-MANUAL-READY {thread}\x1b[?2004h").as_bytes())
        .map_err(|e| e.to_string())?;
    stdout.flush().map_err(|e| e.to_string())?;
    let mut input = std::io::stdin().lock();
    let mut text = Vec::new();
    let mut escape = Vec::new();
    let mut paste = false;
    let mut byte = [0u8; 1];
    while input.read(&mut byte).map_err(|e| e.to_string())? != 0 {
        let value = byte[0];
        if !escape.is_empty() || value == 0x1b {
            escape.push(value);
            if escape == b"\x1b[200~" {
                paste = true;
                escape.clear();
            } else if escape == b"\x1b[201~" {
                paste = false;
                escape.clear();
                // Echo the completed paste as frontend state, so native
                // consumers can observe it before a daemon restart.
                let display: String = String::from_utf8_lossy(&text)
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                write!(stdout, "\x1b[3;1H\x1b[2KFAKE-MANUAL-DRAFT {display}")
                    .map_err(|e| e.to_string())?;
                stdout.flush().map_err(|e| e.to_string())?;
            } else if !b"\x1b[200~".starts_with(&escape) && !b"\x1b[201~".starts_with(&escape) {
                text.append(&mut escape);
                if text.len() > 128 * 1024 {
                    return Err("manual fixture input exceeded 128 KiB".into());
                }
            }
            continue;
        }
        if !paste && value == 4 && text.is_empty() {
            break;
        }
        if !paste && value == b'\r' && !text.is_empty() {
            let message =
                String::from_utf8(std::mem::take(&mut text)).map_err(|e| e.to_string())?;
            // A human turn has no daemon clientUserMessageId. Busy/idle and
            // user items come from the server, never invented by this TUI.
            let result = probe.call(
                "turn/start",
                json!({"threadId":thread,
                "input":[{"type":"text","text":message,"text_elements":[]}]}),
            )?;
            writeln!(
                stdout,
                "\r\nFAKE-MANUAL-SUBMITTED {}",
                result["turn"]["id"].as_str().unwrap_or_default()
            )
            .map_err(|e| e.to_string())?;
            stdout.flush().map_err(|e| e.to_string())?;
        } else {
            text.push(value);
        }
        if text.len() > 128 * 1024 {
            return Err("manual fixture input exceeded 128 KiB".into());
        }
    }
    Ok(())
}
