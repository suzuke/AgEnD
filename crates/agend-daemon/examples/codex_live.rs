//! `codex_live`: gate 7 end to end against the REAL `codex` (the owner's
//! optional step 8). It spends a few short turns of the owner's account, so
//! it refuses to run unless `AGEND_REAL_CODEX=1`; CI never runs it, and the
//! agent that built gate 7 never ran it. Run it inside the write sandbox
//! (`record-sandbox.sh`), after building outside it:
//!
//! ```text
//! cargo build -q -p agend --bin agend && cargo build -q -p agend-daemon --example codex_live
//! AGEND_REAL_CODEX=1 ~/Documents/Hack/AgEnD-ops/record-sandbox.sh target/debug/examples/codex_live
//! ```
//!
//! What it checks (the gate 7 page, U4, U6–U8, U12–U16): the `-c` settings
//! and `resume <thread> --remote …` are accepted; app-server and TUI share a
//! process group that is the terminal's foreground group; a message is
//! confirmed; the agent's `git`, `pkill`, `killall` are the shims in
//! `$AGEND_HOME/bin`; after `kill -9` of the holder no codex is left and the
//! same thread resumes on the same socket path, with its context.
//!
//! Needs: `codex` on `PATH` (or `CODEX_BIN`), `agend` next to this example
//! (or `AGEND_BIN`). Home: a fresh `/tmp/g7-live-<pid>`, removed at the end.
//! It never writes under `~/.codex` (codex itself writes its sessions there).
//!
//! Safety: the daemon is a child (SIGINT / `Child::kill`); the holder is
//! killed only as a pid (> 1) from its lock file in this home; the sweep
//! only signals this instance's group.

#[path = "../tests/common/daemon_process.rs"]
mod lab;

#[path = "../tests/common/codex_process.rs"]
mod process;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use agend_core::model::{Backend, DeliveryState};
use agend_core::policy::busy::BusyLevel;
use agend_core::traits::{AgentMessage, Driver};
use agend_daemon::driver::codex::{CodexDriver, launch};
use agend_daemon::runtime::shutdown_holder;
use agend_daemon::store::{Instance, InstanceStatus, SqliteStore};
use agend_testkit::block_on;
use agend_testkit::fake_agent::codex::Probe;
use serde_json::json;

const ID: &str = "g7-live";
const TURN: Duration = Duration::from_secs(180);

