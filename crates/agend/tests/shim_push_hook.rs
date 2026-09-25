//! T7 (owner decision 2026-09-25): a push runs when git sends it to the
//! agent's bound branch. The shim no longer resolves destinations from
//! config: git does (`push.default`, upstream, `remote.<r>.push`, pushRemote)
//! and the agend pre-push hook refuses any remote ref other than the bound
//! branch, so `git push`, `git push origin`, `git push -u origin <branch>`
//! and `git push origin HEAD` just run when they land on it.
//!
//! The config is written with the harness git (like a human would); every
//! push goes to the fixture's bare `origin` (or a second bare repo), and
//! each refusal is checked against the refs there, not only the message.
//!
//! Real temporary repos only (`shim_common`); every call names absolute
//! paths and its own cwd.

#![cfg(unix)]

mod shim_common;

use shim_common::{Fixture, git, gitshim, try_git};

fn remote_ref(dir: &std::path::Path, name: &str) -> Option<String> {
    let out = try_git(dir, &["rev-parse", "--verify", "-q", name]);
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
    // First push: `-u origin <branch>` (no upstream yet).
    gitshim(&wt, &["push", "-q", "-u", "origin", &f.branch]).ok();
    assert_eq!(remote_ref(&f.origin, &f.branch), Some(head));
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
            remote_ref(&f.origin, &f.branch),
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
        assert_eq!(remote_ref(&f.origin, &f.branch), Some(head), "{cmd:?}");
    }
    for mode in ["simple", "upstream", "current"] {
        set(&f, "push.default", mode);
        let head = commit(&f, mode);
        gitshim(&wt, &["push", "-q"]).ok();
        assert_eq!(
            remote_ref(&f.origin, &f.branch),
            Some(head),
            "push.default={mode}"
        );
    }
    // `current` needs no upstream.
    git(&f.worktree, &["branch", "--unset-upstream"]);
    let head = commit(&f, "current, no upstream");
    gitshim(&wt, &["push", "-q"]).ok();
    assert_eq!(remote_ref(&f.origin, &f.branch), Some(head.clone()));
    // The bound branch to any remote is the agent's own work.
    let fork = f.root.join("fork.git");
    git(&f.root, &["init", "-q", "--bare", fork.to_str().unwrap()]);
    gitshim(&wt, &["push", "-q", fork.to_str().unwrap(), "HEAD"]).ok();
    assert_eq!(remote_ref(&fork, &f.branch), Some(head));
}

#[test]
fn pushes_that_git_sends_elsewhere_are_refused() {
    let f = Fixture::new("push-refused");
    let wt = f.ctx(&f.worktree);
    let main_before = remote_ref(&f.origin, "main");
    let next = format!(
        "next step: push only your branch: git push origin HEAD:refs/heads/{}",
        f.branch
    );
    let by_hook = |cmd: &[&str], why: &str| {
        let ran = gitshim(&wt, cmd);
        assert!(ran.hook_refused(), "git {cmd:?}: {}", ran.text());
        assert!(ran.text().contains(why), "git {cmd:?}: {}", ran.text());
        assert!(ran.text().contains(&next), "git {cmd:?}: {}", ran.text());
        assert_eq!(remote_ref(&f.origin, "main"), main_before, "git {cmd:?}");
        assert_eq!(remote_ref(&f.origin, &f.branch), None, "git {cmd:?}");
    };
    let by_git = |cmd: &[&str]| {
        let ran = gitshim(&wt, cmd);
        assert!(
            ran.refused.is_none() && !ran.hook_refused(),
            "{}",
            ran.text()
        );
        assert!(!ran.output.unwrap().status.success(), "git {cmd:?} ran");
        assert_eq!(remote_ref(&f.origin, &f.branch), None, "git {cmd:?}");
    };
    commit(&f, "c1");
    // No upstream, or an upstream with another name: git itself refuses
    // under `simple`.
    by_git(&["push"]);
    git(
        &f.worktree,
        &["branch", "-q", "--set-upstream-to=origin/main"],
    );
    by_git(&["push"]);
    // A misconfigured upstream that points at main.
    set(&f, "push.default", "upstream");
    by_hook(
        &["push"],
        "refs/heads/main is refused: it is a protected ref",
    );
    // matching: every branch that exists on both sides and differs.
    set(&f, "push.default", "matching");
    git(&f.worktree, &["update-ref", "refs/heads/master", "HEAD"]);
    by_hook(&["push"], "refs/heads/master");
    set(&f, "push.default", "current");
    set(&f, "remote.origin.push", "HEAD:refs/heads/main");
    by_hook(&["push"], "refs/heads/main");
    git(&f.repo, &["config", "--unset", "remote.origin.push"]);
    // Another agent's branch, a tag, deleting the bound branch.
    by_hook(
        &["push", "origin", "HEAD:refs/heads/agend/t-2/x"],
        "agend/t-2/x",
    );
    by_hook(&["push", "origin", "HEAD:refs/tags/v1"], "refs/tags/v1");
    by_hook(&["push", "origin", "master"], "refs/heads/master");
    assert_eq!(
        remote_ref(&f.origin, "refs/heads/agend/t-2/x"),
        None,
        "other agent's branch"
    );
    // A colon-less refspec goes to the same name (git 2.39 does not map
    // `HEAD` through the upstream; the old shim's resolver guessed main).
    set(&f, "push.default", "upstream");
    gitshim(&wt, &["push", "-q", "origin", "HEAD"]).ok();
    let pushed = remote_ref(&f.origin, &f.branch);
    assert_eq!(pushed, Some(git(&f.worktree, &["rev-parse", "HEAD"])));
    let ran = gitshim(&wt, &["push", "origin", "--delete", &f.branch]);
    assert!(ran.hook_refused(), "{}", ran.text());
    assert_eq!(remote_ref(&f.origin, &f.branch), pushed, "not deleted");
}
