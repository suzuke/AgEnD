//! Command-line front end (gate 9). Commands are split into agent commands
//! ([`agent`], inside an agent where `AGEND_INSTANCE` is set) and operator
//! commands ([`operator`]); the daemon decides who may run what (D17) and
//! says what to do instead, the CLI never checks it first.
//!
//! Output (P4): results on stdout, errors on stderr as `agend: …`, English,
//! no color, the same with or without a TTY. `--json` (anywhere) prints one
//! JSON value on stdout: a core type encoded with `serde_json`, or
//! `{"error":{"code":…,"message":…}}`; nothing on stderr. Exit codes: 0
//! success, 1 failure, 2 usage error (bad arguments, `AGEND_HOME`).
//!
//! Resends after the daemon restarted mid-request (P5): reads and `send`
//! (same message id, so the daemon keeps it once) are sent again; anything
//! else reports `interrupted` with the command to check.
//!
//! Must NOT: build an async runtime, read config or open the DB on the CLI
//! path; check permissions itself.

mod agent;
mod operator;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agend_client::{Client, ClientError};
use agend_core::protocol::client::DAEMON_SOCKET;
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

/// Examples shown first in `agend --help` (P4: examples before options).
const EXAMPLES: &str = "\
Examples:
  agend status                              where you are and what to do next
  agend send dev-2 \"please review t-3\"      message another agent
  agend inbox                               your last 20 messages
  agend instance add dev-1 claude           (operator) add an agent and start it
  agend instance list                       (operator) every agent
  agend daemon restart                      (operator) restart the daemon to this binary
  agend doctor                              check the setup";

const MORE: &str = "\
Also: agend daemon (run the daemon in the foreground; Ctrl-C stops it, the
agents keep running), agend holder <name> (started by the daemon), agend
debug ping|watch, agend --version. Every command needs AGEND_HOME; --json
prints one JSON value.";

#[derive(Parser)]
#[command(
    name = "agend",
    about = "AgEnD: a daemon that runs a team of coding agents",
    before_help = EXAMPLES,
    after_help = MORE,
    disable_version_flag = true,
    disable_help_subcommand = true
)]
struct Cli {
    /// Print one JSON value on stdout (errors too)
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Your instance and task (agent), or the whole fleet (operator)
    #[command(before_help = "Example: agend status")]
    Status,
    /// Send a message to another agent (agent)
    #[command(before_help = "Example: agend send dev-2 \"please review t-3\" --level steer")]
    Send {
        /// The agent's name
        to: String,
        message: String,
        /// How a busy agent gets it
        #[arg(long, value_enum, default_value_t = agent::Level::Queue)]
        level: agent::Level,
    },
    /// Your messages: the last 20, or all after one (agent)
    #[command(before_help = "Example: agend inbox --after <message-id>")]
    Inbox {
        /// A message id from an earlier `agend inbox`
        #[arg(long, value_name = "MESSAGE-ID")]
        after: Option<String>,
    },
    /// Report your stage done (agent)
    #[command(before_help = "Example: agend done t-42/work/1")]
    Done {
        /// <task>/<stage>/<attempt>, from agend status or your assignment
        ticket: String,
    },
    /// Report a result with a summary (agent)
    #[command(before_help = "Example: agend result t-42/plan/1 \"split into two tasks\"")]
    Result { ticket: String, summary: String },
    /// Approve or request changes (agent)
    #[command(subcommand)]
    Review(agent::Review),
    /// Ask the operator a question (agent)
    #[command(
        before_help = "Example: agend ask \"which database?\" --option sqlite --option postgres"
    )]
    Ask {
        /// The question (with --resolve: what was decided)
        text: String,
        /// A choice the operator can pick (repeat for more)
        #[arg(long = "option", value_name = "TEXT")]
        options: Vec<String>,
        /// Continue an ask after its answer
        #[arg(long, value_name = "ASK-ID", conflicts_with = "resolve")]
        follow_up: Option<String>,
        /// Close an ask with what was decided
        #[arg(long, value_name = "ASK-ID")]
        resolve: Option<String>,
    },
    /// Say your task is blocked (agent)
    #[command(before_help = "Example: agend block \"waiting for the API key\"")]
    Block { reason: String },
    /// Say your task is no longer blocked (agent)
    #[command(before_help = "Example: agend unblock")]
    Unblock,
    /// Get reminded of your task later (agent)
    #[command(before_help = "Example: agend remind 30m")]
    Remind {
        /// 90s, 30m or 2h
        delay: String,
    },
    /// Create or cancel tasks
    #[command(subcommand)]
    Task(Task),
    /// Add, remove and list agents (operator)
    #[command(subcommand)]
    Instance(operator::Instance),
    /// Restart the daemon (operator); `agend daemon` alone runs it
    #[command(subcommand)]
    Daemon(operator::Daemon),
    /// Check the setup; exit 1 when a check fails
    #[command(before_help = "Example: agend doctor")]
    Doctor,
    /// Create AGEND_HOME (0700), run doctor, print the next steps
    #[command(before_help = "Example: export AGEND_HOME=$HOME/agend-home && agend init")]
    Init,
}

