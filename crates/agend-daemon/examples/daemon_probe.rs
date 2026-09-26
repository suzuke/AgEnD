//! `daemon_probe`: adds and removes instances in `$AGEND_HOME/agend.db` by
//! hand, and the gate 6 acceptance demo (`daemon_probe demo`, run by `cargo
//! xtask accept daemon-holder`). Not a production command (gate 6 P2): it
//! opens the DB directly, so it only works while no daemon runs (otherwise
//! `agend.db is in use by another process`).
//!
//! Commands (`add`, `remove`, `list` need `AGEND_HOME`):
//!   add <id> [--dies]     a claude-style instance running a bash counter
//!                         (`--dies`: an agent that exits at once), working
//!                         directory `$AGEND_HOME/workspace/<id>`
//!   remove <id>
//!   list
//!   demo                  finds `agend` through `AGEND_BIN` (default:
//!                         `target/<profile>/agend` next to this example)
//!
//! Safety: the demo only signals daemons it started itself (SIGINT, or
//! `Child::kill`); holders are stopped with `Shutdown` (see
//! `tests/common/daemon_process.rs`).

#[path = "../tests/common/daemon_process.rs"]
mod lab;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("add") => home().and_then(|home| add(&home, &args[1..])),
        Some("remove") => home().and_then(|home| match args.get(1) {
            Some(id) => lab::remove(&home, id).map(|()| println!("removed {id}")),
            None => Err("usage: daemon_probe remove <id>".into()),
        }),
        Some("list") => home().and_then(|home| list(&home)),
        Some("demo") => demo(),
        _ => Err("usage: daemon_probe add <id> [--dies] | remove <id> | list | demo".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            println!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn home() -> Result<PathBuf, String> {
    match std::env::var_os("AGEND_HOME") {
        Some(home) if Path::new(&home).is_absolute() => Ok(PathBuf::from(home)),
        Some(_) => Err("AGEND_HOME must be an absolute path".into()),
        None => Err("AGEND_HOME is not set".into()),
    }
}

fn add(home: &Path, args: &[String]) -> Result<(), String> {
    let (id, script) = match args {
        [id] => (id, lab::COUNTER),
        [id, flag] if flag == "--dies" => (id, lab::DIES_AT_ONCE),
        _ => return Err("usage: daemon_probe add <id> [--dies]".into()),
    };
    agend_daemon::store::instances::validate_id(id)?;
    let instance = lab::add(home, id, script)?;
    println!(
        "added {id}: claude session {} in {}",
        instance.session_id.unwrap_or_default(),
        instance.working_directory
    );
    Ok(())
}

fn list(home: &Path) -> Result<(), String> {
    let store = SqliteStore::open(home, 0).map_err(|e| e.to_string())?;
    for i in block_on(store.instances()).map_err(|e| e.to_string())? {
        println!(
            "{} {} {} session={}",
            i.id,
            i.backend.as_str(),
            i.status.as_str(),
            i.session_id.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

fn agend_bin() -> Result<PathBuf, String> {
    if let Some(bin) = std::env::var_os("AGEND_BIN") {
        return Ok(PathBuf::from(bin));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // target/<profile>/examples/daemon_probe -> target/<profile>/agend
    let bin = exe
        .parent()
        .and_then(Path::parent)
        .map(|dir| dir.join("agend"))
        .ok_or("cannot locate agend; set AGEND_BIN")?;
    if bin.is_file() {
        Ok(bin)
    } else {
        Err(format!(
            "{} not found; run `cargo build -p agend` or set AGEND_BIN",
            bin.display()
        ))
    }
}

fn section(name: &str, lines: Result<Vec<String>, String>) -> Result<(), String> {
    println!("\n== {name}");
    for line in lines.map_err(|e| format!("{name} failed: {e}"))? {
        println!("{line}");
    }
    Ok(())
}

fn demo() -> Result<(), String> {
    let lab = lab::Lab::new(&agend_bin()?);
    println!("demo directory {}", lab.root.display());
    let tag = "d";
    let home = lab.home(1);
    section("restart", lab::restart(&lab, tag, &|_| home.clone()))?;
    let negative = lab::restart(&lab, "n", &|boot| lab.home(10 + boot));
    match negative {
        Ok(_) => return Err("negative check passed: it cannot tell restarts apart".into()),
        Err(e) => println!(
            "negative check (new AGEND_HOME each boot): {}",
            e.lines().next().unwrap_or_default()
        ),
    }
    section("give-up", lab::give_up(&lab, tag))?;
    section("crash-before-spawn", lab::crash_before_spawn(&lab, tag))?;
    section("env", lab::env(&lab, tag))?;
    section("second-daemon", lab::second_daemon(&lab, tag))?;
    section("orphan", lab::orphan(&lab, tag))?;
    println!("\n== cleanup");
    let stopped = lab.stop_all_holders();
    let left = lab.running_holders();
    println!(
        "stopped {stopped} holder(s) with Shutdown; running now: {}",
        left.len()
    );
    if !left.is_empty() {
        return Err(format!("holders left: {left:?}"));
    }
    println!("daemon demo: all sections passed");
    Ok(())
}
