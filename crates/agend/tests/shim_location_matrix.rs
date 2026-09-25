//! Verifier round 3: the shim asks the real git where a call acts, so every
//! spelling of a location resolves alike. Table-driven over
//! spellings × work tree given or not × cwd × command:
//! - every protected action is refused (by the shim, or by the agend hook
//!   in the bound worktree), and a destructive one is refused or
//!   snapshotted before it runs; nothing outside the bound worktree changes;
//! - normal work passes with every spelling that names the bound worktree.
//!
//! Real temporary repos only (`git init` / `git clone`, via `shim_common`).

#![cfg(unix)]

mod shim_common;

use agend_shim::ctx::Ctx;
use shim_common::{Fixture, Ran, git, gitshim, try_git};
use std::path::{Path, PathBuf};

/// How the call names the repo.
#[derive(Debug, Clone, Copy)]
enum Spelling {
    Plain,
    /// `-C <worktree>/sub`
    CSubdir,
    /// `-C <worktree>`
    CWorktree,
    /// `--git-dir=<worktree>/.git` (the gitfile)
    GitDirFile,
    /// `--git-dir=<the real git dir>`
    GitDirReal,
    /// `GIT_DIR=<worktree>/.git`
    EnvGitDirFile,
    /// `GIT_DIR=<the real git dir>`
    EnvGitDirReal,
}

const SPELLINGS: &[Spelling] = &[
    Spelling::Plain,
    Spelling::CSubdir,
    Spelling::CWorktree,
    Spelling::GitDirFile,
    Spelling::GitDirReal,
    Spelling::EnvGitDirFile,
    Spelling::EnvGitDirReal,
];

/// Where the agent runs it.
#[derive(Debug, Clone, Copy)]
enum From {
    Worktree,
    Canonical,
    Workspace,
    Subdir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Refused,
    /// Refused, or run after a snapshot of the bound worktree.
    RefusedOrSnapshot,
}

const PROTECTED: &[(&str, Expect)] = &[
    ("push origin HEAD:main", Expect::Refused),
    ("update-ref refs/heads/main HEAD", Expect::Refused),
    ("branch -D release", Expect::Refused),
    ("checkout -b x", Expect::Refused),
    ("reset --hard", Expect::RefusedOrSnapshot),
    ("clean -fd", Expect::RefusedOrSnapshot),
];

fn subdir(f: &Fixture) -> PathBuf {
    f.worktree.join("sub")
}

fn gitdir(f: &Fixture) -> String {
    git(&f.worktree, &["rev-parse", "--absolute-git-dir"])
}

/// The ctx and global options for one spelling from one cwd.
fn call(f: &Fixture, s: Spelling, work_tree: bool, from: From) -> (Ctx, Vec<String>) {
    let cwd = match from {
        From::Worktree => f.worktree.clone(),
        From::Canonical => f.repo.clone(),
        From::Workspace => f.workspace.clone(),
        From::Subdir => subdir(f),
    };
    let mut ctx = f.ctx(&cwd);
    let wt = f.worktree.display().to_string();
    let mut globals: Vec<String> = match s {
        Spelling::Plain => vec![],
        Spelling::CSubdir => vec!["-C".into(), subdir(f).display().to_string()],
        Spelling::CWorktree => vec!["-C".into(), wt.clone()],
        Spelling::GitDirFile => vec![format!("--git-dir={wt}/.git")],
        Spelling::GitDirReal => vec![format!("--git-dir={}", gitdir(f))],
        Spelling::EnvGitDirFile => {
            ctx.git_dir = Some(f.worktree.join(".git"));
            vec![]
        }
        Spelling::EnvGitDirReal => {
            ctx.git_dir = Some(PathBuf::from(gitdir(f)));
            vec![]
        }
    };
    if work_tree {
        globals.push(format!("--work-tree={wt}"));
    }
    (ctx, globals)
}

fn run(ctx: &Ctx, globals: &[String], cmd: &str) -> Ran {
    let argv: Vec<&str> = globals
        .iter()
        .map(String::as_str)
        .chain(cmd.split_whitespace())
        .collect();
    gitshim(ctx, &argv)
}

/// Uncommitted work everywhere a misdirected command could land.
fn dirty(f: &Fixture) {
    std::fs::write(f.repo.join("canon-wip.txt"), "canonical wip\n").unwrap();
    std::fs::write(f.repo.join("README.md"), "canonical edit\n").unwrap();
    std::fs::write(f.workspace.join("ws-notes.txt"), "notes\n").unwrap();
    std::fs::create_dir_all(subdir(f)).unwrap();
    std::fs::write(subdir(f).join("keep.txt"), "keep\n").unwrap();
    std::fs::write(f.worktree.join("wip.txt"), "worktree wip\n").unwrap();
    std::fs::write(f.worktree.join("README.md"), "worktree edit\n").unwrap();
}

