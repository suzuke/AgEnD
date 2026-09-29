//! Setup rules as data and pure functions: the minimum git version
//! (merge-tree needs >= 2.38), how each backend and git is installed, the
//! disk thresholds, and the shape of an `agend doctor` report (gate 9 P8).
//! Gate 13 adds tested backend version ranges, login recognition and the
//! text of generated launchd/systemd units.
//!
//! Must NOT: run commands, read or write files, or register services (the
//! `agend` crate's `setup` module executes these rules).

use alloc::string::String;
use serde::{Deserialize, Serialize};

use crate::model::Backend;

/// The oldest git AgEnD works with: `git merge-tree --write-tree` (2.38).
pub const GIT_MIN: (u32, u32) = (2, 38);

/// Less free space than this on the home's disk fails `agend doctor`.
pub const MIN_FREE_BYTES: u64 = 1 << 30;
/// A home larger than this is a warning (v1 grew to 161 GB).
pub const MAX_HOME_BYTES: u64 = 20 << 30;

/// `(major, minor, patch)` from `git --version` output such as
/// `git version 2.39.5 (Apple Git-154)`; a missing patch is 0.
pub fn parse_git_version(output: &str) -> Option<(u32, u32, u32)> {
    let version = output.trim().strip_prefix("git version ")?;
    let version = version.split_whitespace().next()?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()
        .and_then(|p| {
            let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .unwrap_or(0);
    Some((major, minor, patch))
}

/// Whether `version` is at least [`GIT_MIN`].
pub fn git_is_new_enough(version: (u32, u32, u32)) -> bool {
    (version.0, version.1) >= GIT_MIN
}

/// How to install or update git.
pub fn git_fix(macos: bool) -> &'static str {
    if macos {
        "brew install git"
    } else {
        "sudo apt-get install git   (or your distribution's package manager)"
    }
}

/// How to install a backend's CLI.
pub fn backend_install(backend: Backend) -> &'static str {
    match backend {
        Backend::Claude => "npm install -g @anthropic-ai/claude-code",
        Backend::Codex => "npm install -g @openai/codex",
        Backend::Opencode => "npm install -g opencode-ai",
    }
}

/// The result of one `agend doctor` check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

impl CheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

/// One line of `agend doctor` (`--json` prints a list of these). Every
/// `warn` and `fail` carries a `fix` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub check: String,
    pub status: CheckStatus,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_versions_parse_and_2_38_is_the_minimum() {
        assert_eq!(
            parse_git_version("git version 2.39.5 (Apple Git-154)\n"),
            Some((2, 39, 5))
        );
        assert_eq!(parse_git_version("git version 2.38"), Some((2, 38, 0)));
        assert_eq!(
            parse_git_version("git version 2.45.1.windows.1"),
            Some((2, 45, 1))
        );
        assert_eq!(parse_git_version("git version 2.30.0"), Some((2, 30, 0)));
        assert_eq!(parse_git_version("not git"), None);
        assert!(git_is_new_enough((2, 38, 0)));
        assert!(git_is_new_enough((3, 0, 0)));
        assert!(!git_is_new_enough((2, 37, 9)));
        assert!(!git_is_new_enough((1, 99, 0)));
    }

    #[test]
    fn every_backend_has_an_install_command() {
        for backend in Backend::ALL {
            assert!(backend_install(backend).starts_with("npm install -g "));
        }
    }
}
