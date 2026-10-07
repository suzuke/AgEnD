//! `agend doctor` (gate 9 P8): one line per check, `ok` / `warn` / `fail`;
//! every `warn` and `fail` comes with a `fix:` command. Exit 1 when a check
//! fails. `--json` prints the list of `agend_core::setup::Check`.
//!
//! | Check | fail | warn |
//! |---|---|---|
//! | home | missing, not a directory, not writable, a v1 home | not 0700 |
//! | daemon | — | not reachable (one attempt) |
//! | git | missing, or older than 2.38 | — |
//! | claude, codex, opencode | missing and an instance uses it | missing |
//! | holders | — | orphans (held lock, no instance); unknown without a daemon |
//! | disk | < 1 GB free | home > 20 GB |
//!
//! Gate 10 adds the write sandbox for checks; gates 12 and 13 the logins,
//! version ranges, gh, the service and Telegram.
//!
//! Must NOT: change anything, open the DB, or connect to a holder's socket
//! (it would take over the daemon's connection); a holder's life is its
//! lock, read with the daemon's own function.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use agend_client::Client;
use agend_core::model::Backend;
use agend_core::protocol::client::{DAEMON_SOCKET, FleetView};
use agend_core::setup::{
    Check, CheckStatus, MAX_HOME_BYTES, MIN_FREE_BYTES, backend_install, git_fix,
    git_is_new_enough, parse_git_version,
};
use agend_daemon::runtime::files;

use crate::cli::{Failure, Output, to_json};
use crate::home;
use crate::setup;

fn check(name: &str, status: CheckStatus, detail: String, fix: Option<String>) -> Check {
    Check {
        check: name.into(),
        status,
        detail,
        fix,
    }
}

fn ok(name: &str, detail: String) -> Check {
    check(name, CheckStatus::Ok, detail, None)
}

pub fn run() -> Result<Output, Failure> {
    let home = home::from_env()?;
    Ok(report(&checks(&home)))
}

/// Every check for `home`.
pub fn checks(home: &Path) -> Vec<Check> {
    let (daemon, fleet) = daemon(home);
    let mut out = vec![home_check(home), daemon, git(home)];
    for backend in Backend::ALL {
        out.push(backend_check(home, backend, fleet.as_ref()));
    }
    out.push(holders(home, fleet.as_ref()));
    out.push(disk(home));
    out.push(sandbox(home));
    out.push(telegram(home));
    out
}

fn sandbox(home: &Path) -> Check {
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
        .and_then(|rt| rt.block_on(agend_daemon::checks::readiness(home)));
    match result {
        Ok(()) => ok("sandbox", "checks write sandbox probe passed".into()),
        Err(reason) => check(
            "sandbox",
            CheckStatus::Fail,
            reason,
            Some(
                if cfg!(target_os = "linux") {
                    "sudo apt-get install bubblewrap; enable unprivileged user namespaces"
                } else {
                    "restore /usr/bin/sandbox-exec and run agend doctor"
                }
                .into(),
            ),
        ),
    }
}

/// The lines people read, and exit 1 when a check failed.
pub fn report(checks: &[Check]) -> Output {
    let mut lines = Vec::new();
    for c in checks {
        lines.push(format!(
            "{:<5} {:<9} {}",
            c.status.as_str(),
            c.check,
            c.detail
        ));
        if let Some(fix) = &c.fix {
            lines.push(format!("      fix: {fix}"));
        }
    }
    let failed = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Fail)
        .count();
    let warned = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Warn)
        .count();
    lines.push(match (failed, warned) {
        (0, 0) => "all checks passed".into(),
        (0, w) => format!("no check failed ({w} warning{})", plural(w)),
        (f, _) => format!("{f} check{} failed", plural(f)),
    });
    let mut out = Output::new(lines, to_json(&checks));
    out.exit = u8::from(failed > 0);
    out
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn home_check(home: &Path) -> Check {
    let shown = home.display();
    let fail = |detail: String, fix: String| check("home", CheckStatus::Fail, detail, Some(fix));
    let Ok(meta) = std::fs::metadata(home) else {
        return fail(format!("{shown} does not exist"), "agend init".into());
    };
    if !meta.is_dir() {
        return fail(
            format!("{shown} is not a directory"),
            "export AGEND_HOME=<a directory>".into(),
        );
    }
    if home::is_v1(home) {
        return fail(
            format!("{shown} looks like an AgEnD v1 home ({})", home::V1_MARKER),
            "export AGEND_HOME=<another directory>".into(),
        );
    }
    if !setup::writable(home) {
        return fail(
            format!("{shown} is not writable"),
            format!("chmod u+rwx {shown}"),
        );
    }
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o700 {
        return check(
            "home",
            CheckStatus::Warn,
            format!("{shown} ({mode:04o}; others can read it)"),
            Some(format!("chmod 700 {shown}")),
        );
    }
    ok("home", format!("{shown} (0700)"))
}

