//! Operator commands (gate 9 P1, P6, P7): `agend status` without
//! `AGEND_INSTANCE` (the fleet view), `agend instance add|remove|list`,
//! `agend daemon restart`, `agend task cancel`. Users see an instance id
//! as its `name`.
//!
//! Resent after a restart (P5): only the reads (`status`, `instance list`).
//!
//! Must NOT: open the DB or start the daemon.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agend_client::{Client, RESTART_RETRY_WINDOW, Redo};
use agend_core::protocol::client::{
    ClientRequest, ClientResponse, CommandResult, FleetView, OperatorCommand, OperatorData,
};
use clap::Subcommand;

use super::{Failure, Output, Target, absolute, retried, to_json};

/// How long `daemon restart` waits for the preflight's answer (60 s + the
/// copy of the database).
const RESTART_REPLY_WITHIN: Duration = Duration::from_secs(70);
/// How long it waits for the old daemon to close the connection.
const STOP_WITHIN: Duration = Duration::from_secs(30);

#[derive(Subcommand)]
pub enum Instance {
    /// Add an agent and start it
    #[command(
        before_help = "Examples:\n  agend instance add dev-1 claude\n  agend instance add g9-1 claude --program ./target/debug/fake-claude\n  agend instance add dev-2 codex --dir ~/src/app -- --turn-ms 300"
    )]
    Add {
        /// agent name you choose: a-z 0-9 -, up to 24 chars (e.g. dev-1)
        name: String,
        /// claude, codex or opencode
        backend: String,
        /// Where it works (default: $AGEND_HOME/workspace/<name>)
        #[arg(long)]
        dir: Option<PathBuf>,
        /// The backend's program (default: its name, from the agent's PATH)
        #[arg(long)]
        program: Option<String>,
        /// The agent's own arguments
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Stop an agent and remove it; its workspace is kept
    #[command(before_help = "Example: agend instance remove dev-1 --yes")]
    Remove {
        /// agent name
        name: String,
        /// Do not ask (required when not on a terminal)
        #[arg(long)]
        yes: bool,
    },
    /// Every agent: NAME BACKEND STATE DIR
    #[command(before_help = "Example: agend instance list")]
    List,
}

#[derive(Subcommand)]
pub enum Daemon {
    /// Check a binary against a copy of the data, then restart the daemon to it
    #[command(
        before_help = "Examples:\n  agend daemon restart\n  agend daemon restart --binary ./target/release/agend"
    )]
    Restart {
        /// The new binary (default: this agend)
        #[arg(long)]
        binary: Option<PathBuf>,
    },
}

fn operator_request(
    client: &mut Client,
    command: OperatorCommand,
    within: Duration,
    check: &str,
) -> Result<CommandResult, Failure> {
    let request = ClientRequest::Operator {
        data: OperatorData {
            request_id: client.next_request_id(),
            command,
        },
    };
    match client.request_within(&request, Redo::Never, within) {
        Ok(ClientResponse::CommandResult { data }) => Ok(data.result),
        Ok(other) => Err(Failure::new(
            "disconnected",
            format!("unexpected reply: {other:?}"),
        )),
        Err(e) => Err(Failure::client(e, check)),
    }
}

fn fleet(client: &mut Client, check: &str) -> Result<FleetView, Failure> {
    client.get_fleet().map_err(|e| Failure::client(e, check))
}

/// `pid 5101, agend 0.0.0, client protocol 1.2`.
fn describe_daemon(client: &Client) -> String {
    let hello = client.daemon();
    format!(
        "pid {}, {}, client protocol {}.{}",
        hello.daemon_pid.map_or("?".into(), |p| p.to_string()),
        hello.daemon_version.as_deref().unwrap_or("agend ?"),
        hello.selected.major,
        hello.selected.minor
    )
}

