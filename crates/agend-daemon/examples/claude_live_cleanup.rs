//! Offline cleanup of the two holders in a nonce-owned live smoke home.
//! This does not start a daemon, backend, channel, or model turn.

use agend_daemon::runtime::{files, shutdown_holder};
use std::path::Path;

#[path = "../src/driver/codex/sweep.rs"]
#[allow(dead_code)]
mod shared_sweep;
use shared_sweep::group_members;

mod driver {
    pub mod codex {
        pub(crate) use crate::shared_sweep as sweep;
    }
}
#[path = "../src/driver/claude/sweep.rs"]
mod sweep;

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        return Err("usage: claude_live_cleanup <home> <nonce> <agend>".into());
    }
    let nonce = &args[2];
    if nonce.len() != 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid nonce".into());
    }
    let home = Path::new(&args[1]);
    let expected = format!("/private/tmp/g12live-{nonce}/home");
    if home != Path::new(&expected)
        || home.canonicalize().map_err(|e| e.to_string())? != home
        || std::fs::read_to_string(home.join(".smoke-owner")).map_err(|e| e.to_string())? != *nonce
    {
        return Err("home ownership mismatch; preserved".into());
    }
    // Caller stops its child daemon first. Connecting to these holder sockets
    // while a daemon owns them would take over its runtime connection.
    let conn = rusqlite::Connection::open_with_flags(
        home.join("agend.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::ZERO)
        .map_err(|e| e.to_string())?;
    // The production daemon holds SQLite EXCLUSIVE for its entire life.
    conn.query_row("SELECT count(*) FROM instances", [], |r| r.get::<_, u64>(0))
        .map_err(|e| format!("database unavailable; stop daemon first: {e}"))?;
    for id in ["g12live-a", "g12live-b"] {
        if files::running(home, id)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            shutdown_holder(home, id)?;
        }
        if files::running(home, id)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err(format!("holder {id} still running; preserved"));
        }
        use rusqlite::OptionalExtension;
        let identity = conn.query_row("SELECT session_id,agent_pid,working_directory FROM instances WHERE id=?1 AND backend='claude'", [id], |r| Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<u32>>(1)?,r.get::<_,String>(2)?))).optional().map_err(|e| e.to_string())?;
        if let Some((session, Some(pgid), workspace)) = identity {
            if Path::new(&workspace) != home.join("workspace").join(id) {
                return Err("foreign workspace; preserved".into());
            }
            let result = sweep::sweep(
                pgid,
                &sweep::Markers {
                    session: session.as_deref(),
                    instance: id,
                    agend: Path::new(&args[3]),
                },
            );
            println!("{id}: {result:?}");
            let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !group_members(pgid).is_empty() && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            if !group_members(pgid).is_empty() {
                return Err(format!("agent group {pgid} remains; preserved"));
            }
        }
    }
    println!("owned holders absent");
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("claude_live_cleanup: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