/// The daemon line, and its fleet view when it answers (one attempt).
fn daemon(home: &Path) -> (Check, Option<FleetView>) {
    let socket = home.join(DAEMON_SOCKET);
    let mut client = match Client::connect_once(&socket, None) {
        Ok(client) => client,
        Err(e) => {
            let fix = match e {
                agend_client::ClientError::Version(_) => "agend daemon restart",
                _ => "agend daemon",
            };
            return (
                check(
                    "daemon",
                    CheckStatus::Warn,
                    format!("not reachable: {e}"),
                    Some(fix.into()),
                ),
                None,
            );
        }
    };
    let hello = client.daemon().clone();
    let fleet = client.get_fleet().ok();
    let detail = format!(
        "pid {}, {}, client protocol {}.{}",
        hello.daemon_pid.map_or("?".into(), |p| p.to_string()),
        hello.daemon_version.as_deref().unwrap_or("agend ?"),
        hello.selected.major,
        hello.selected.minor
    );
    (ok("daemon", detail), fleet)
}

fn git(home: &Path) -> Check {
    let fix = Some(git_fix(cfg!(target_os = "macos")).to_owned());
    let Some(program) = setup::find_on_path("git", &home.join("bin")) else {
        return check("git", CheckStatus::Fail, "not on PATH".into(), fix);
    };
    let line = match setup::version_line(&program) {
        Ok(line) => line,
        Err(e) => return check("git", CheckStatus::Fail, e, fix),
    };
    match parse_git_version(&line) {
        Some(v) if git_is_new_enough(v) => ok("git", format!("git {}.{}.{}", v.0, v.1, v.2)),
        Some(v) => check(
            "git",
            CheckStatus::Fail,
            format!(
                "git {}.{}.{} is older than 2.38 (merge-tree needs it)",
                v.0, v.1, v.2
            ),
            fix,
        ),
        None => check(
            "git",
            CheckStatus::Fail,
            format!("cannot read the version from {line:?}"),
            fix,
        ),
    }
}

fn backend_check(home: &Path, backend: Backend, fleet: Option<&FleetView>) -> Check {
    let name = backend.as_str();
    let fix = Some(backend_install(backend).to_owned());
    let users: Vec<&str> = fleet
        .map(|f| {
            f.instances
                .iter()
                .filter(|i| i.backend == name)
                .map(|i| i.instance_id.as_str())
                .collect()
        })
        .unwrap_or_default();
    let Some(program) = setup::find_on_path(name, &home.join("bin")) else {
        return if users.is_empty() {
            check(
                name,
                CheckStatus::Warn,
                "not on PATH; no instance uses it".into(),
                fix,
            )
        } else {
            check(
                name,
                CheckStatus::Fail,
                format!("not on PATH; used by {}", users.join(", ")),
                fix,
            )
        };
    };
    match setup::version_line(&program) {
        Ok(line) => ok(name, line),
        Err(e) => check(name, CheckStatus::Warn, e, fix),
    }
}

