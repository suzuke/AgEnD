//! Guards `kill`, `killall` and `pkill` so an agent cannot take down the
//! daemon, holders or other agents (kept from v1's `agend-git` kill guard,
//! `src/bin/agend-git/kill_guard.rs`).
//!
//! Policy:
//! - `pkill`/`killall` match processes by name across the whole host, so they
//!   are refused (only purely informational flags pass); the message shows
//!   the scoped alternative: `pgrep`, then `kill <pid>`.
//! - `kill <pid>...`: parsed deny-by-default. At most one leading signal
//!   spec (`-9`, `-KILL`, `-s SIG`, `-n NUM`, `--signal[=]SIG`), an optional
//!   `--`, then targets. Every target must be a plain pid once surrounding
//!   whitespace is trimmed (kill implementations skip it: `" 123"` is pid
//!   123). Refused: `0` and negative targets (process groups; `-1` is every
//!   process you own), pids above `i32::MAX` or longer than 10 digits (BSD
//!   `kill` keeps the pid in an `int`: 4294967295 becomes -1), names
//!   (`kill agend` on util-linux), job specs,
//!   unknown options, and a pid whose executable is `agend` (the daemon and
//!   every holder run the `agend` binary).
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
        cwd: ctx.cwd.clone().unwrap_or_default(),
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
            let operands = kill_operands(args)?;
            for op in operands {
                let pid = kill_target(op)?;
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
        Tool::Git | Tool::Hook(_) => Ok(()),
    }
}

fn is_protected_name(name: &str) -> bool {
    Path::new(name.trim())
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == PROTECTED_EXE)
}

const KILL_INFO: &[&str] = &["-l", "-L", "--list", "--table", "--help", "--version"];

fn refuse_kill_form(what: &str) -> Refusal {
    Refusal::new(
        "kill_target",
        format!("{what}: the kill guard only accepts `kill [-SIGNAL] <pid>...` with plain pids"),
        "find the pid first (pgrep -fl <pattern>), then: kill <pid> ...",
    )
}

/// Target operands of `kill` after one optional leading signal spec and
/// `--`. `Ok(empty)` for informational forms (`-l`, `--help`). A numeric
/// signal spec with no targets (`kill -1`) is refused: some kills read it
/// as pid -1.
fn kill_operands(args: &[String]) -> Result<Vec<&str>, Refusal> {
    let first = args.first().map(String::as_str);
    if first.is_some_and(|a| KILL_INFO.contains(&a)) {
        return Ok(Vec::new());
    }
    let skip = match first {
        None => return Ok(Vec::new()),
        Some("-s" | "-n" | "--signal") => 2,
        Some(a) if a.starts_with("--signal=") => 1,
        Some("--") => 0,
        Some(a) if a.starts_with("--") => {
            return Err(refuse_kill_form(&format!("option {a} is not accepted")));
        }
        Some(a)
            if a.len() > 1
                && a.starts_with('-')
                && a[1..].bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            1
        }
        Some(_) => 0,
    };
    let mut rest: Vec<&str> = args.iter().skip(skip).map(String::as_str).collect();
    if rest.first() == Some(&"--") {
        rest.remove(0);
    }
    if rest.is_empty()
        && skip == 1
        && first.is_some_and(|a| a[1..].bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(Refusal::new(
            "group_kill",
            format!(
                "`kill {}` without a pid can be read as target {}: every process you own",
                first.unwrap_or_default(),
                first.unwrap_or_default()
            ),
            "kill explicit pids instead: pgrep -fl <pattern>, then kill -SIGNAL <pid> ...",
        ));
    }
    Ok(rest)
}

/// Longest pid spelling accepted: the digits of `i32::MAX` (`2147483647`).
/// Longer strings are refused whatever their value: `strtol` saturates an
/// overlong one to `LONG_MAX`, which an `int` pid truncates to -1.
const MAX_PID_DIGITS: usize = 10;

/// A plain positive pid in `1..=i32::MAX`, the way kill implementations read
/// it (surrounding whitespace skipped, `+` and leading zeros allowed).
/// BSD/macOS `kill` stores `strtol()` in an `int`, so 4294967295 would be
/// -1: every process you own. Anything above `i32::MAX` is refused.
fn kill_target(op: &str) -> Result<u32, Refusal> {
    let t = op.trim_matches(|c: char| c.is_ascii_whitespace());
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(refuse_kill_form(&format!("target {op:?} is not a pid")));
    }
    if digits.len() > MAX_PID_DIGITS {
        return Err(refuse_kill_form(&format!(
            "target {op:?} is longer than any pid ({MAX_PID_DIGITS} digits at most)"
        )));
    }
    let n: i64 = t
        .parse()
        .map_err(|_| refuse_kill_form(&format!("target {op:?} is not a pid")))?;
    if n <= 0 {
        return Err(Refusal::new(
            "group_kill",
            format!(
                "target {op} is {}: it signals a whole process group and can reach beyond your own processes",
                if n == 0 {
                    "0 (your own process group)"
                } else {
                    "negative"
                }
            ),
            "kill explicit pids instead: pgrep -fl <pattern>, then kill <pid> ...",
        ));
    }
    if n > i64::from(i32::MAX) {
        return Err(refuse_kill_form(&format!(
            "target {op:?} is above {} (the largest pid): some kills store it in an int, where it wraps to a negative target such as -1, every process you own",
            i32::MAX
        )));
    }
    u32::try_from(n).map_err(|_| refuse_kill_form(&format!("target {op:?} is out of range")))
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
mod tests;
