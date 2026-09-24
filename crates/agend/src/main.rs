//! `agend`: the only binary. Hosts the CLI, the daemon, the holder, the TUI and
//! the shim; which one runs is decided by argv[0] and the subcommand.
//!
//! The argv[0] check is the first thing `main` does: under the names `git`,
//! `kill`, `killall` or `pkill` the process is the shim and must not build a
//! runtime, read config or open the DB (plan §4.7).
//!
//! Must NOT: do any work before the argv[0] dispatch.

mod cli;
mod debug;
mod doctor;
mod init;

use std::process::ExitCode;

fn main() -> ExitCode {
    let argv0 = std::env::args_os().next().unwrap_or_default();
    if let Some(tool) = agend_shim::Tool::from_argv0(&argv0) {
        return agend_shim::run(tool);
    }
    cli::run(std::env::args_os().skip(1).collect())
}
