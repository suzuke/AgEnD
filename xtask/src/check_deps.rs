//! `cargo xtask check-deps`: enforces the crate-boundary rules.
//!
//! For each rule, the crate's normal dependency tree (`cargo tree -e normal
//! --target all`, so build- and dev-dependencies are excluded and every
//! platform is included) must not contain a denied crate. A deny entry ending
//! in `*` is a name prefix. In addition, `agend-core`'s sources must not use
//! `std::process` or `std::net`.

use crate::{cargo, workspace_root};
use std::path::Path;
use std::process::Command;

/// Async runtimes: must not be linked by pure or light-startup crates.
const ASYNC_RUNTIME: &[&str] = &["tokio", "tokio-*", "async-std", "smol", "mio"];
/// Databases: only the daemon's store touches SQLite.
const DATABASE: &[&str] = &["rusqlite", "libsqlite3-sys", "sqlx", "sqlx-*"];
/// Network clients/servers.
const NETWORK: &[&str] = &[
    "hyper",
    "hyper-*",
    "reqwest",
    "ureq",
    "h2",
    "socket2",
    "tungstenite",
    "tokio-tungstenite",
    "teloxide",
];
/// Process, PTY and signal handling.
const PROCESS: &[&str] = &["portable-pty", "nix", "signal-hook", "signal-hook-*"];

pub struct Rule {
    pub krate: &'static str,
    pub deny: &'static [&'static [&'static str]],
    pub why: &'static str,
}

pub const RULES: &[Rule] = &[
    Rule {
        krate: "agend-core",
        deny: &[ASYNC_RUNTIME, DATABASE, NETWORK, PROCESS, &["agend-*"]],
        why: "agend-core is pure logic and the root of the dependency graph",
    },
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
];

/// Crates that may depend on `agend-testkit` only as a dev-dependency: all of them.
const TESTKIT: &str = "agend-testkit";

pub fn run() -> Result<(), String> {
    let members = workspace_members()?;
    let mut problems = Vec::new();

    for rule in RULES {
        let names = normal_dependencies(rule.krate)?;
        for v in violations(rule, &names) {
            problems.push(format!(
                "{} depends on {v} ({}); inspect with `cargo tree -e normal -p {} -i {v}`",
                rule.krate, rule.why, rule.krate
            ));
        }
    }

    for member in members.iter().filter(|m| m.as_str() != TESTKIT) {
        if normal_dependencies(member)?.iter().any(|n| n == TESTKIT) {
            problems.push(format!(
                "{member} has {TESTKIT} as a normal dependency; it is dev-only (D10)"
            ));
        }
    }

    let core_src = workspace_root().join("crates/agend-core/src");
    problems.extend(forbidden_std_uses(&core_src)?);

    if problems.is_empty() {
        println!(
            "check-deps: ok ({} rules, {} crates checked for {TESTKIT})",
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

fn normal_dependencies(krate: &str) -> Result<Vec<String>, String> {
    let out = Command::new(cargo())
        .current_dir(workspace_root())
        .args(["tree", "--quiet", "-e", "normal", "--target", "all"])
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

/// Non-comment lines in `dir` (recursively, `.rs` files) that use
/// `std::process` or `std::net`, including grouped imports like
/// `use std::{net, process}`.
pub fn forbidden_std_uses(dir: &Path) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            found.extend(forbidden_std_uses(&path)?);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            for (i, line) in text.lines().enumerate() {
                if line_uses_forbidden_std(line) {
                    found.push(format!(
                        "{}:{}: agend-core must not use std::process or std::net",
                        path.display(),
                        i + 1
                    ));
                }
            }
        }
    }
    Ok(found)
}

fn line_uses_forbidden_std(line: &str) -> bool {
    let code = line.trim_start();
    if code.starts_with("//") || !code.contains("std::") {
        return false;
    }
    code.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == "process" || word == "net")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_rule() -> &'static Rule {
        RULES.iter().find(|r| r.krate == "agend-core").unwrap()
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
        let names: Vec<String> = ["agend-core", "serde", "tokio", "tokio-util", "agend-daemon"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            violations(core_rule(), &names),
            ["agend-daemon", "tokio", "tokio-util"]
        );
    }

    #[test]
    fn a_crate_is_not_a_violation_of_its_own_rule() {
        assert!(violations(core_rule(), &["agend-core".to_string()]).is_empty());
    }

    #[test]
    fn flags_process_and_net_uses_but_not_comments() {
        assert!(line_uses_forbidden_std("use std::process::Command;"));
        assert!(line_uses_forbidden_std(
            "    let s = std::net::TcpStream::connect(a);"
        ));
        assert!(line_uses_forbidden_std("use std::{fmt, process};"));
        assert!(!line_uses_forbidden_std(
            "//! Must NOT: use `std::process`."
        ));
        assert!(!line_uses_forbidden_std("use std::fmt;"));
        assert!(!line_uses_forbidden_std("let network = 1;"));
    }

    #[test]
    fn current_workspace_passes() {
        run().unwrap();
    }
}
