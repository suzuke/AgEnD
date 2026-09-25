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
        Tool::Git => Ok(()),
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
        assert_eq!(code(Tool::Kill, "--signal=TERM 100"), "kill_protected");
        assert_eq!(code(Tool::Kill, "-- -1"), "group_kill");
        assert_eq!(code(Tool::Kill, "-9 -1234"), "group_kill");
        for cmd in ["300", "-9 300", "999", "-l", "-l 9", "--", ""] {
            assert_eq!(code(Tool::Kill, cmd), "allow", "{cmd}");
        }
    }

    /// T9 round 1: forms that reached a holder or a group before.
    #[test]
    fn kill_targets_are_normalised_and_deny_by_default() {
        let k = |args: &[&str]| {
            let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            match classify(Tool::Kill, &v, &names) {
                Ok(()) => "allow",
                Err(r) => r.code,
            }
        };
        for spaced in [" 100", "100 ", "\t100\n", "+100", "0100", "000200"] {
            assert_eq!(k(&[spaced]), "kill_protected", "{spaced:?}");
            assert_eq!(k(&["-9", spaced]), "kill_protected", "{spaced:?}");
        }
        for group in [
            &["0"][..],
            &["-9", "0"],
            &["-1"],
            &["-9"],
            &["-s", "9", "-1"],
            &["-9", "--", "-300"],
            &["00"],
        ] {
            assert_eq!(k(group), "group_kill", "{group:?}");
        }
        for bad in [
            &["agend"][..],
            &["%1"],
            &["-9", "-a", "300"],
            &["--timeout", "1", "300"],
            &["1e3"],
            &["99999999999"],
        ] {
            assert_eq!(k(bad), "kill_target", "{bad:?}");
        }
    }

    /// T9 round 2: BSD/macOS `kill` stores `strtol()` in an `int`, so
    /// 4294967295 wraps to -1 (every process you own) and an overlong
    /// string saturates to `LONG_MAX`, whose low 32 bits are also -1.
    /// Classifier only: nothing here sends a signal.
    #[test]
    fn pids_outside_the_int_range_are_refused() {
        let k = |args: &[&str]| {
            let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            match classify(Tool::Kill, &v, &names) {
                Ok(()) => "allow",
                Err(r) => r.code,
            }
        };
        for big in [
            "2147483648",
            "4294967295",
            "4294967296",
            "4294967297",
            "+4294967295",
            " 4294967295",
            "4294967295\n",
            "04294967295",
            "0000004294967295",
            "18446744073709551615",
            "9223372036854775807",
            "99999999999999999999999999999999999999999999999999",
        ] {
            assert_eq!(k(&[big]), "kill_target", "{big:?}");
            assert_eq!(k(&["-9", big]), "kill_target", "-9 {big:?}");
            assert_eq!(k(&["-s", "KILL", "--", big]), "kill_target", "{big:?}");
            assert_eq!(k(&["300", big]), "kill_target", "300 {big:?}");
        }
        // Overlong spellings of small pids are refused too: no kill needs
        // more digits than i32::MAX has.
        for long in ["00000000300", "+00000000300", "000000000000000000001"] {
            assert_eq!(k(&[long]), "kill_target", "{long:?}");
        }
        // Zero after normalisation stays a group kill.
        for zero in ["+0", " 0 ", "0000000000", "-0", "+00"] {
            assert_eq!(k(&[zero]), "group_kill", "{zero:?}");
        }
        // The boundary itself is a plain pid.
        assert_eq!(k(&["2147483647"]), "allow");
        assert_eq!(k(&["0000000300"]), "allow");
        assert!(kill_target("2147483647").is_ok());
        assert!(kill_target("2147483648").is_err());
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
