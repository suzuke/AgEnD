//! The agent's environment (gate 6 P3): built from a whitelist, never
//! inherited, so a secret in the daemon's environment (a Telegram token, an
//! API key) never reaches an agent. The holder adds only `TERM` (and
//! portable-pty `SHELL`, gate 4 G3).
//!
//! - Set by the daemon: `AGEND_HOME`, `AGEND_INSTANCE` (the shim reads both),
//!   and `PATH` = `$AGEND_HOME/bin` (the shims, found first) + the daemon's
//!   `PATH`.
//! - Copied from the daemon when set: [`PASS_THROUGH`].
//!
//! Must NOT: copy any other variable, including other `AGEND_*` ones (for
//! example `AGEND_SHIM_BYPASS`).

use std::collections::BTreeMap;
use std::path::Path;

/// Variables copied from the daemon's environment when present.
pub const PASS_THROUGH: &[&str] = &[
    "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "TZ",
];

/// `PATH` used after `$AGEND_HOME/bin` when the daemon has none.
const DEFAULT_PATH: &str = "/usr/bin:/bin";

/// The environment of instance `id`'s agent, from the daemon's environment
/// `daemon_env` (normally `std::env::vars()`).
pub fn agent_env(
    home: &Path,
    id: &str,
    daemon_env: impl IntoIterator<Item = (String, String)>,
) -> BTreeMap<String, String> {
    let daemon: BTreeMap<String, String> = daemon_env.into_iter().collect();
    let mut env: BTreeMap<String, String> = PASS_THROUGH
        .iter()
        .filter_map(|k| daemon.get(*k).map(|v| ((*k).to_owned(), v.clone())))
        .collect();
    let rest = daemon
        .get("PATH")
        .filter(|p| !p.is_empty())
        .map_or(DEFAULT_PATH, String::as_str);
    let bin = home.join(super::shims::BIN_DIR);
    env.insert("PATH".into(), format!("{}:{rest}", bin.display()));
    env.insert("AGEND_HOME".into(), home.display().to_string());
    env.insert("AGEND_INSTANCE".into(), id.into());
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn only_whitelisted_variables_reach_the_agent_and_shims_come_first() {
        let daemon = vars(&[
            ("TELEGRAM_BOT_TOKEN", "secret"),
            ("ANTHROPIC_API_KEY", "secret"),
            ("AGEND_SHIM_BYPASS", "1"),
            ("AGEND_HOME", "/elsewhere"),
            ("HOME", "/Users/me"),
            ("LANG", "en_US.UTF-8"),
            ("TERM", "screen"),
            ("PATH", "/opt/homebrew/bin:/usr/bin"),
        ]);
        let env = agent_env(Path::new("/h"), "g6-1", daemon);
        assert_eq!(
            env,
            BTreeMap::from(
                [
                    ("AGEND_HOME", "/h"),
                    ("AGEND_INSTANCE", "g6-1"),
                    ("HOME", "/Users/me"),
                    ("LANG", "en_US.UTF-8"),
                    ("PATH", "/h/bin:/opt/homebrew/bin:/usr/bin"),
                ]
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
            )
        );
    }

    #[test]
    fn a_daemon_without_path_still_gives_the_agent_one() {
        let env = agent_env(Path::new("/h"), "a", vars(&[("PATH", "")]));
        assert_eq!(env["PATH"], "/h/bin:/usr/bin:/bin");
    }
}
