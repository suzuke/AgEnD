//! T7, owner decision 2026-09-25: a push whose destination git takes from
//! config (`git push`, `git push origin`, `git push -u origin <branch>`,
//! `git push origin HEAD`) runs when that destination is the agent's bound
//! branch on the team remote, and is refused (with the exact command that
//! works) when it resolves anywhere else or cannot be resolved.
//!
//! The config is written with the real git (not the shim, which refuses
//! these keys); every push goes to the fixture's bare `origin`, and each
//! refusal is checked against origin's refs, not only the refusal code.
//!
//! Real temporary repos only (`common`); every call names absolute paths
//! and its own cwd.

#![cfg(unix)]

mod common;

use common::{Fixture, git, gitshim, try_git};

fn origin_ref(f: &Fixture, name: &str) -> Option<String> {
    let out = try_git(&f.origin, &["rev-parse", "--verify", "-q", name]);
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn commit(f: &Fixture, msg: &str) -> String {
    git(&f.worktree, &["commit", "-q", "--allow-empty", "-m", msg]);
    git(&f.worktree, &["rev-parse", "HEAD"])
}

fn set(f: &Fixture, key: &str, value: &str) {
    git(&f.repo, &["config", key, value]);
}

#[test]
fn pushes_that_resolve_to_the_bound_branch_run() {
    let f = Fixture::new("push-ok");
    let wt = f.ctx(&f.worktree);
    let head = commit(&f, "c1");
    // First push: `-u origin <branch>` (colon-less, no upstream yet).
    gitshim(&wt, &["push", "-q", "-u", "origin", &f.branch]).ok();
    assert_eq!(origin_ref(&f, &f.branch), Some(head));
    assert_eq!(
        git(&f.worktree, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        format!("origin/{}", f.branch)
    );
    // Then plain `git push` (push.default unset = simple), from the
    // worktree, the canonical checkout and the workspace (both routed).
    for (i, at) in [&f.worktree, &f.repo, &f.workspace].into_iter().enumerate() {
        let head = commit(&f, &format!("plain {i}"));
        gitshim(&f.ctx(at), &["push", "-q"]).ok();
        assert_eq!(
            origin_ref(&f, &f.branch),
            Some(head),
            "from {}",
            at.display()
        );
    }
    for cmd in [
        &["push", "-q", "origin"][..],
        &["push", "-q", "origin", "HEAD"][..],
    ] {
        let head = commit(&f, "more");
        gitshim(&wt, cmd).ok();
        assert_eq!(origin_ref(&f, &f.branch), Some(head), "{cmd:?}");
    }
    for mode in ["simple", "upstream", "current"] {
        set(&f, "push.default", mode);
        let head = commit(&f, mode);
        gitshim(&wt, &["push", "-q"]).ok();
        assert_eq!(origin_ref(&f, &f.branch), Some(head), "push.default={mode}");
    }
    // `current` needs no upstream.
    git(&f.worktree, &["branch", "--unset-upstream"]);
    let head = commit(&f, "current, no upstream");
    gitshim(&wt, &["push", "-q"]).ok();
    assert_eq!(origin_ref(&f, &f.branch), Some(head));
}

#[test]
fn pushes_that_resolve_elsewhere_are_refused_with_the_working_command() {
    let f = Fixture::new("push-refused");
    let wt = f.ctx(&f.worktree);
    let main_before = origin_ref(&f, "main");
    let next = format!(
        "next step: name source and destination: git push origin HEAD:refs/heads/{}",
        f.branch
    );
    let refused = |cmd: &[&str], code: &str, why: &str| {
        let ran = gitshim(&wt, cmd);
        assert_eq!(ran.refused, Some(code), "git {cmd:?}: {}", ran.text());
        assert!(ran.text().contains(why), "git {cmd:?}: {}", ran.text());
        assert!(ran.text().contains(&next), "git {cmd:?}: {}", ran.text());
        assert_eq!(origin_ref(&f, "main"), main_before, "git {cmd:?}");
        assert_eq!(origin_ref(&f, &f.branch), None, "git {cmd:?}");
    };
    commit(&f, "c1");
    // No upstream yet: git itself refuses under `simple`.
    refused(&["push"], "push_explicit", "has no upstream branch");
    // A misconfigured upstream that points at main.
    git(
        &f.worktree,
        &["branch", "-q", "--set-upstream-to=origin/main"],
    );
    refused(&["push"], "push_explicit", "a different name");
    set(&f, "push.default", "upstream");
    refused(&["push"], "protected_ref", "refs/heads/main");
    refused(
        &["push", "origin", "HEAD"],
        "protected_ref",
        "refs/heads/main",
    );
    refused(
        &["push", "-u", "origin", &f.branch],
        "protected_ref",
        "refs/heads/main",
    );
    set(&f, "push.default", "matching");
    refused(&["push"], "push_explicit", "every branch");
    set(&f, "push.default", "nothing");
    refused(&["push"], "push_explicit", "nothing");
    set(&f, "push.default", "current");
    set(&f, "remote.origin.push", "HEAD:refs/heads/main");
    refused(&["push"], "push_explicit", "remote.origin.push");
    git(&f.repo, &["config", "--unset", "remote.origin.push"]);
    // Somewhere that is not the team remote, and another branch. (By path:
    // `git remote add` would write the shared config, and T5 counts every
    // remote of the canonical checkout as the team's.)
    let fork = f.root.join("fork.git");
    let fork_s = fork.to_str().unwrap();
    git(&f.root, &["init", "-q", "--bare", fork_s]);
    refused(
        &["push", fork_s, "HEAD"],
        "push_explicit",
        "not the team remote",
    );
    set(&f, &format!("branch.{}.pushRemote", f.branch), fork_s);
    refused(&["push"], "push_explicit", "not the team remote");
    refused(&["push", "."], "push_explicit", "not the team remote");
    refused(
        &["push", "origin", "main"],
        "push_explicit",
        "would push main",
    );
    assert!(
        !try_git(&fork, &["rev-parse", "--verify", "-q", &f.branch])
            .status
            .success(),
        "nothing reached the fork"
    );
}