fn holders(home: &Path, fleet: Option<&FleetView>) -> Check {
    let running = files::running_holders(home).unwrap_or_default();
    let Some(fleet) = fleet else {
        if running.is_empty() {
            return ok("holders", "0 running".into());
        }
        return check(
            "holders",
            CheckStatus::Warn,
            format!(
                "{} running; orphans unknown (the daemon is not running)",
                running.len()
            ),
            Some("agend daemon   (its boot sweep stops orphans)".into()),
        );
    };
    let orphans: Vec<&str> = running
        .iter()
        .map(|(id, _)| id.as_str())
        .filter(|id| !fleet.instances.iter().any(|i| i.instance_id == *id))
        .collect();
    if orphans.is_empty() {
        return ok("holders", format!("{} running, 0 orphans", running.len()));
    }
    check(
        "holders",
        CheckStatus::Warn,
        format!(
            "{} running, {} orphan{}: {}",
            running.len(),
            orphans.len(),
            plural(orphans.len()),
            orphans.join(", ")
        ),
        Some("agend daemon restart   (its boot sweep stops orphans)".into()),
    )
}

fn disk(home: &Path) -> Check {
    let free = match setup::free_bytes(home) {
        Ok(free) => free,
        Err(e) => {
            return check(
                "disk",
                CheckStatus::Warn,
                format!("cannot read the free space: {e}"),
                Some(format!("df -h {}", home.display())),
            );
        }
    };
    let used = setup::dir_bytes(home);
    let detail = format!("{} free, home {}", setup::size(free), setup::size(used));
    if free < MIN_FREE_BYTES {
        return check(
            "disk",
            CheckStatus::Fail,
            detail,
            Some("free space on the home's disk (less than 1 GB left)".into()),
        );
    }
    if used > MAX_HOME_BYTES {
        return check(
            "disk",
            CheckStatus::Warn,
            format!("{detail} (more than 20 GB)"),
            Some(format!("du -sh {}/* | sort -h", home.display())),
        );
    }
    ok("disk", detail)
}

/// Local configuration only: doctor never consumes updates or sends a message.
fn telegram(home: &Path) -> Check {
    match agend_daemon::notifier::config::load(home) {
        Ok(None) => ok("telegram", "not configured".into()),
        Ok(Some((config, _token))) if config.allow_user_ids.is_empty() => check(
            "telegram", CheckStatus::Fail,
            "notifications configured; inbound control disabled because allow_user_ids is empty".into(),
            Some("set telegram.allow_user_ids in $AGEND_HOME/config.toml to the permitted human user IDs".into()),
        ),
        Ok(Some((config, _token))) => ok("telegram", format!(
            "local configuration valid; {} allowed human user(s); network delivery not probed",
            config.allow_user_ids.len())),
        Err(reason) => check("telegram", CheckStatus::Fail, reason,
            Some("check the Telegram token reference and private token file permissions in $AGEND_HOME/config.toml".into())),
    }
}

#[cfg(test)]
mod telegram_tests {
    use super::*;
    #[test]
    fn empty_allowlist_fails_and_secret_failures_do_not_echo_configuration() {
        let dir = agend_testkit::tempdir::TempDir::new("telegram-doctor").unwrap();
        assert_eq!(telegram(dir.path()).status, CheckStatus::Ok);
        let token = dir.path().join("token");
        std::fs::write(&token, "123:abcdefghijklmnopqrstuvwxyz_123456789").unwrap();
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600)).unwrap();
        let config = format!(
            "[telegram]\nchat_id=42\ntoken={{kind='file',value='{}'}}\n",
            token.display()
        );
        std::fs::write(dir.path().join("config.toml"), &config).unwrap();
        let check = telegram(dir.path());
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(check.detail.contains("allow_user_ids is empty"));
        std::fs::write(
            dir.path().join("config.toml"),
            format!("{config}allow_user_ids=[7]\n"),
        )
        .unwrap();
        assert_eq!(telegram(dir.path()).status, CheckStatus::Ok);
        std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(telegram(dir.path()).status, CheckStatus::Fail);
        std::fs::write(
            dir.path().join("config.toml"),
            "[telegram]\ntoken='TOP-SECRET'\n",
        )
        .unwrap();
        let check = telegram(dir.path());
        assert_eq!(check.status, CheckStatus::Fail);
        assert!(!format!("{check:?}").contains("TOP-SECRET"));
    }
}