pub fn status(target: &Target) -> Result<Output, Failure> {
    let mut client = target.connect("agend status")?;
    let view = fleet(&mut client, "agend status")?;
    let instances: Vec<String> = view
        .instances
        .iter()
        .map(|i| format!("{} {}", i.instance_id, i.state.as_str()))
        .collect();
    let mut lines = vec![
        format!("daemon: {}{}", describe_daemon(&client), retried(&client)),
        format!(
            "instances: {} ({})",
            view.instances.len(),
            instances.join(", ")
        ),
        format!("tasks: {}", view.tasks.len()),
        format!("needs you: {}", view.attention.len()),
    ];
    lines.extend(view.attention.iter().map(|a| {
        let actions: Vec<&str> = a.actions.iter().map(|x| x.as_str()).collect();
        format!(
            "  {}: {} (actions: {})",
            a.attention_id.as_deref().unwrap_or("-"),
            a.reason,
            if actions.is_empty() {
                "none".into()
            } else {
                actions.join(", ")
            }
        )
    }));
    Ok(Output::new(lines, to_json(&view)))
}

pub fn instance(target: &Target, command: Instance, json: bool) -> Result<Output, Failure> {
    match command {
        Instance::Add {
            name,
            backend,
            dir,
            program,
            args,
        } => add(target, name, backend, dir, program, args),
        Instance::Remove { name, yes } => remove(target, &name, yes, json),
        Instance::List => list(target),
    }
}

fn add(
    target: &Target,
    name: String,
    backend: String,
    dir: Option<PathBuf>,
    program: Option<String>,
    args: Vec<String>,
) -> Result<Output, Failure> {
    let dir = dir.map(|d| absolute(&d)).transpose()?;
    // A program given as a path is made absolute; a bare name is looked up
    // on the agent's PATH.
    let program = match program {
        Some(p) if p.contains('/') => Some(absolute(Path::new(&p))?.display().to_string()),
        other => other,
    };
    let check = "agend instance list";
    let mut client = target.connect(check)?;
    let command = OperatorCommand::InstanceAdd {
        instance_id: name,
        backend: backend.clone(),
        working_directory: dir.map(|d| d.display().to_string()),
        program,
        args,
    };
    let result = operator_request(
        &mut client,
        command,
        agend_client::connection::REPLY_WITHIN,
        check,
    )?;
    let CommandResult::InstanceAdded { data } = &result else {
        return Err(Failure::new(
            "disconnected",
            format!("unexpected result: {result:?}"),
        ));
    };
    let session = data
        .session_id
        .as_deref()
        .map(|s| format!("session {s}, "))
        .unwrap_or_default();
    let line = format!(
        "added {} ({backend}, {session}{}); starting",
        data.instance_id, data.working_directory
    );
    Ok(Output::new(vec![line], to_json(&result)))
}

/// Asks on a terminal; `false` unless the answer is yes.
fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim(), "y" | "Y" | "yes")
}

fn remove(target: &Target, name: &str, yes: bool, json: bool) -> Result<Output, Failure> {
    if !yes {
        if json || !std::io::stdin().is_terminal() {
            return Err(Failure::usage(format!(
                "agend instance remove needs --yes when not on a terminal\nexample: agend instance remove {name} --yes"
            )));
        }
        if !confirm(&format!(
            "remove {name}? its agent is stopped; the workspace is kept"
        )) {
            return Err(Failure::new("usage", format!("{name} was not removed")));
        }
    }
    let check = "agend instance list";
    let mut client = target.connect(check)?;
    let dir = fleet(&mut client, check)?
        .instances
        .into_iter()
        .find(|i| i.instance_id == name)
        .and_then(|i| i.working_directory);
    let command = OperatorCommand::InstanceRemove {
        instance_id: name.to_owned(),
    };
    let result = operator_request(
        &mut client,
        command,
        agend_client::connection::REPLY_WITHIN,
        check,
    )?;
    let line = match dir {
        Some(dir) => format!("removed {name}; workspace kept at {dir}"),
        None => format!("removed {name}; its workspace is kept"),
    };
    Ok(Output::new(vec![line], to_json(&result)))
}

