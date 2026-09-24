//! Guards `kill`, `killall` and `pkill` so an agent cannot take down the
//! daemon, holders or other agents (kept from v1's `agend-git` kill guard,
//! `src/bin/agend-git/kill_guard.rs`).
//!
//! Policy:
//! - `pkill`/`killall` match processes by name across the whole host, so they
//!   are refused (only purely informational flags pass); the message shows
//!   the scoped alternative: `pgrep`, then `kill <pid>`.
//! - `kill <pid>...`: explicit pids pass, except a pid whose executable is
//!   `agend` (the daemon and every holder run the `agend` binary) and
//!   negative targets (a whole process group; `-1` is every process you own).
//!
//! Limits: `kill` is a shell builtin in bash/zsh/sh, so only the external
//! `kill` (`/bin/kill`, `command kill`, `xargs kill`) reaches the shim. A
//! seatbelt against accidents, not a security boundary.
//!
//! Must NOT: guard anything but these three tools.

use crate::audit::{self, Record};
use crate::ctx::{Ctx, MAX_DEPTH, lossy};
use crate::{Action, Outcome, Refusal, Tool};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// Executable name of the daemon and holder processes.
pub const PROTECTED_EXE: &str = "agend";

pub fn plan(ctx: &Ctx, tool: Tool, args: &[OsString]) -> Outcome {
    let argv = lossy(args);
    let Some(real) = ctx.find_real(tool.as_str()) else {
        return Outcome::fail(&format!(
            "agend-shim: cannot find the real {} on PATH (the shim directory is excluded)",
            tool.as_str()
        ));
    };
    let record = |event: &str, code: Option<&str>, detail: Option<String>| Record {
        ts: audit::now(),
        instance: ctx.instance.clone(),
        tool: tool.as_str().into(),
        event: event.into(),
        code: code.map(String::from),
        argv: argv.clone(),
        cwd: ctx.cwd.clone(),
        detail,
    };
    if ctx.bypass {
        audit::append(ctx.home.as_deref(), &record("bypass", None, None));
        return Outcome::exec(ctx.real_command(&real, args));
    }
    let verdict = if ctx.depth >= MAX_DEPTH {
        Err(Refusal::shim_loop(tool.as_str()))
    } else {
        classify(tool, &argv, &process_name)
    };
    match verdict {
        Ok(()) => Outcome::exec(ctx.real_command(&real, args)),
        Err(r) => {
            audit::append(
                ctx.home.as_deref(),
                &record("refuse", Some(r.code), Some(r.reason.clone())),
            );
            Outcome {
                messages: r.render(&format!("{} {}", tool.as_str(), argv.join(" "))),
                action: Action::Refuse(r),
            }
        }
    }
}

/// Pure policy. `name_of(pid)` returns the executable path or name of a
/// running process (`None` if it is not running or cannot be read).
pub fn classify(
    tool: Tool,
    args: &[String],
    name_of: &dyn Fn(u32) -> Option<String>,
) -> Result<(), Refusal> {
    match tool {
        Tool::Pkill | Tool::Killall => {
            const INFO: &[&str] = &["--help", "-h", "--version", "-V", "-l", "--list", "-L"];
            if args.iter().all(|a| INFO.contains(&a.as_str())) {
                return Ok(());
            }
            let pattern = pattern_hint(args);
            Err(Refusal::new(
                "pattern_kill",
                format!(
                    "`{}` kills by name across the whole host and can hit the agend daemon, holders or other agents",
                    tool.as_str()
                ),
                format!(
                    "list the matches first: pgrep -fl -- '{pattern}'; then kill only your own pids: kill <pid> ..."
                ),
            ))
        }
        Tool::Kill => {
            for op in kill_operands(args) {
                let Ok(n) = op.parse::<i64>() else { continue };
                if n < 0 {
                    return Err(Refusal::new(
                        "group_kill",
                        format!(
                            "target {op} is negative: it signals a whole process group and can reach beyond your own processes"
                        ),
                        "kill explicit pids instead: pgrep -fl <pattern>, then kill <pid> ...",
                    ));
                }
                let Ok(pid) = u32::try_from(n) else { continue };
                if pid == 0 {
                    continue;
                }
                if let Some(name) = name_of(pid)
                    && is_protected_name(&name)
                {
                    return Err(Refusal::new(
                        "kill_protected",
                        format!(
                            "pid {pid} is an agend process ({name}): the daemon or a holder; killing it takes agents down"
                        ),
                        "leave agend processes alone; check your state with `agend status`, and kill only processes you started (by pid)",
                    ));
                }
            }
            Ok(())
        }
        Tool::Git => Ok(()),
    }
}