fn snapshots(f: &Fixture) -> usize {
    git(
        &f.repo,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/agend/snapshots/",
        ],
    )
    .lines()
    .count()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

fn violations(
    f: &Fixture,
    before: &str,
    snaps_before: usize,
    expect: Expect,
    ran: &Ran,
) -> Vec<String> {
    let mut bad = Vec::new();
    let snapped = snapshots(f) > snaps_before;
    let refused = ran.is_refused();
    match (expect, ran.refused) {
        (Expect::Refused, _) if !refused => bad.push("was not refused (shim or hook)".into()),
        (Expect::RefusedOrSnapshot, _) if !refused && !snapped => {
            bad.push("ran without a snapshot".into())
        }
        // Every spelling here acts on the team repo's own worktree; calling
        // it "a clone of the team repo" sends the agent the wrong way.
        (_, Some("team_clone")) => bad.push("refused as a team clone".into()),
        _ => {}
    }
    if f.protected_state() != before {
        bad.push(format!("protected refs moved:\n{}", f.protected_state()));
    }
    let head = try_git(&f.worktree, &["symbolic-ref", "-q", "HEAD"]);
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if head != format!("refs/heads/{}", f.branch) {
        bad.push(format!("worktree HEAD is {head:?}"));
    }
    for b in git(
        &f.repo,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )
    .lines()
    {
        if !matches!(b, "main" | "master" | "release" | "feature") && !b.starts_with("agend/t-1/") {
            bad.push(format!("new branch {b}"));
        }
    }
    if !f.repo.join("canon-wip.txt").exists()
        || read(&f.repo.join("README.md")) != "canonical edit\n"
    {
        bad.push("canonical checkout lost uncommitted work".into());
    }
    if !f.workspace.join("ws-notes.txt").exists() {
        bad.push("workspace lost a file".into());
    }
    let intact = f.worktree.join("wip.txt").exists()
        && subdir(f).join("keep.txt").exists()
        && read(&f.worktree.join("README.md")) == "worktree edit\n";
    if !intact && !snapped {
        bad.push("worktree lost uncommitted work without a snapshot".into());
    }
    bad
}

/// Every spelling × work tree given or not × every protected command, from
/// `from`. Returns (cases run, failures).
fn protected_matrix(from: From) -> (usize, Vec<String>) {
    let mut failures = Vec::new();
    let mut n = 0;
    for &s in SPELLINGS {
        for work_tree in [false, true] {
            let f = Fixture::new("matrix");
            std::fs::write(f.worktree.join("ahead.txt"), "ahead\n").unwrap();
            git(&f.worktree, &["add", "ahead.txt"]);
            git(&f.worktree, &["commit", "-q", "-m", "ahead"]);
            let (ctx, globals) = {
                dirty(&f);
                call(&f, s, work_tree, from)
            };
            for &(cmd, expect) in PROTECTED {
                n += 1;
                dirty(&f);
                let before = f.protected_state();
                let snaps = snapshots(&f);
                let ran = run(&ctx, &globals, cmd);
                let bad = violations(&f, &before, snaps, expect, &ran);
                if !bad.is_empty() {
                    failures.push(format!(
                        "from {from:?}, {s:?}, work tree {work_tree}: `git {} {cmd}`\n  - {}\n  shim said: {}",
                        globals.join(" "),
                        bad.join("\n  - "),
                        ran.text()
                    ));
                }
            }
        }
    }
    (n, failures)
}

