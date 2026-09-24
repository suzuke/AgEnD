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
pub mod kill_guard;
pub mod protected_ref;
pub mod snapshot;

use std::ffi::OsStr;
use std::path::Path;
use std::process::ExitCode;

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

/// Shim entry point. The guards are not implemented yet (stage 3), so this
/// refuses loudly instead of silently passing the call through.
pub fn run(tool: Tool) -> ExitCode {
    eprintln!(
        "agend shim ({}): not implemented yet; refusing to run",
        tool.as_str()
    );
    ExitCode::from(1)
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
}
