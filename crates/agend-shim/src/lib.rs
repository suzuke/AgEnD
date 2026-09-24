//! AgEnD shim: the `git` and `kill`/`killall`/`pkill` guards that sit on an
//! agent's PATH (D5). The `agend` binary dispatches here when invoked under one
//! of those names (argv[0]).
//!
//! Startup must stay light: no async runtime, no config file, no DB. The only
//! input about the agent is the read-only binding snapshot the daemon writes.
//!
//! Must NOT: open the DB, start a runtime, or contact the daemon to decide.
//! The snapshot being read-only is a seatbelt, not a security boundary: an
//! agent with the same uid can chmod it (plan §4.6).

pub mod audit;
pub mod binding;
pub mod classify;
pub mod ctx;
pub mod git;
pub mod kill_guard;
pub mod location;
pub mod protected_ref;
pub mod snapshot;

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, ExitCode};

/// Exit code of a refused command.
pub const REFUSED_EXIT: u8 = 1;
/// Exit code when the real tool cannot be found or started.
pub const NOT_FOUND_EXIT: u8 = 127;

/// Program names the shim answers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Git,
    Kill,
    Killall,
    Pkill,
}

impl Tool {
    pub fn as_str(self) -> &'static str {
        match self {
            Tool::Git => "git",
            Tool::Kill => "kill",
            Tool::Killall => "killall",
            Tool::Pkill => "pkill",
        }
    }

    /// The shim tool named by `argv0`'s basename, or `None` for anything else
    /// (including `agend` itself).
    pub fn from_argv0(argv0: &OsStr) -> Option<Tool> {
        match Path::new(argv0).file_name()?.to_str()? {
            "git" => Some(Tool::Git),
            "kill" => Some(Tool::Kill),
            "killall" => Some(Tool::Killall),
            "pkill" => Some(Tool::Pkill),
            _ => None,
        }
    }
}

/// Why a command was refused and the exact next step, written for an LLM
/// agent to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// Stable machine-readable code (audit log, tests).
    pub code: &'static str,
    pub reason: String,
    pub next: String,
}

impl Refusal {
    pub fn new(code: &'static str, reason: impl Into<String>, next: impl Into<String>) -> Refusal {
        Refusal {
            code,
            reason: reason.into(),
            next: next.into(),
        }
    }

    fn shim_loop(tool: &str) -> Refusal {
        Refusal::new(
            "shim_loop",
            format!("the {tool} shim is calling itself (PATH loop)"),
            format!(
                "put the real {tool} on PATH, or report this: agend ask \"shim loop for {tool}\""
            ),
        )
    }

    /// The stderr lines for the agent.
    pub fn render(&self, command: &str) -> Vec<String> {
        vec![
            format!("agend-shim: refused `{command}`"),
            format!("agend-shim: why: {}", self.reason),
            format!("agend-shim: next step: {}", self.next),
        ]
    }
}

/// What the shim decided: lines for the agent (stderr) and the action.
#[derive(Debug)]
pub struct Outcome {
    pub messages: Vec<String>,
    pub action: Action,
}

#[derive(Debug)]
pub enum Action {
    /// Replace this process with the real tool.
    Exec(Command),
    Refuse(Refusal),
    /// The real tool cannot be found; the message is in `messages`.
    NotFound,
}

impl Outcome {
    fn exec(cmd: Command) -> Outcome {
        Outcome {
            messages: Vec::new(),
            action: Action::Exec(cmd),
        }
    }

    fn fail(message: &str) -> Outcome {
        Outcome {
            messages: vec![message.to_string()],
            action: Action::NotFound,
        }
    }
}

/// Decides what to do for `tool` with `args` (argv without argv[0]).
pub fn plan(ctx: &ctx::Ctx, tool: Tool, args: &[OsString]) -> Outcome {
    match tool {
        Tool::Git => git::plan(ctx, args),
        _ => kill_guard::plan(ctx, tool, args),
    }
}

/// Shim entry point, called by `agend`'s `main` after the argv[0] dispatch.
pub fn run(tool: Tool) -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let outcome = plan(&ctx::Ctx::from_env(), tool, &args);
    for line in &outcome.messages {
        eprintln!("{line}");
    }
    match outcome.action {
        Action::Exec(cmd) => exec(cmd),
        Action::Refuse(_) => ExitCode::from(REFUSED_EXIT),
        Action::NotFound => ExitCode::from(NOT_FOUND_EXIT),
    }
}

#[cfg(unix)]
fn exec(mut cmd: Command) -> ExitCode {
    use std::os::unix::process::CommandExt;
    let err = cmd.exec();
    eprintln!(
        "agend-shim: cannot run {}: {err}",
        cmd.get_program().to_string_lossy()
    );
    ExitCode::from(NOT_FOUND_EXIT)
}

#[cfg(not(unix))]
fn exec(mut cmd: Command) -> ExitCode {
    match cmd.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(err) => {
            eprintln!(
                "agend-shim: cannot run {}: {err}",
                cmd.get_program().to_string_lossy()
            );
            ExitCode::from(NOT_FOUND_EXIT)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches_on_basename() {
        assert_eq!(Tool::from_argv0(OsStr::new("git")), Some(Tool::Git));
        assert_eq!(
            Tool::from_argv0(OsStr::new("/home/u/.agend/bin/git")),
            Some(Tool::Git)
        );
        assert_eq!(Tool::from_argv0(OsStr::new("kill")), Some(Tool::Kill));
        assert_eq!(Tool::from_argv0(OsStr::new("killall")), Some(Tool::Killall));
        assert_eq!(Tool::from_argv0(OsStr::new("./pkill")), Some(Tool::Pkill));
    }

    #[test]
    fn other_names_are_not_the_shim() {
        for name in ["agend", "/usr/local/bin/agend", "git-lfs", "gitk", "", "/"] {
            assert_eq!(Tool::from_argv0(OsStr::new(name)), None, "{name}");
        }
    }

    #[test]
    fn refusals_render_reason_and_next_step() {
        let r = Refusal::new("x", "because", "do this");
        assert_eq!(
            r.render("git checkout main"),
            vec![
                "agend-shim: refused `git checkout main`",
                "agend-shim: why: because",
                "agend-shim: next step: do this",
            ]
        );
    }
}
