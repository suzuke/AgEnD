//! Command-line front end. Commands are split into agent commands and operator
//! commands (D17); the daemon enforces which caller may run which. Agents pass
//! intent only; context comes from caller identity -> binding (plan §4.7).
//! Subcommands also start the daemon, a holder and the TUI (`app`).
//!
//! Status: only `--version` and `--help` exist here (`agend holder` and
//! `agend daemon` are split off in `main`, gates 4 and 6). Everything else
//! arrives with gate 9
//! (docs/ROADMAP.md).
//!
//! Must NOT: build an async runtime, read config or open the DB on the CLI path.

mod agent;
mod operator;

use std::ffi::OsString;
use std::process::ExitCode;

const USAGE: &str = "\
agend - Agent Engineering Daemon (pre-alpha: no commands implemented yet)

Usage:
  agend --version    Print the version
  agend --help       Print this help
  agend daemon       Run the daemon in the foreground (needs AGEND_HOME;
                     Ctrl-C stops it, the agents keep running)
  agend holder <instance-id>
                     Run the holder of one instance (started by the daemon;
                     needs AGEND_HOME)
";

pub fn run(args: Vec<OsString>) -> ExitCode {
    let args: Vec<String> = match args.into_iter().map(OsString::into_string).collect() {
        Ok(args) => args,
        Err(bad) => {
            eprintln!(
                "agend: argument is not valid UTF-8: {}",
                bad.to_string_lossy()
            );
            return ExitCode::from(2);
        }
    };
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("agend {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        None | Some("--help" | "-h") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("agend: unknown command '{other}'\nRun `agend --help` for usage.");
            ExitCode::from(2)
        }
    }
}