fn assert_protected(from: From) {
    let (n, failures) = protected_matrix(from);
    assert_eq!(n, SPELLINGS.len() * 2 * PROTECTED.len());
    assert!(
        failures.is_empty(),
        "{} of {n} cases broke the guard:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn protected_actions_from_the_worktree() {
    assert_protected(From::Worktree);
}

#[test]
fn protected_actions_from_the_canonical_checkout() {
    assert_protected(From::Canonical);
}

#[test]
fn protected_actions_from_the_workspace() {
    assert_protected(From::Workspace);
}

#[test]
fn protected_actions_from_a_worktree_subdir() {
    assert_protected(From::Subdir);
}

/// Spellings that name the bound worktree with its own work tree.
const NAMING_THE_WORKTREE: &[(Spelling, bool)] = &[
    (Spelling::Plain, false),
    (Spelling::CWorktree, false),
    (Spelling::CSubdir, false),
    (Spelling::GitDirFile, true),
    (Spelling::GitDirReal, true),
    (Spelling::EnvGitDirFile, true),
    (Spelling::EnvGitDirReal, true),
];

/// Normal work passes with every spelling that names the bound worktree,
/// from every cwd, and git's hooks run.
#[test]
fn normal_work_passes_with_every_spelling() {
    let mut failures = Vec::new();
    let mut n = 0;
    for from in [
        From::Worktree,
        From::Canonical,
        From::Workspace,
        From::Subdir,
    ] {
        for &(s, work_tree) in NAMING_THE_WORKTREE {
            let f = Fixture::new("matrix-ok");
            std::fs::create_dir_all(subdir(&f)).unwrap();
            // origin/main one commit ahead, for the rebase.
            git(
                &f.repo,
                &["commit", "-q", "--allow-empty", "-m", "upstream"],
            );
            git(&f.repo, &["push", "-q", "origin", "main"]);
            git(&f.worktree, &["fetch", "-q", "origin"]);
            let marker = f.root.join("hooks.log");
            let hooks = f.repo.join(".git").join("hooks");
            std::fs::create_dir_all(&hooks).unwrap();
            for hook in ["pre-commit", "post-commit"] {
                let p = hooks.join(hook);
                std::fs::write(
                    &p,
                    format!("#!/bin/sh\necho {hook} >> '{}'\n", marker.display()),
                )
                .unwrap();
                std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                    .unwrap();
            }
            let (ctx, globals) = call(&f, s, work_tree, from);
            let own = format!("HEAD:refs/heads/{}", f.branch);
            let steps = [
                "commit -q --allow-empty -m n1".to_string(),
                "-c core.editor=true commit -q --amend --allow-empty".into(),
                format!("push -q origin {own}"),
                "rebase -q origin/main".into(),
                "stash -q".into(),
                "stash pop -q".into(),
            ];
            for cmd in &steps {
                n += 1;
                if cmd == "stash -q" {
                    std::fs::write(f.worktree.join("README.md"), "stash me\n").unwrap();
                }
                // `-c` is a global option: keep it before the spelling's.
                let (pre, cmd) = match cmd.strip_prefix("-c core.editor=true ") {
                    Some(rest) => (vec!["-c".to_string(), "core.editor=true".into()], rest),
                    None => (vec![], cmd.as_str()),
                };
                let all: Vec<String> = pre.into_iter().chain(globals.iter().cloned()).collect();
                let ran = run(&ctx, &all, cmd);
                let ok = ran.refused.is_none()
                    && ran.output.as_ref().is_some_and(|o| o.status.success());
                if !ok {
                    let stderr = ran
                        .output
                        .as_ref()
                        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
                        .unwrap_or_default();
                    failures.push(format!(
                        "from {from:?}, {s:?}: `git {} {cmd}`: {} {stderr}",
                        all.join(" "),
                        ran.text()
                    ));
                }
            }
            if read(&f.worktree.join("README.md")) != "stash me\n" {
                failures.push(format!("from {from:?}, {s:?}: stash pop lost the change"));
            }
            let origin_own = try_git(&f.origin, &["rev-parse", "-q", "--verify", &f.branch]);
            if !origin_own.status.success() {
                failures.push(format!("from {from:?}, {s:?}: own branch not on origin"));
            }
            let log = read(&marker);
            if log.matches("pre-commit").count() < 2 || !log.contains("post-commit") {
                failures.push(format!("from {from:?}, {s:?}: hooks did not run: {log:?}"));
            }
        }
    }
    assert_eq!(n, 4 * NAMING_THE_WORKTREE.len() * 6);
    assert!(
        failures.is_empty(),
        "{} of {n} normal steps failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Hooks call git with `GIT_DIR` (and during a commit `GIT_INDEX_FILE`) set
/// to the worktree's own; the cwd is the worktree. Both spellings pass.
#[test]
fn hook_style_calls_pass() {
    let f = Fixture::new("matrix-hook");
    for dir in [PathBuf::from(gitdir(&f)), f.worktree.join(".git")] {
        let ctx = Ctx {
            git_dir: Some(dir.clone()),
            git_index_file: Some(PathBuf::from(gitdir(&f)).join("index")),
            ..f.ctx(&f.worktree)
        };
        std::fs::write(f.worktree.join("h.txt"), "h\n").unwrap();
        gitshim(&ctx, &["add", "h.txt"]).ok();
        gitshim(&ctx, &["diff", "--cached", "--quiet"]);
        gitshim(&ctx, &["reset", "-q", "--hard"]).ok();
    }
}
