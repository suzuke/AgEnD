//! Debug subcommands for inspecting a running daemon (gate 8 P7):
//!
//! - `agend debug ping [--count N] [--interval MS]`: per ping, connect
//!   (retrying up to 10 s while the daemon restarts) and fetch the fleet
//!   view; prints the protocol version and the instance count.
//! - `agend debug watch`: the fleet view, then every event after it. When
//!   the connection ends it reconnects every 500 ms without giving up (like
//!   the TUI) and always fetches the fleet view again (a 1.1 client never
//!   reuses an event cursor).
//!
//! The socket is `$AGEND_HOME/run/daemon.sock`; the caller is
//! `AGEND_INSTANCE` when set (an agent), otherwise the operator.
//!
//! Must NOT: mutate state.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use agend_client::{Client, ClientError};
use agend_core::protocol::client::{
    AttentionAction, AttentionRequiredData, DAEMON_SOCKET, DaemonEvent, FleetView,
};

/// Pause between two connection attempts of `watch` (the TUI's, gate 11 T6).
const WATCH_RECONNECT_EVERY: Duration = Duration::from_millis(500);

const USAGE: &str = "\
usage: agend debug ping [--count N] [--interval MS]
       agend debug watch
";

pub fn run(args: &[String]) -> ExitCode {
    let target = match (socket(), caller()) {
        (Ok(socket), caller) => (socket, caller),
        (Err(e), _) => {
            eprintln!("agend: {e}");
            return ExitCode::from(1);
        }
    };
    match args.first().map(String::as_str) {
        Some("ping") => match ping_options(&args[1..]) {
            Some((count, interval)) => ping(&target.0, target.1, count, interval),
            None => usage(),
        },
        Some("watch") if args.len() == 1 => watch(&target.0, target.1),
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprint!("{USAGE}");
    ExitCode::from(2)
}

/// `$AGEND_HOME/run/daemon.sock`.
fn socket() -> Result<PathBuf, String> {
    match std::env::var_os("AGEND_HOME") {
        Some(home) if Path::new(&home).is_absolute() => Ok(Path::new(&home).join(DAEMON_SOCKET)),
        Some(home) => Err(format!(
            "AGEND_HOME must be an absolute path, got {}",
            Path::new(&home).display()
        )),
        None => Err("AGEND_HOME is not set".into()),
    }
}

/// The instance id inside an agent (the daemon sets `AGEND_INSTANCE`).
fn caller() -> Option<String> {
    std::env::var("AGEND_INSTANCE")
        .ok()
        .filter(|id| !id.is_empty())
}

fn ping_options(args: &[String]) -> Option<(u32, u64)> {
    let (mut count, mut interval) = (1, 1000);
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args.next()?;
        match flag.as_str() {
            "--count" => count = value.parse().ok().filter(|&n| n > 0)?,
            "--interval" => interval = value.parse().ok()?,
            _ => return None,
        }
    }
    Some((count, interval))
}

fn ping(socket: &Path, caller: Option<String>, count: u32, interval_ms: u64) -> ExitCode {
    for n in 1..=count {
        if n > 1 {
            std::thread::sleep(Duration::from_millis(interval_ms));
        }
        let answer = Client::connect(socket, caller.clone()).and_then(|mut client| {
            let fleet = client.get_fleet()?;
            Ok((client.selected(), fleet.instances.len(), client.retried()))
        });
        match answer {
            Ok((version, instances, retried)) => {
                let what = format!(
                    "client protocol {}.{}, instances={instances}",
                    version.major, version.minor
                );
                let retried = if retried.is_zero() {
                    String::new()
                } else {
                    format!(" (retried {:.1} s)", retried.as_secs_f64())
                };
                if count == 1 {
                    println!("agend daemon: {what}{retried}");
                } else {
                    println!("ok {n}/{count}{retried}: {what}");
                }
            }
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}

fn watch(socket: &Path, caller: Option<String>) -> ExitCode {
    loop {
        let mut client = match Client::connect_once(socket, caller.clone()) {
            Ok(client) => client,
            Err(e @ ClientError::Version(_)) => {
                eprintln!("{e}");
                return ExitCode::from(1);
            }
            Err(e) => {
                println!("reconnecting… ({e})");
                std::thread::sleep(WATCH_RECONNECT_EVERY);
                continue;
            }
        };
        let fleet = match client.get_fleet() {
            Ok(fleet) => fleet,
            Err(e) => {
                println!("disconnected: {e}");
                continue;
            }
        };
        for line in fleet_lines(&fleet) {
            println!("{line}");
        }
        if let Err(e) = follow(&mut client, fleet.as_of_event_id) {
            println!("disconnected: {e}");
        }
    }
}

/// Prints every event after `as_of` until the connection ends.
fn follow(client: &mut Client, as_of: u64) -> Result<(), ClientError> {
    client.subscribe_events(Some(as_of))?;
    loop {
        let event = client.next_event()?;
        println!("{}", event_line(&event.event));
    }
}

/// The fleet view in a few lines: a summary, then each needs-you item.
pub fn fleet_lines(fleet: &FleetView) -> Vec<String> {
    let instances: Vec<String> = fleet
        .instances
        .iter()
        .map(|i| format!("{} {}", i.instance_id, i.state.as_str()))
        .collect();
    let mut lines = vec![format!(
        "fleet: instances={} ({}) tasks={} teams={} attention={} as_of={}",
        fleet.instances.len(),
        instances.join(", "),
        fleet.tasks.len(),
        fleet.teams.len(),
        fleet.attention.len(),
        fleet.as_of_event_id
    )];
    lines.extend(
        fleet
            .attention
            .iter()
            .map(|a| format!("  {}", attention(a))),
    );
    lines
}

fn actions(actions: &[AttentionAction]) -> String {
    if actions.is_empty() {
        return "none".into();
    }
    let names: Vec<&str> = actions.iter().map(|a| a.as_str()).collect();
    names.join(", ")
}

fn attention(item: &AttentionRequiredData) -> String {
    format!(
        "{} (unblocks {}, waiting since {}; if ignored: {}) actions: {}",
        item.attention_id.as_deref().unwrap_or("-"),
        item.unblocks.unwrap_or(0),
        item.waiting_since_unix_ms
            .map_or("-".into(), |ms| ms.to_string()),
        item.if_ignored.as_deref().unwrap_or("-"),
        actions(&item.actions)
    )
}

/// One event as `agend debug watch` prints it.
pub fn event_line(event: &DaemonEvent) -> String {
    match event {
        DaemonEvent::InstanceChanged { data } => match &data.instance {
            Some(view) => format!(
                "instance_changed {} {}: {}",
                data.instance_id,
                view.state.as_str(),
                data.summary
            ),
            None => format!("instance_changed {}: {}", data.instance_id, data.summary),
        },
        DaemonEvent::AttentionRequired { data } => {
            format!("attention_required {}", attention(data))
        }
        DaemonEvent::AttentionResolved { data } => format!(
            "attention_resolved {} {}",
            data.attention_id,
            data.action.as_str()
        ),
        DaemonEvent::TaskChanged { data } => {
            format!("task_changed {}: {}", data.task_id, data.summary)
        }
        DaemonEvent::MessageReceived { data } => {
            format!("message_received {} from {}", data.message_id, data.from)
        }
        DaemonEvent::AskUpdated { data } => format!("ask_updated {}", data.ask_id),
        DaemonEvent::Unknown => "unknown event (a newer daemon?)".into(),
    }
}
