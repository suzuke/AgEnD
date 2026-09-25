//! Fake agent programs: tiny stand-ins for the backends that speak only the
//! subset of each real protocol recorded in `docs/backends/*` and in the
//! real-CLI recordings under `transcripts/` (codex 0.156.1, opencode
//! 1.18.31, claude 2.1.282; `tests/conformance.rs` compares them by shape).
//! Each module documents what it covers and what it does not. The binaries
//! in `src/bin/` call `main` here.
//!
//! Replies are deterministic (`fake reply: <prompt>`); a turn takes
//! `--turn-ms` milliseconds so tests can steer or interrupt it. Every fake
//! exits with status 0 when its stdin reaches end of file.
//!
//! Must NOT: call real backends or the network beyond loopback.

use std::io::Read;
use std::path::PathBuf;

pub mod claude;
pub mod codex;
pub mod http;
pub mod opencode;

/// Environment variable naming the directory where a fake persists state
/// across a restart (resume). Unset: nothing persists. Fake-only on purpose:
/// the real CLIs' own variables (`CODEX_HOME`, `XDG_DATA_HOME`) may point at
/// the user's real directories.
pub const STATE_DIR_ENV: &str = "AGEND_FAKE_STATE_DIR";

/// The state directory from [`STATE_DIR_ENV`].
pub(crate) fn state_dir() -> Option<PathBuf> {
    std::env::var_os(STATE_DIR_ENV)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
}

/// Default turn duration of every fake agent.
pub const DEFAULT_TURN_MS: u64 = 100;

pub const CODEX_BIN: &str = "fake-codex-app-server";
pub const OPENCODE_BIN: &str = "fake-opencode-serve";
pub const CLAUDE_BIN: &str = "fake-claude";

/// Finds a fake agent binary next to the running executable (a test in
/// `target/<profile>/deps/` or an example in `target/<profile>/examples/`).
/// Build them first with `cargo build -p agend-testkit --bins`.
pub fn locate(binary: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let name = format!("{binary}{}", std::env::consts::EXE_SUFFIX);
    exe.ancestors()
        .skip(1)
        .take(2)
        .map(|dir| dir.join(&name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            format!(
                "{binary} not found near {}; run `cargo build -p agend-testkit --bins`",
                exe.display()
            )
        })
}

/// Blocks until stdin reaches end of file (the fakes' clean-exit signal).
pub(crate) fn wait_for_stdin_eof() {
    let mut sink = [0u8; 256];
    let mut stdin = std::io::stdin();
    while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
}

/// Parses `--flag value` pairs; unknown flags are errors unless listed in
/// `ignored` (accepted for command-line compatibility with the real CLI).
pub(crate) struct Args {
    pairs: Vec<(String, String)>,
}

impl Args {
    pub(crate) fn parse(
        args: impl IntoIterator<Item = String>,
        known: &[&str],
        switches: &[&str],
        leading: &[&str],
    ) -> Result<Args, String> {
        let mut pairs = Vec::new();
        let mut args = args.into_iter().peekable();
        while args.next_if(|a| leading.contains(&a.as_str())).is_some() {}
        while let Some(flag) = args.next() {
            if switches.contains(&flag.as_str()) {
                continue;
            }
            if !known.contains(&flag.as_str()) {
                return Err(format!("unknown argument {flag}"));
            }
            let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
            pairs.push((flag, value));
        }
        Ok(Args { pairs })
    }

    pub(crate) fn get(&self, flag: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(f, _)| f == flag)
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn turn_ms(&self) -> Result<u64, String> {
        self.get("--turn-ms").map_or(Ok(DEFAULT_TURN_MS), |v| {
            v.parse().map_err(|e| format!("--turn-ms: {e}"))
        })
    }
}

/// `fake reply: <prompt>` with the prompt on one line.
pub(crate) fn reply_to(prompt: &str) -> String {
    format!("fake reply: {}", prompt.replace('\n', " "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn args_skip_leading_words_and_switches() {
        let args = Args::parse(
            strings(&[
                "app-server",
                "--listen",
                "unix:///s",
                "--yolo",
                "--turn-ms",
                "5",
            ]),
            &["--listen", "--turn-ms"],
            &["--yolo"],
            &["app-server"],
        )
        .unwrap();
        assert_eq!(args.get("--listen"), Some("unix:///s"));
        assert_eq!(args.turn_ms(), Ok(5));
        assert!(Args::parse(strings(&["--nope"]), &[], &[], &[]).is_err());
        assert!(Args::parse(strings(&["--listen"]), &["--listen"], &[], &[]).is_err());
    }
}
