//! `agend`: the only binary. Hosts the CLI, the daemon, the holder, the TUI and
//! the shim; which one runs is decided by argv[0] and the subcommand.
//!
//! The argv[0] check is the first thing `main` does: under the names `git`,
//! `kill`, `killall` or `pkill` the process is the shim and must not build a
//! runtime, read config or open the DB (plan §4.7).
//!
//! `agend holder <instance-id>` is split off next, before CLI parsing
//! (gate 4 P1), and so is `agend daemon` (gate 6 P1), which builds its own
//! runtime; the CLI path never does.
//!
//! Must NOT: do any work before the argv[0] dispatch.

mod cli;
mod debug;
mod doctor;
mod init;
mod setup;

use std::process::ExitCode;

fn main() -> ExitCode {
    let argv0 = std::env::args_os().next().unwrap_or_default();
    if let Some(tool) = agend_shim::Tool::from_argv0(&argv0) {
        return agend_shim::run(tool);
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    // The holder runs for days and must not parse config or start a runtime:
    // it splits off right after the argv[0] dispatch (gate 4 P1).
    if args.first().is_some_and(|a| a == "holder") {
        return agend_holder::run(args[1..].to_vec());
    }
    if args.first().is_some_and(|a| a == "daemon") {
        return agend_daemon::daemon::run(args[1..].to_vec());
    }
    cli::run(args)
}
