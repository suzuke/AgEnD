//! `fake-codex`: a stand-in for the `codex` CLI the way the daemon's gate 7
//! wrapper runs it inside a holder (gate 7 P2, P8). Global `-c key=value`
//! options come first and are ignored, like the settings they carry.
//!
//! | Command | Behaviour |
//! |---|---|
//! | `app-server --listen unix://<path> [--turn-ms <ms>]` | the fake app-server of [`super::codex`], running until a signal ends it or a client sends `agendFake/exit` (the wrapper starts it in the background, so its stdin is `/dev/null`: it does not stop at end of file like `fake-codex-app-server`). Threads persist under `$AGEND_FAKE_STATE_DIR`, or else `<path>.fake-state/` (the agent's environment is a whitelist without that variable) |
//! | `resume <thread id> --remote unix://<path>` | the fake TUI: prints `agent args: resume <id> --remote <url>` and `agent config: <-c values>`, then waits; a line `q` (or end of input) ends it. It never connects to the app-server (the daemon does not depend on the TUI) |
//!
//! Must NOT: call a model, or read anything but its arguments and stdin.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use super::Args;
use super::codex::Server;

pub fn main(args: impl IntoIterator<Item = String>) -> ExitCode {
    let mut args = args.into_iter().peekable();
    let mut config = Vec::new();
    while args.next_if(|a| a == "-c").is_some() {
        match args.next() {
            Some(value) => config.push(value),
            None => return usage("-c needs a value"),
        }
    }
    match args.next().as_deref() {
        Some("app-server") => app_server(args.collect()),
        Some("resume") => tui(args.collect(), &config),
        _ => usage("expected app-server or resume"),
    }
}

fn usage(error: &str) -> ExitCode {
    eprintln!(
        "fake-codex: {error}\nusage: fake-codex [-c key=value]... app-server --listen unix://<path> [--turn-ms <ms>]\n       fake-codex [-c key=value]... resume <thread id> --remote unix://<path>"
    );
    ExitCode::from(2)
}

/// Where the app-server listening on `listen` keeps its threads.
pub fn state_dir_for(listen: &Path) -> PathBuf {
    super::state_dir().unwrap_or_else(|| {
        let mut name = listen.as_os_str().to_owned();
        name.push(".fake-state");
        PathBuf::from(name)
    })
}

fn app_server(args: Vec<String>) -> ExitCode {
    let args = match Args::parse(args, &["--listen", "--turn-ms"], &[], &[]) {
        Ok(args) => args,
        Err(e) => return usage(&e),
    };
    let Some(path) = args.get("--listen").and_then(|l| l.strip_prefix("unix://")) else {
        return usage("--listen unix://<path> is required");
    };
    let turn_ms = match args.turn_ms() {
        Ok(ms) => ms,
        Err(e) => return usage(&e),
    };
    let path = Path::new(path);
    let state = state_dir_for(path);
    let _server = match Server::bind(path, Duration::from_millis(turn_ms), Some(state)) {
        Ok(server) => server,
        Err(e) => {
            eprintln!(
                "fake-codex app-server: cannot start on {}: {e}",
                path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "fake-codex app-server: listening on unix://{}",
        path.display()
    );
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn tui(args: Vec<String>, config: &[String]) -> ExitCode {
    let [thread, remote_flag, remote] = args.as_slice() else {
        return usage("resume needs <thread id> --remote unix://<path>");
    };
    if remote_flag != "--remote" || !remote.starts_with("unix://") {
        return usage("resume needs <thread id> --remote unix://<path>");
    }
    let mut out = std::io::stdout();
    let _ = writeln!(out, "agent args: resume {thread} --remote {remote}");
    let _ = writeln!(out, "agent config: {}", config.join(" | "));
    let _ = out.flush();
    for line in std::io::stdin().lock().lines() {
        match line {
            Ok(line) if line.trim() == "q" => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    ExitCode::SUCCESS
}