fn list(target: &Target) -> Result<Output, Failure> {
    let mut client = target.connect("agend instance list")?;
    let view = fleet(&mut client, "agend instance list")?;
    let rows: Vec<[String; 4]> = view
        .instances
        .iter()
        .map(|i| {
            [
                i.instance_id.clone(),
                i.backend.clone(),
                i.state.as_str().to_owned(),
                i.working_directory.clone().unwrap_or_else(|| "-".into()),
            ]
        })
        .collect();
    let header = ["NAME", "BACKEND", "STATE", "DIR"].map(str::to_owned);
    let width = |col: usize| {
        std::iter::once(&header)
            .chain(&rows)
            .map(|r| r[col].len())
            .max()
            .unwrap_or(0)
    };
    let widths = [width(0), width(1), width(2)];
    let line = |r: &[String; 4]| {
        format!(
            "{:w0$}  {:w1$}  {:w2$}  {}",
            r[0],
            r[1],
            r[2],
            r[3],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2]
        )
    };
    let lines = std::iter::once(&header).chain(&rows).map(line).collect();
    Ok(Output::new(lines, to_json(&view.instances)))
}

pub fn task_cancel(target: &Target, task: String) -> Result<Output, Failure> {
    let mut client = target.connect("agend status")?;
    let command = OperatorCommand::TaskCancel { task_id: task };
    let result = operator_request(
        &mut client,
        command,
        agend_client::connection::REPLY_WITHIN,
        "agend status",
    )?;
    Ok(Output::new(vec!["accepted".into()], to_json(&result)))
}

pub fn daemon(target: &Target, command: Daemon, json: bool) -> Result<Output, Failure> {
    let Daemon::Restart { binary } = command;
    let binary = match binary {
        Some(path) => absolute(&path)?,
        None => std::env::current_exe()
            .map_err(|e| Failure::new("usage", format!("cannot find this agend: {e}")))?,
    };
    restart(target, &binary, json)
}

/// `agend daemon restart` (P7): preflight on the daemon's side, then wait
/// for the old daemon to close this connection, then for a daemon with
/// another boot id.
fn restart(target: &Target, binary: &Path, json: bool) -> Result<Output, Failure> {
    let check = "agend status";
    let restart_floor = agend_client::version::RESTART_SINCE;
    let mut client = Client::connect_needing(&target.socket, target.caller.clone(), restart_floor)
        .map_err(|e| Failure::client(e, check))?;
    let old_pid = client.daemon().daemon_pid;
    let old_boot = client.daemon().boot_id;
    let command = OperatorCommand::DaemonRestart {
        binary: Some(binary.display().to_string()),
    };
    let result = operator_request(&mut client, command, RESTART_REPLY_WITHIN, check)?;
    let CommandResult::Restarting { data } = &result else {
        return Err(Failure::new(
            "disconnected",
            format!("unexpected result: {result:?}"),
        ));
    };
    let started = Instant::now();
    let mut steps = data.preflight.iter();
    let version = steps.next().map_or("agend ?", String::as_str);
    let mut lines = vec![format!("preflight {version} ({}):", binary.display())];
    lines.extend(steps.map(|l| format!("  {l}")));
    lines.push(format!(
        "restarting the daemon (pid {}) ...",
        old_pid.map_or("?".into(), |p| p.to_string())
    ));
    // Shown now: the wait below takes a while.
    if !json {
        for line in &lines {
            println!("{line}");
        }
    }
    if !client.wait_closed(STOP_WITHIN) {
        return Err(Failure::new(
            "restart_failed",
            format!(
                "the daemon accepted the restart but did not stop within {} s; check the daemon's terminal or log",
                STOP_WITHIN.as_secs()
            ),
        ));
    }
    drop(client);
    let deadline = Instant::now() + RESTART_RETRY_WINDOW;
    let (client, view) = loop {
        let attempt = Client::connect(&target.socket, target.caller.clone())
            .map_err(|e| Failure::client(e, check));
        match attempt {
            Ok(mut client) if client.daemon().boot_id != old_boot => {
                let view = fleet(&mut client, check)?;
                break (client, view);
            }
            Ok(_) | Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(_) => {
                return Err(Failure::new(
                    "restart_failed",
                    format!(
                        "the daemon did not come back within {} s (the old one still answers); check the daemon's terminal or log",
                        RESTART_RETRY_WINDOW.as_secs()
                    ),
                ));
            }
            Err(failure) => return Err(failure),
        }
    };
    let back = format!(
        "the daemon is back: {}, instances={} ({:.1} s)",
        describe_daemon(&client),
        view.instances.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(Output::new(vec![back], to_json(&result)))
}
