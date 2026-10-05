//! Offline cleanup of the two holders in a nonce-owned live smoke home.
//! This does not start a daemon, backend, channel, or model turn.

use agend_daemon::runtime::{files, shutdown_holder};
use rusqlite::OptionalExtension;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

const IDS: [&str; 2] = ["g12live-a", "g12live-b"];

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

fn validate_path(path: &Path, kind: &str, required: bool) -> Result<(), String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => return Ok(()),
        Err(error) => {
            return Err(format!(
                "cannot inspect {}: {error}; preserved",
                path.display()
            ));
        }
    };
    let file_type = metadata.file_type();
    let correct = match kind {
        "directory" => file_type.is_dir(),
        "file" => file_type.is_file(),
        "socket" => file_type.is_socket(),
        _ => false,
    };
    if !correct || file_type.is_symlink() {
        return Err(format!(
            "unexpected control path type: {}; preserved",
            path.display()
        ));
    }
    Ok(())
}

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
    // Check every control path before reading a lock or connecting a socket.
    // A known filename alone does not establish ownership when it is a link.
    validate_path(home, "directory", true)?;
    validate_path(&home.join(".smoke-owner"), "file", true)?;
    validate_path(&home.join("run"), "directory", false)?;
    validate_path(&files::holders_dir(home), "directory", false)?;
    validate_path(&home.join("agend.db"), "file", false)?;
    for id in IDS {
        validate_path(
            &files::holders_dir(home).join(format!("{id}.lock")),
            "file",
            false,
        )?;
        validate_path(
            &files::holders_dir(home).join(format!("{id}.sock")),
            "socket",
            false,
        )?;
    }
    if home != Path::new(&expected)
        || home.canonicalize().map_err(|e| e.to_string())? != home
        || std::fs::read_to_string(home.join(".smoke-owner")).map_err(|e| e.to_string())? != *nonce
    {
        return Err("home ownership mismatch; preserved".into());
    }
    // Inventory the entire namespace BEFORE stopping any holder. Unknown or
    // unreadable locks remain evidence, never an excuse to delete the home.
    let holders = files::holders_dir(home);
    if holders.exists() {
        for entry in std::fs::read_dir(&holders).map_err(|e| e.to_string())? {
            let name = entry.map_err(|e| e.to_string())?.file_name();
            if let Some(id) = name.to_str().and_then(|s| s.strip_suffix(".lock"))
                && !IDS.contains(&id)
            {
                return Err("foreign holder namespace; preserved".into());
            }
        }
    }
    let live = IDS.map(|id| files::running(home, id));
    for state in &live {
        state.as_ref().map_err(|e| e.to_string())?;
    }
    if !home.join("agend.db").exists() {
        if live.iter().all(|s| matches!(s, Ok(None))) {
            println!("no database or holders; backend was not started");
            return Ok(());
        }
        return Err("holders without database identity; preserved".into());
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
    let foreign: u64 = conn.query_row("SELECT count(*) FROM instances WHERE id NOT IN ('g12live-a','g12live-b') OR backend!='claude'", [], |r| r.get(0)).map_err(|e| e.to_string())?;
    if foreign != 0 {
        return Err("foreign database instance; preserved".into());
    }
    let mut identities = Vec::new();
    for (index, id) in IDS.iter().enumerate() {
        let identity = conn.query_row("SELECT session_id,agent_pid,working_directory FROM instances WHERE id=?1 AND backend='claude'", [id], |r| Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<u32>>(1)?,r.get::<_,String>(2)?))).optional().map_err(|e| e.to_string())?;
        if let Some((session, pgid, workspace)) = identity.as_ref() {
            if Path::new(workspace) != home.join("workspace").join(id)
                || !session
                    .as_deref()
                    .is_some_and(agend_core::protocol::client::is_uuid_v4)
                || (live[index].as_ref().is_ok_and(|s| s.is_some()) && pgid.is_none())
            {
                return Err(
                    "foreign workspace or incomplete session/group identity; preserved".into(),
                );
            }
        } else if live[index].as_ref().is_ok_and(|s| s.is_some()) {
            return Err("holder without database identity; preserved".into());
        }
        identities.push(identity);
    }
    // Every candidate was validated before the first side effect.
    for (id, identity) in IDS.into_iter().zip(identities) {
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
        if let Some((session, Some(pgid), _workspace)) = identity {
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