#[derive(Subcommand)]
enum Task {
    /// Create a task (agent; the operator needs --team)
    #[command(
        before_help = "Example: agend task create --role dev \"add a login page\" --team web"
    )]
    Create {
        title: String,
        #[arg(long)]
        role: String,
        #[arg(long)]
        team: Option<String>,
        #[arg(long)]
        workflow: Option<String>,
    },
    /// Cancel a task (operator)
    #[command(before_help = "Example: agend task cancel t-42")]
    Cancel { task: String },
}

/// A failed command: its `--json` code, message and exit code.
#[derive(Debug)]
pub struct Failure {
    pub code: String,
    pub message: String,
    pub exit: u8,
    /// People see `agend: <code>: <message>` (a code the daemon or the
    /// command itself gives), not just `agend: <message>`.
    pub show_code: bool,
}

impl Failure {
    /// Exit 1.
    pub fn new(code: &str, message: impl Into<String>) -> Failure {
        Failure {
            code: code.into(),
            message: message.into(),
            exit: 1,
            show_code: false,
        }
    }

    /// Exit 1, shown as `agend: <code>: <message>` like a daemon error.
    pub fn coded(code: &str, message: impl Into<String>) -> Failure {
        Failure {
            show_code: true,
            ..Failure::new(code, message)
        }
    }

    /// A usage error: exit 2.
    pub fn usage(message: impl Into<String>) -> Failure {
        Failure {
            exit: 2,
            ..Failure::new("usage", message)
        }
    }

    /// A client error; `check` is the command that shows whether a request
    /// cut off by a restart was done.
    pub fn client(error: ClientError, check: &str) -> Failure {
        match error {
            ClientError::Unreachable { .. } | ClientError::Connect { .. } => {
                Failure::new("daemon_unreachable", error.to_string())
            }
            ClientError::Version(message) => Failure::new("version_mismatch", message),
            ClientError::Daemon { code, message } => Failure::coded(&code, message),
            ClientError::Restarted => Failure::new(
                "interrupted",
                format!("daemon restarted during the request; check with {check}"),
            ),
            ClientError::Disconnected(message) => Failure::new("disconnected", message),
        }
    }

    /// Prints it (stderr, or one JSON value on stdout) and returns the exit
    /// code.
    pub fn report(&self, json: bool) -> ExitCode {
        if json {
            let value = json!({"error": {"code": self.code, "message": self.message}});
            println!("{value}");
        } else if self.show_code {
            eprintln!("agend: {}: {}", self.code, self.message);
        } else {
            eprintln!("agend: {}", self.message);
        }
        ExitCode::from(self.exit)
    }
}

/// What a command prints on success: lines for people, one JSON value, and
/// the exit code (doctor exits 1 when a check fails).
pub struct Output {
    pub lines: Vec<String>,
    pub json: Value,
    pub exit: u8,
}

impl Output {
    pub fn new(lines: Vec<String>, json: Value) -> Output {
        Output {
            lines,
            json,
            exit: 0,
        }
    }
}

/// Encodes a core type for `--json`.
pub fn to_json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// The daemon's socket and who calls: `AGEND_INSTANCE` inside an agent.
pub struct Target {
    pub socket: PathBuf,
    pub caller: Option<String>,
}

impl Target {
    pub fn from_env() -> Result<Target, Failure> {
        let home = crate::home::resolve()?;
        Ok(Target {
            socket: home.join(DAEMON_SOCKET),
            caller: std::env::var("AGEND_INSTANCE")
                .ok()
                .filter(|id| !id.is_empty()),
        })
    }

    /// Connects (retrying up to 10 s while the daemon restarts).
    pub fn connect(&self, check: &str) -> Result<Client, Failure> {
        Client::connect(&self.socket, self.caller.clone()).map_err(|e| Failure::client(e, check))
    }
}

/// ` (retried 1.4 s)` when the client had to wait for the daemon.
pub fn retried(client: &Client) -> String {
    let waited = client.retried();
    if waited.is_zero() {
        String::new()
    } else {
        format!(" (retried {:.1} s)", waited.as_secs_f64())
    }
}

