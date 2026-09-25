//! Which git config keys an agent may set, through any channel: `git config`
//! writes, `git -c key=value`, `--config-env`, and the `GIT_CONFIG_COUNT` /
//! `GIT_CONFIG_KEY_<n>` / `GIT_CONFIG_PARAMETERS` environment.
//!
//! Deny-by-default: config decides where refs go (`remote.*.push`,
//! `remote.*.fetch`, `push.default`, `branch.*.merge`, `rebase.updateRefs`),
//! which work tree and hooks git uses (`core.worktree`, `core.hooksPath`),
//! and what a command name means (`alias.*`, `include.*`). Only identity,
//! display and editor-style keys are allowed.
//!
//! Command-valued keys (`core.editor`, `core.pager`, `gpg.program`,
//! `credential.helper`) stay allowed: git runs them with its exec-path (the
//! real git) first on PATH, which only a deliberate bypass would exploit;
//! out of scope per the gate 3 threat model (see the gate page's known
//! limits). Read commands do not check this list at all.
//!
//! Must NOT: allow a key that changes ref destinations, the work tree, hooks
//! or command names.

/// Keys allowed exactly (section and name lowercased; no subsection).
const EXACT: &[&str] = &[
    "user.name",
    "user.email",
    "user.signingkey",
    "author.name",
    "author.email",
    "committer.name",
    "committer.email",
    "commit.gpgsign",
    "commit.verbose",
    "tag.gpgsign",
    "gpg.program",
    "gpg.format",
    "core.editor",
    "core.pager",
    "core.quotepath",
    "core.autocrlf",
    "core.safecrlf",
    "core.eol",
    "core.whitespace",
    "merge.conflictstyle",
    "merge.log",
    "pull.rebase",
    "pull.ff",
    "rerere.enabled",
    "rerere.autoupdate",
    "rebase.autosquash",
    "rebase.autostash",
    "rebase.stat",
    "init.defaultbranch",
    "safe.directory",
    "i18n.commitencoding",
    "i18n.logoutputencoding",
    "gc.auto",
    "maintenance.auto",
    "diff.algorithm",
    "diff.renames",
    "diff.colormoved",
    "diff.noprefix",
    "diff.mnemonicprefix",
    "status.showuntrackedfiles",
    "log.date",
    "log.decorate",
    "format.pretty",
    "credential.helper",
    "protocol.version",
];

/// Sections whose every key is display-only.
const SECTIONS: &[&str] = &["color", "advice", "column"];

/// Whether an agent may set `key` (`section[.subsection].name`).
pub fn allowed(key: &str) -> bool {
    let Some((section, rest)) = key.split_once('.') else {
        return false;
    };
    let section = section.to_ascii_lowercase();
    if SECTIONS.contains(&section.as_str()) {
        return true;
    }
    // A key with a subsection (`remote.origin.push`) is never in EXACT.
    if rest.contains('.') {
        return false;
    }
    let full = format!("{section}.{}", rest.to_ascii_lowercase());
    EXACT.contains(&full.as_str())
}

/// Short list for refusal messages.
pub const ALLOWED_HINT: &str = "user.*, author.*, committer.*, core.editor, core.pager, pull.rebase, pull.ff, merge.conflictStyle, color.*, advice.*";

/// Keys set through the environment. `count` is `GIT_CONFIG_COUNT`,
/// `key_n(i)` reads `GIT_CONFIG_KEY_<i>`, `params` is `GIT_CONFIG_PARAMETERS`.
/// An unreadable value is an error (the caller refuses writes).
pub fn from_env(
    params: Option<&str>,
    count: Option<&str>,
    key_n: &dyn Fn(usize) -> Option<String>,
) -> Result<Vec<String>, String> {
    let mut keys = match params {
        Some(p) => parse_parameters(p)?,
        None => Vec::new(),
    };
    if let Some(count) = count {
        let n: usize = count
            .trim()
            .parse()
            .map_err(|_| format!("GIT_CONFIG_COUNT={count:?} is not a number"))?;
        for i in 0..n {
            keys.push(key_n(i).ok_or_else(|| format!("GIT_CONFIG_KEY_{i} is missing"))?);
        }
    }
    Ok(keys)
}

/// Keys in `GIT_CONFIG_PARAMETERS`: single-quoted entries (`'key=value'`,
/// or `'key'='value'` / `'key'` since git 2.31), `'\''` for a quote.
fn parse_parameters(s: &str) -> Result<Vec<String>, String> {
    let bad = || format!("GIT_CONFIG_PARAMETERS is not in git's quoted format: {s:?}");
    let b = s.as_bytes();
    let mut i = 0;
    let mut keys = Vec::new();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i == b.len() {
            return Ok(keys);
        }
        let token = sq_token(b, &mut i).ok_or_else(bad)?;
        if b.get(i) == Some(&b'=') {
            i += 1;
            sq_token(b, &mut i).ok_or_else(bad)?;
            keys.push(token);
        } else {
            keys.push(token.split('=').next().unwrap_or_default().to_string());
        }
    }
}

fn sq_token(b: &[u8], i: &mut usize) -> Option<String> {
    if b.get(*i) != Some(&b'\'') {
        return None;
    }
    *i += 1;
    let mut out = Vec::new();
    loop {
        let c = *b.get(*i)?;
        *i += 1;
        if c != b'\'' {
            out.push(c);
            continue;
        }
        // Closing quote; `\''` continues the same token with a literal quote.
        if b.get(*i..*i + 3) == Some(b"\\''".as_slice()) {
            out.push(b'\'');
            *i += 3;
            continue;
        }
        return String::from_utf8(out).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_display_keys_only() {
        for k in [
            "user.name",
            "User.Email",
            "core.editor",
            "color.diff.meta",
            "advice.detachedHead",
            "pull.rebase",
        ] {
            assert!(allowed(k), "{k}");
        }
        for k in [
            "remote.origin.push",
            "remote.origin.fetch",
            "push.default",
            "branch.agend/t-1/fix.merge",
            "core.worktree",
            "core.hooksPath",
            "alias.co",
            "include.path",
            "rebase.updateRefs",
            "clean.requireForce",
            "user",
            "user.name.x",
        ] {
            assert!(!allowed(k), "{k}");
        }
    }

    #[test]
    fn env_keys_are_read_from_both_formats() {
        let none = |_: usize| None;
        assert_eq!(
            from_env(Some("'user.name=A B' 'core.worktree'='/x'"), None, &none).unwrap(),
            ["user.name", "core.worktree"]
        );
        assert_eq!(
            from_env(Some("'user.name'='it'\\''s'  'push.default'"), None, &none).unwrap(),
            ["user.name", "push.default"]
        );
        let keys = |i: usize| Some(format!("k.{i}"));
        assert_eq!(from_env(None, Some("2"), &keys).unwrap(), ["k.0", "k.1"]);
        assert!(from_env(None, Some("2"), &none).is_err());
        assert!(from_env(None, Some("x"), &none).is_err());
        assert!(from_env(Some("user.name=x"), None, &none).is_err());
        assert!(from_env(Some("'unterminated"), None, &none).is_err());
    }
}
