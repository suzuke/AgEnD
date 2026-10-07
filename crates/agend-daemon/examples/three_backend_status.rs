//! Read-only original Codex thread status for the nonce-owned three-backend smoke.
//! Never starts/resumes a thread, acquires terminal control, or sends model input.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn socket_connect_path(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}
#[path = "../src/driver/codex/rpc.rs"]
#[allow(dead_code)]
mod rpc;

fn call(conn: &mut rpc::Conn, method: &str, params: Value) -> Result<Value, String> {
    let id = conn.request(method, params).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(rpc::Incoming::Response {
            id: received,
            result,
        }) = conn.read_message().map_err(|e| e.to_string())?
            && received == id
        {
            return result.map_err(|e| e.to_string());
        }
    }
    Err(format!("{method} timed out"))
}

fn run() -> Result<Value, String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: three_backend_status <home> <nonce>".into());
    }
    let home = Path::new(&args[1]);
    let nonce = &args[2];
    if nonce.len() != 32
        || !nonce.bytes().all(|b| b.is_ascii_hexdigit())
        || home != Path::new(&format!("/private/tmp/t3-{nonce}/h"))
        || home.canonicalize().map_err(|e| e.to_string())? != home
        || std::fs::read_to_string(home.join(".smoke-owner")).map_err(|e| e.to_string())? != *nonce
    {
        return Err("smoke ownership mismatch".into());
    }
    let go = std::fs::read_to_string(home.join("run/holders/tri-codex.codex-go"))
        .map_err(|e| e.to_string())?;
    let thread = go.lines().next().ok_or("missing original thread")?;
    if thread.len() != 36 || !thread.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err("invalid original thread".into());
    }
    let mut conn = rpc::Conn::open(&home.join("run/holders/tri-codex.codex.sock"))
        .map_err(|e| e.to_string())?;
    call(
        &mut conn,
        "initialize",
        json!({"clientInfo":{"name":"agend-smoke-status","version":"1"},"capabilities":{"experimentalApi":true}}),
    )?;
    conn.send(&json!({"method":"initialized"}))
        .map_err(|e| e.to_string())?;
    let result = call(
        &mut conn,
        "thread/read",
        json!({"threadId":thread,"includeTurns":false}),
    )?;
    let row = &result["thread"];
    if row["id"] != thread || row["cwd"].as_str() != home.join("workspace/tri-codex").to_str() {
        return Err("original thread identity mismatch".into());
    }
    Ok(json!({"thread":thread,"status":row["status"],"model":row["model"],"cwd":row["cwd"]}))
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(value) => {
            println!("{value}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("three_backend_status: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
