//! `cargo xtask check-deps`: enforces the crate-boundary rules.
//!
//! 1. Dependency rules for other crates: each rule's crate must not have a
//!    denied crate in its normal + build dependency tree (`cargo tree -e
//!    normal,build --target all`; dev-dependencies are not checked). A deny
//!    entry ending in `*` is a name prefix.
//! 2. `agend-testkit` is never a normal dependency of any crate.
//! 3. agend-core structural checks (`check_core`): no build script, only its
//!    reviewed no-std serde dependency configuration, and it compiles for a
//!    target that has no std at all.
use crate::{cargo, workspace_root};
use std::process::Command;

/// Async runtimes: must not be linked by pure or light-startup crates.
const ASYNC_RUNTIME: &[&str] = &["tokio", "tokio-*", "async-std", "smol", "mio"];
/// Databases: only the daemon's store touches SQLite.
const DATABASE: &[&str] = &["rusqlite", "libsqlite3-sys", "sqlx", "sqlx-*"];
pub struct Rule {
    pub krate: &'static str,
    pub deny: &'static [&'static [&'static str]],
    pub why: &'static str,
}

pub const RULES: &[Rule] = &[
    Rule {
        krate: "agend-shim",
        deny: &[ASYNC_RUNTIME, DATABASE, &["agend-daemon"]],
        why: "shim startup must stay light: no runtime, no DB (plan 4.7)",
    },
    Rule {
        krate: "agend-client",
        deny: &[ASYNC_RUNTIME, DATABASE, &["agend-daemon"]],
        why: "CLI startup must stay light: sync I/O only (D11, plan 4.7)",
    },
    Rule {
        krate: "agend-holder",
        deny: &[ASYNC_RUNTIME, DATABASE, &["agend-daemon"]],
        why: "the holder runs for days on std threads and never opens the DB (gate 4 P1)",
    },
    Rule {
        krate: "agend-tui",
        deny: &[DATABASE, &["agend-daemon"]],
        why: "only the daemon opens agend.db; the TUI goes through the daemon protocol (gate 5 P2)",
    },
    Rule {
        krate: "agend-daemon",
        deny: &[&["agend-holder", "agend-shim"]],
        why: "the daemon reaches holders only through the holder protocol and the `agend holder` subcommand (gate 6 P9)",
    },
];

/// Crates that may depend on `agend-testkit` only as a dev-dependency: all of them.
const TESTKIT: &str = "agend-testkit";

pub fn run(allow_skip: bool) -> Result<(), String> {
    let members = workspace_members()?;
    let mut problems = Vec::new();

    for rule in RULES {
        let names = dependencies(rule.krate, "normal,build")?;
        for v in violations(rule, &names) {
            problems.push(format!(
                "{} depends on {v} ({}); inspect with `cargo tree -e normal,build -p {} -i {v}`",
                rule.krate, rule.why, rule.krate
            ));
        }
    }

    for member in members.iter().filter(|m| m.as_str() != TESTKIT) {
        if dependencies(member, "normal")?.iter().any(|n| n == TESTKIT) {
            problems.push(format!(
                "{member} has {TESTKIT} as a normal dependency; it is dev-only (D10)"
            ));
        }
    }

    let (core_problems, skipped) = crate::check_core::run(allow_skip)?;
    problems.extend(core_problems);

    if problems.is_empty() {
        let core = if skipped {
            "agend-core metadata ok, no-std build SKIPPED"
        } else {
            "agend-core metadata ok, no-std build ok"
        };
        println!(
            "check-deps: ok ({} rules, {} crates checked for {TESTKIT}, {core})",
            RULES.len(),
            members.len() - 1
        );
        Ok(())
    } else {
        for p in &problems {
            eprintln!("check-deps: {p}");
        }
        Err(format!("{} dependency rule violation(s)", problems.len()))
    }
}

/// Denied crate names found in `names` (the crate itself is ignored).
pub fn violations(rule: &Rule, names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = names
        .iter()
        .filter(|n| n.as_str() != rule.krate)
        .filter(|n| {
            rule.deny
                .iter()
                .flat_map(|g| g.iter())
                .any(|p| matches(p, n))
        })
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// Package names from `cargo tree --prefix none --format {p}` output
/// (`name vX.Y.Z (source)`, possibly followed by `(*)`).
pub fn parse_tree_names(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

fn dependencies(krate: &str, edges: &str) -> Result<Vec<String>, String> {
    let out = Command::new(cargo())
        .current_dir(workspace_root())
        .args(["tree", "--quiet", "-e", edges, "--target", "all"])
        .args(["--prefix", "none", "--format", "{p}", "-p", krate])
        .output()
        .map_err(|e| format!("cannot run cargo tree: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo tree -p {krate} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(parse_tree_names(&String::from_utf8_lossy(&out.stdout)))
}

fn workspace_members() -> Result<Vec<String>, String> {
    // Every workspace member is the root line of its own `cargo tree --depth 0`.
    let out = Command::new(cargo())
        .current_dir(workspace_root())
        .args(["tree", "--quiet", "--workspace", "--depth", "0"])
        .args(["--prefix", "none", "--format", "{p}"])
        .output()
        .map_err(|e| format!("cannot run cargo tree: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo tree --workspace failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let mut names = parse_tree_names(&String::from_utf8_lossy(&out.stdout));
    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shim_rule() -> &'static Rule {
        RULES.iter().find(|r| r.krate == "agend-shim").unwrap()
    }

    #[test]
    fn parses_real_cargo_tree_output() {
        // Produced by the real producer, not hand-written (#1493).
        let out = Command::new(cargo())
            .current_dir(workspace_root())
            .args(["tree", "--quiet", "-e", "normal", "--prefix", "none"])
            .args(["--format", "{p}", "-p", "agend-tui"])
            .output()
            .unwrap();
        assert!(out.status.success());
        let names = parse_tree_names(&String::from_utf8(out.stdout).unwrap());
        assert!(names.contains(&"agend-tui".to_string()));
        assert!(names.contains(&"agend-client".to_string()));
        assert!(names.contains(&"agend-core".to_string()));
    }

    #[test]
    fn denies_runtime_and_prefix_matches() {
        let names: Vec<String> = [
            "agend-shim",
            "agend-core",
            "serde",
            "tokio",
            "tokio-util",
            "agend-daemon",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            violations(shim_rule(), &names),
            ["agend-daemon", "tokio", "tokio-util"]
        );
    }

    #[test]
    fn holder_denies_runtimes_databases_and_the_daemon() {
        let rule = RULES.iter().find(|r| r.krate == "agend-holder").unwrap();
        let names: Vec<String> = ["agend-holder", "polling", "mio", "rusqlite", "agend-daemon"]
            .map(String::from)
            .to_vec();
        // `polling` (pulled in by alacritty_terminal's unused event loop) is not a runtime.
        assert_eq!(
            violations(rule, &names),
            ["agend-daemon", "mio", "rusqlite"]
        );
    }

    #[test]
    fn tui_may_not_reach_sqlite_or_the_daemon() {
        let tui = RULES.iter().find(|r| r.krate == "agend-tui").unwrap();
        let names: Vec<String> = [
            "agend-tui",
            "agend-client",
            "rusqlite",
            "libsqlite3-sys",
            "agend-daemon",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            violations(tui, &names),
            ["agend-daemon", "libsqlite3-sys", "rusqlite"]
        );
    }

    #[test]
    fn the_daemon_may_not_link_the_holder_or_the_shim() {
        let daemon = RULES.iter().find(|r| r.krate == "agend-daemon").unwrap();
        let names: Vec<String> = [
            "agend-daemon",
            "agend-core",
            "tokio",
            "rusqlite",
            "agend-holder",
            "agend-shim",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(violations(daemon, &names), ["agend-holder", "agend-shim"]);
    }

    #[test]
    fn a_crate_is_not_a_violation_of_its_own_rule() {
        assert!(violations(shim_rule(), &["agend-shim".to_string()]).is_empty());
    }

    #[test]
    fn current_workspace_passes() {
        // allow_skip: under a non-rustup cargo the no-std target may be missing;
        // CI runs `cargo xtask check-deps` without --allow-skip.
        run(true).unwrap();
    }
}