fn main() -> ExitCode {
    if std::env::var("AGEND_REAL_CODEX").as_deref() != Ok("1") {
        eprintln!(
            "codex_live runs the REAL codex and spends a few turns; set AGEND_REAL_CODEX=1 to run it"
        );
        return ExitCode::from(2);
    }
    match run() {
        Ok(()) => {
            println!("codex_live: ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("codex_live FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|dir| Path::new(dir).join(name))
        .find(|p| p.is_file())
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn ensure(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what()) }
}

/// `ps` lines of this instance's processes: pid, pgid, tpgid, command.
fn ps(home: &Path, thread: &str) -> Vec<String> {
    let socket = launch::socket_path(home, ID).display().to_string();
    let out = Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,pgid=,tpgid=,command="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    out.lines()
        .filter(|l| l.contains(&socket) || l.contains(&format!("resume {thread}")))
        .map(|l| l.trim().to_owned())
        .collect()
}

fn run() -> Result<(), String> {
    let codex = std::env::var_os("CODEX_BIN")
        .map(PathBuf::from)
        .or_else(|| which("codex"))
        .ok_or("codex not found on PATH (or set CODEX_BIN)")?;
    let agend = match std::env::var_os("AGEND_BIN") {
        Some(bin) => PathBuf::from(bin),
        None => std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .and_then(Path::parent)
            .map(|d| d.join("agend"))
            .ok_or("cannot locate agend")?,
    };
    let config = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".codex/config.toml"))
        .ok_or("HOME is not set")?;
    let config_before = modified(&config);
    println!(
        "codex: {}; ~/.codex/config.toml modified {config_before:?}",
        codex.display()
    );
    let lab = lab::Lab::with_prefix(&agend, "g7-live");
    let home = lab.home(1);
    let workdir = home.join("workspace").join(ID);
    fs::create_dir_all(&workdir).map_err(|e| e.to_string())?;
    {
        let store = SqliteStore::open(&home, 0).map_err(|e| e.to_string())?;
        block_on(store.add_instance(&Instance {
            id: ID.into(),
            backend: Backend::Codex,
            program: codex.display().to_string(),
            args: Vec::new(),
            working_directory: workdir.display().to_string(),
            session_id: None,
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
        }))
        .map_err(|e| e.to_string())?;
    }

    // 1. First start: thread, $GO, TUI.
    let mut d = lab::Daemon::start(&lab, &home, &[])?;
    d.ready()?;
    let go = d.expect_within(&format!("{ID}: go (resume "), Duration::from_secs(90))?;
    let thread = go
        .split("go (resume ")
        .nth(1)
        .and_then(|r| r.split(')').next())
        .unwrap_or_default()
        .to_owned();
    println!("thread {thread} created");
    std::thread::sleep(Duration::from_secs(3));
    let procs = ps(&home, &thread);
    for line in &procs {
        println!("ps: {line}");
    }
    let groups: Vec<(&str, &str)> = procs
        .iter()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            Some((*f.get(1)?, *f.get(2)?))
        })
        .collect();
    let same = groups.windows(2).all(|w| w[0].0 == w[1].0);
    println!(
        "ps: app-server and TUI share pgid {}, tpgid {} (same group: {same}; U16)",
        groups.first().map_or("-", |g| g.0),
        groups.first().map_or("-", |g| g.1)
    );
    let screen_note = d
        .log
        .iter()
        .filter(|l| l.contains(ID))
        .map(|l| lab::untimed(l).to_owned())
        .collect::<Vec<_>>();
    for line in screen_note {
        println!("  {line}");
    }
    d.interrupt()?;
    let screen = process::screen(&home, ID)?;
    let trust_prompt = screen.to_lowercase().contains("trust this folder");
    println!("TUI screen shows a trust prompt: {trust_prompt} (U7: expect false)");

    // 2. Messages through the driver (the daemon is stopped; the holder,
    //    app-server and TUI keep running).
    let first_reply = {
        let store = Arc::new(SqliteStore::open(&home, 0).map_err(|e| e.to_string())?);
        let driver = CodexDriver::new(&home, Arc::clone(&store), Arc::new(|_| {}));
        for line in block_on(driver.connect(ID, 1))?.unwrap_or_default() {
            println!("  {ID}: {line}");
        }
        deliver_and_wait(
            &driver,
            &store,
            "m-1",
            "Remember the word PAPAYA. Reply with exactly: OK",
        )?;
        deliver_and_wait(
            &driver,
            &store,
            "m-cmd",
            "Run exactly this shell command and reply with its output only: command -v git pkill killall",
        )?;
        last_reply(&home, &thread)?
    };
    let bin = home.join("bin");
    println!("command -v git pkill killall → {first_reply:?}");
    for tool in ["git", "pkill", "killall"] {
        let want = bin.join(tool).display().to_string();
        let found = first_reply.contains(&want);
        println!(
            "  {tool}: {}",
            if found {
                format!("{want} (the shim)")
            } else {
                "NOT the shim".into()
            }
        );
        ensure(found, || format!("{tool} is not {want} (U8)"))?;
    }

    // 3. kill -9 the holder while a daemon runs: sweep, restart, resume on
    //    the same socket path (U14, U15).
    let mut d = lab::Daemon::start(&lab, &home, &[])?;
    d.ready()?;
    d.expect_within(
        &format!("{ID}: thread {thread} resumed"),
        Duration::from_secs(90),
    )?;
    let holder = process::kill9_holder(&home, ID)?;
    println!("kill -9 holder pid={holder}");
    let swept = d.expect_within(
        &format!("{ID}: sweep of agent group"),
        Duration::from_secs(30),
    )?;
    println!("{}", lab::untimed(&swept));
    std::thread::sleep(Duration::from_secs(1));
    let left: Vec<String> = ps(&home, &thread)
        .into_iter()
        .filter(|l| !l.contains(&format!("holder {ID}")))
        .collect();
    println!("codex left right after the sweep: {left:?}");
    d.expect_within(&format!("{ID}: restart 1/3"), Duration::from_secs(30))?;
    d.expect_within(
        &format!("{ID}: thread {thread} resumed"),
        Duration::from_secs(90),
    )?;
    println!("restart 1/3, thread {thread} resumed; second start on the same socket ok");
    d.interrupt()?;

    // 4. The context survived.
    let reply = {
        let store = Arc::new(SqliteStore::open(&home, 0).map_err(|e| e.to_string())?);
        let driver = CodexDriver::new(&home, Arc::clone(&store), Arc::new(|_| {}));
        block_on(driver.connect(ID, 2))?;
        deliver_and_wait(
            &driver,
            &store,
            "m-2",
            "What word did I ask you to remember? Reply with the word only.",
        )?;
        last_reply(&home, &thread)?
    };
    println!(
        "m-2 → confirmed; reply {reply:?} (mentions m-1's word: {})",
        reply.contains("PAPAYA")
    );
    ensure(reply.contains("PAPAYA"), || "the context was lost".into())?;

    shutdown_holder(&home, ID)?;
    let config_after = modified(&config);
    println!(
        "~/.codex/config.toml modified {config_after:?} (unchanged: {})",
        config_before == config_after
    );
    ensure(config_before == config_after, || {
        "~/.codex/config.toml changed".into()
    })?;
    Ok(())
}