/// An example of the command in `args`, shown after a usage error.
fn example_for(args: &[String]) -> Option<&'static str> {
    let words: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-'))
        .take(2)
        .collect();
    Some(match words.as_slice() {
        ["status", ..] => "agend status",
        ["send", ..] => "agend send dev-2 \"please review t-3\" [--level queue|steer|interrupt]",
        ["inbox", ..] => "agend inbox [--after <message-id>]",
        ["done", ..] => "agend done t-42/work/1",
        ["result", ..] => "agend result t-42/plan/1 \"<summary>\"",
        ["review", ..] => {
            "agend review approve t-42/review/2 | agend review changes t-42/review/2 \"<what to change>\""
        }
        ["ask", ..] => "agend ask \"<question>\" [--option <text>]...",
        ["block", ..] => "agend block \"<reason>\"",
        ["remind", ..] => "agend remind 30m",
        ["task", "cancel"] => "agend task cancel t-42",
        ["task", ..] => "agend task create --role dev \"<title>\" [--team <team>]",
        ["instance", "add"] => {
            "agend instance add dev-1 claude [--dir <path>] [--program <path>] [-- <args>...]"
        }
        ["instance", "remove"] => "agend instance remove dev-1 [--yes]",
        ["instance", ..] => "agend instance add|remove|list",
        ["daemon", ..] => "agend daemon restart [--binary <path>]",
        _ => return None,
    })
}

/// A usage error from clap, with an example (human) or its first line
/// (`--json`).
fn usage_error(error: &clap::Error, args: &[String], json: bool) -> Failure {
    let text = error.to_string();
    let message = if json {
        // The error itself, on one line (without clap's usage and hints).
        text.lines()
            .map(str::trim)
            .take_while(|l| !l.is_empty() && !l.starts_with("Usage:"))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        let mut message = text.trim_end().to_owned();
        if let Some(example) = example_for(args) {
            message.push_str(&format!("\nexample: {example}"));
        }
        message
    };
    Failure::usage(message.trim_start_matches("error: ").to_owned())
}

pub fn run(args: Vec<OsString>) -> ExitCode {
    let args: Vec<String> = match args.into_iter().map(OsString::into_string).collect() {
        Ok(args) => args,
        Err(bad) => {
            let message = format!("argument is not valid UTF-8: {}", bad.to_string_lossy());
            return Failure::usage(message).report(false);
        }
    };
    match args.first().map(String::as_str) {
        // Answered before clap: startup stays under 10 ms (P10).
        Some("--version" | "-V") if args.len() == 1 => {
            println!("agend {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("debug") => return crate::debug::run(&args[1..]),
        _ => {}
    }
    let json = args
        .iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a == "--json");
    let argv = std::iter::once("agend".to_owned()).chain(args.iter().cloned());
    let cli = match Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp || args.is_empty() => {
            print!("{e}");
            return ExitCode::SUCCESS;
        }
        Err(e)
            if !json
                && e.kind() == clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand =>
        {
            eprint!("{e}");
            return ExitCode::from(2);
        }
        Err(e) => return usage_error(&e, &args, json).report(json),
    };
    match dispatch(cli.command, json) {
        Ok(out) => {
            if json {
                println!("{}", out.json);
            } else {
                for line in &out.lines {
                    println!("{line}");
                }
            }
            ExitCode::from(out.exit)
        }
        Err(failure) => failure.report(json),
    }
}

fn dispatch(command: Command, json: bool) -> Result<Output, Failure> {
    match command {
        Command::Doctor => return crate::doctor::run(),
        Command::Init => return crate::init::run(),
        _ => {}
    }
    let target = Target::from_env()?;
    match command {
        Command::Status => match target.caller {
            Some(_) => agent::status(&target),
            None => operator::status(&target),
        },
        Command::Send { to, message, level } => agent::send(&target, to, message, level),
        Command::Inbox { after } => agent::inbox(&target, after),
        Command::Done { ticket } => agent::done(&target, &ticket),
        Command::Result { ticket, summary } => agent::result(&target, &ticket, summary),
        Command::Review(review) => agent::review(&target, review),
        Command::Ask {
            text,
            options,
            follow_up,
            resolve,
        } => agent::ask(&target, text, options, follow_up, resolve),
        Command::Block { reason } => agent::block(&target, Some(reason)),
        Command::Unblock => agent::block(&target, None),
        Command::Remind { delay } => agent::remind(&target, &delay),
        Command::Task(Task::Create {
            title,
            role,
            team,
            workflow,
        }) => agent::task_create(&target, title, role, team, workflow),
        Command::Task(Task::Cancel { task }) => operator::task_cancel(&target, task),
        Command::Instance(instance) => operator::instance(&target, instance, json),
        Command::Daemon(daemon) => operator::daemon(&target, daemon, json),
        Command::Doctor | Command::Init => unreachable!("handled above"),
    }
}

/// `path` made absolute against the current directory (the daemon runs
/// elsewhere).
pub fn absolute(path: &Path) -> Result<PathBuf, Failure> {
    std::path::absolute(path)
        .map_err(|e| Failure::usage(format!("cannot resolve {}: {e}", path.display())))
}
