//! One read-only frame from a nonce-owned diagnostic home. No control or input.
use agend_client::{Client, FullTerminalUpdate};
use agend_core::protocol::{client::TerminalSubscribeData, terminal::TerminalViewport};
use std::path::Path;

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: startup_frame <home> <nonce> <instance>".into());
    }
    let home = Path::new(&args[1]);
    let nonce = &args[2];
    let instance = &args[3];
    if nonce.len() != 32
        || !nonce.bytes().all(|b| b.is_ascii_hexdigit())
        || home != Path::new(&format!("/private/tmp/g12live-{nonce}/home"))
        || home.canonicalize().map_err(|e| e.to_string())? != home
        || std::fs::read_to_string(home.join(".smoke-owner")).map_err(|e| e.to_string())? != *nonce
        || !matches!(instance.as_str(), "g12live-a" | "g12live-b")
    {
        return Err("diagnostic ownership mismatch".into());
    }
    let mut client =
        Client::connect_once(&home.join("run/daemon.sock"), None).map_err(|e| e.to_string())?;
    client
        .sender()
        .map_err(|e| e.to_string())?
        .subscribe_terminal_frames(TerminalSubscribeData {
            request_id: "startup-diagnostic".into(),
            instance_id: instance.clone(),
            viewport: TerminalViewport {
                top: None,
                rows: 24,
            },
        })
        .map_err(|e| e.to_string())?;
    loop {
        match client.next_full_terminal().map_err(|e| e.to_string())? {
            FullTerminalUpdate::Frame(data) => {
                if data.instance_id != *instance || data.request_id != "startup-diagnostic" {
                    return Err("frame identity mismatch".into());
                }
                let frame = &data.frame;
                let text = frame
                    .cells
                    .iter()
                    .map(|row| {
                        row.iter()
                            .filter(|cell| !cell.leading_spacer)
                            .map(|cell| cell.text.as_str())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let workspace = home.join("workspace").join(instance);
                let known = if !frame.alternate_screen
                    && !frame.viewport_clamped
                    && frame.viewport_top == frame.live_top
                {
                    agend_core::screen::claude_startup::classify(
                        &text,
                        workspace.to_str().ok_or("workspace is not UTF-8")?,
                        frame.size.columns,
                        frame.size.rows,
                    )
                    .map(|prompt| prompt.name())
                } else {
                    None
                };
                println!(
                    "{}",
                    serde_json::json!({"data":data,"classified_startup_prompt":known})
                );
                return Ok(());
            }
            FullTerminalUpdate::Rejected(error) => return Err(error.message),
            FullTerminalUpdate::ControlAck(_) => return Err("unexpected control reply".into()),
            FullTerminalUpdate::ControlChanged(_) => {}
        }
    }
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("startup_frame: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