fn deliver_and_wait(
    driver: &CodexDriver,
    store: &SqliteStore,
    id: &str,
    body: &str,
) -> Result<(), String> {
    let message = AgentMessage {
        id: id.into(),
        from: "operator".into(),
        task_id: None,
        body: body.into(),
    };
    let receipt =
        block_on(driver.deliver(ID, &message, BusyLevel::Queue)).map_err(|e| e.to_string())?;
    println!("{id} idle → turn/start → {:?}", receipt.state);
    let deadline = Instant::now() + TURN;
    loop {
        let events = block_on(driver.events(ID, None)).map_err(|e| e.to_string())?;
        let state = block_on(store.message(id))
            .map_err(|e| e.to_string())?
            .map(|m| m.state);
        let idle = matches!(
            events.last().map(|e| &e.kind),
            Some(agend_core::traits::DriverEventKind::BusyChanged { busy: false })
        );
        if state == Some(DeliveryState::Confirmed) && idle {
            println!("{id} … confirmed");
            return Ok(());
        }
        ensure(Instant::now() < deadline, || {
            format!("{id}: {state:?} after {TURN:?}")
        })?;
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// The last agent message of the thread, through a separate client.
fn last_reply(home: &Path, thread: &str) -> Result<String, String> {
    let mut probe = Probe::connect(&launch::socket_path(home, ID))?;
    probe.call(
        "initialize",
        json!({"clientInfo": {"name": "codex_live", "title": null, "version": "0"}}),
    )?;
    let page = probe.call(
        "thread/turns/list",
        json!({"threadId": thread, "cursor": null, "limit": 100}),
    )?;
    Ok(page["data"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|t| t["items"].as_array().cloned().unwrap_or_default())
        .filter(|i| i["type"] == "agentMessage")
        .filter_map(|i| i["text"].as_str().map(str::to_owned))
        .next_back()
        .unwrap_or_default())
}