fn is_protected_name(name: &str) -> bool {
    Path::new(name.trim())
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == PROTECTED_EXE)
}

/// Target operands of `kill`, after an optional leading signal spec
/// (`-s SIG`, `-n NUM`, `--signal SIG`, `-9`, `-TERM`) and `--`.
fn kill_operands(args: &[String]) -> Vec<&str> {
    let skip = match args.first().map(String::as_str) {
        Some("-s" | "--signal" | "-n") => 2,
        Some("--") => 1,
        Some(s) if s.starts_with('-') && s.len() > 1 => 1,
        _ => 0,
    };
    args.iter()
        .skip(skip.min(args.len()))
        .map(String::as_str)
        .filter(|a| *a != "--")
        .collect()
}

/// The pattern part of a pkill/killall argv, for the `pgrep` hint.
fn pattern_hint(args: &[String]) -> String {
    let toks: Vec<&str> = match args.iter().position(|a| a == "--") {
        Some(pos) => args[pos + 1..].iter().map(String::as_str).collect(),
        None => args
            .iter()
            .map(String::as_str)
            .filter(|a| !a.starts_with('-'))
            .collect(),
    };
    toks.join(" ").replace('\'', "")
}

/// Executable of a running process via `ps -o comm= -p <pid>` (POSIX; on
/// macOS the full path, on Linux the short name).
pub fn process_name(pid: u32) -> Option<String> {
    let ps = if Path::new("/bin/ps").exists() {
        "/bin/ps"
    } else {
        "ps"
    };
    let out = Command::new(ps)
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    fn names(pid: u32) -> Option<String> {
        match pid {
            100 => Some("/usr/local/bin/agend".into()),
            200 => Some("agend".into()),
            300 => Some("/usr/bin/sleep".into()),
            _ => None,
        }
    }

    fn code(tool: Tool, cmd: &str) -> &'static str {
        match classify(tool, &argv(cmd), &names) {
            Ok(()) => "allow",
            Err(r) => r.code,
        }
    }

    #[test]
    fn pattern_kills_are_refused_unless_informational() {
        for cmd in [
            "-f resume --last",
            "agend",
            "-u me",
            "-- -weird",
            "-9 codex",
        ] {
            assert_eq!(code(Tool::Pkill, cmd), "pattern_kill", "{cmd}");
            assert_eq!(code(Tool::Killall, cmd), "pattern_kill", "{cmd}");
        }
        for cmd in ["", "--help", "-l", "--version"] {
            assert_eq!(code(Tool::Pkill, cmd), "allow", "{cmd}");
        }
        let r = classify(Tool::Pkill, &argv("-f my-server"), &names).unwrap_err();
        assert!(r.next.contains("pgrep -fl -- 'my-server'"), "{}", r.next);
    }

    #[test]
    fn kill_refuses_agend_processes_and_groups() {
        assert_eq!(code(Tool::Kill, "100"), "kill_protected");
        assert_eq!(code(Tool::Kill, "-9 300 200"), "kill_protected");
        assert_eq!(code(Tool::Kill, "-s TERM 100"), "kill_protected");
        assert_eq!(code(Tool::Kill, "-- -1"), "group_kill");
        assert_eq!(code(Tool::Kill, "-9 -1234"), "group_kill");
        for cmd in ["300", "-9 300", "0", "999", "-l", "%1"] {
            assert_eq!(code(Tool::Kill, cmd), "allow", "{cmd}");
        }
    }

    #[test]
    fn protected_name_is_the_basename() {
        assert!(is_protected_name("/Users/x/.cargo/bin/agend"));
        assert!(is_protected_name("agend\n"));
        assert!(!is_protected_name("agend-git"));
        assert!(!is_protected_name("/bin/sleep"));
    }

    #[cfg(unix)]
    #[test]
    fn process_name_reads_a_live_process() {
        let me = process_name(std::process::id()).expect("own process visible to ps");
        assert!(!me.is_empty());
        assert_eq!(process_name(u32::MAX - 1), None);
    }
}
