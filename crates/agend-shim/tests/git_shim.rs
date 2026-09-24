//! End-to-end behaviour of the git guard against real temporary repos (see
//! `common`): a canonical checkout, a daemon-style worktree on
//! `agend/t-1/fix`, and a binding snapshot written with the shim's own
//! `Snapshot` type. Kill-guard behaviour lives in `bypass_corpus.rs`, against
//! a fake kill recorder only.

#![cfg(unix)]

mod common;

use agend_shim::audit;
use agend_shim::binding::snapshot_path;
use agend_shim::ctx::Ctx;
use common::{Fixture, INSTANCE, git, gitshim};

#[test]
fn bound_commit_from_workspace_lands_on_the_task_branch() {
    let f = Fixture::new("route");
    let main_before = f.head(&f.repo, "main");
    std::fs::write(f.worktree.join("fix.txt"), "fixed\n").unwrap();
    let ctx = f.ctx(&f.workspace);
    gitshim(&ctx, &["add", "fix.txt"]).ok();
    gitshim(&ctx, &["commit", "-q", "-m", "fix"]).ok();
    assert_eq!(
        git(&f.worktree, &["log", "-1", "--format=%s", &f.branch]),
        "fix"
    );
    assert_eq!(f.head(&f.repo, "main"), main_before, "main unchanged");
    assert_eq!(git(&f.repo, &["status", "--porcelain"]), "");
    let status = gitshim(&ctx, &["status", "--short", "--branch"]);
    let out = String::from_utf8_lossy(&status.ok().stdout).to_string();
    assert!(out.contains(&f.branch), "read routed to worktree: {out}");
}

#[test]
fn bound_commit_in_canonical_is_routed_with_a_note() {
    let f = Fixture::new("canon");
    std::fs::write(f.worktree.join("a.txt"), "a\n").unwrap();
    let ctx = f.ctx(&f.repo);
    let ran = gitshim(&ctx, &["add", "a.txt"]);
    ran.ok();
    assert!(
        ran.text().contains("running in your bound worktree"),
        "{}",
        ran.text()
    );
    gitshim(&ctx, &["commit", "-q", "-m", "a"]).ok();
    let via_c = f.ctx(&f.workspace);
    std::fs::write(f.worktree.join("b.txt"), "b\n").unwrap();
    gitshim(
        &via_c,
        &["-C", f.repo.to_str().unwrap(), "commit", "-q", "-am", "x"],
    );
    assert_eq!(git(&f.repo, &["branch", "--show-current"]), "main");
    assert_eq!(git(&f.repo, &["log", "-1", "--format=%s", "main"]), "init");
    assert_eq!(git(&f.worktree, &["log", "-1", "--format=%s"]), "a");
}

#[test]
fn checkout_main_is_refused_with_next_step_and_audited() {
    let f = Fixture::new("refuse");
    let ctx = f.ctx(&f.worktree);
    let ran = gitshim(&ctx, &["checkout", "main"]);
    assert_eq!(ran.refused, Some("branch_switch"));
    let text = ran.text();
    assert!(text.contains("refused `git checkout main`"), "{text}");
    assert!(text.contains("next step:"), "{text}");
    assert!(text.contains("agend task create"), "{text}");
    assert_eq!(
        git(&f.worktree, &["branch", "--show-current"]),
        f.branch,
        "still on the task branch"
    );
    assert_eq!(
        gitshim(&ctx, &["checkout", "feature"]).refused,
        Some("branch_switch")
    );
    let records = audit::read(&f.home);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event, "refuse");
    assert_eq!(records[0].code.as_deref(), Some("branch_switch"));
    assert_eq!(records[0].argv, ["checkout", "main"]);
    assert_eq!(records[0].instance.as_deref(), Some(INSTANCE));
}

#[test]
fn worktree_and_branch_creation_are_refused() {
    let f = Fixture::new("create");
    let ctx = f.ctx(&f.worktree);
    for (cmd, code) in [
        (&["worktree", "add", "../x"][..], "worktree_managed"),
        (&["checkout", "-b", "feat/x"][..], "branch_create"),
        (&["switch", "-c", "feat/x"][..], "branch_create"),
        (&["branch", "feat/x"][..], "branch_create"),
    ] {
        assert_eq!(gitshim(&ctx, cmd).refused, Some(code), "{cmd:?}");
    }
    gitshim(&ctx, &["branch", "agend/t-1/scratch"]).ok();
    let branches = git(&f.repo, &["branch", "--format=%(refname:short)"]);
    assert!(!branches.contains("feat/x"), "{branches}");
    assert!(branches.contains("agend/t-1/scratch"), "{branches}");
    assert_eq!(git(&f.repo, &["worktree", "list"]).lines().count(), 2);
}

#[test]
fn unbound_and_missing_snapshot_refuse_mutations_but_allow_reads() {
    let f = Fixture::new("unbound");
    f.write_snapshot(false);
    let ctx = f.ctx(&f.repo);
    std::fs::write(f.repo.join("x.txt"), "x\n").unwrap();
    assert_eq!(
        gitshim(&ctx, &["add", "x.txt"]).refused,
        Some("canonical_checkout")
    );
    gitshim(&ctx, &["status", "--short"]).ok();

    std::fs::remove_file(snapshot_path(&f.home, INSTANCE)).unwrap();
    let ran = gitshim(&ctx, &["commit", "-m", "x"]);
    assert_eq!(ran.refused, Some("no_binding"));
    assert!(ran.text().contains("is missing"), "{}", ran.text());
    gitshim(&ctx, &["log", "--oneline"]).ok();

    std::fs::write(snapshot_path(&f.home, INSTANCE), "{ not json").unwrap();
    let ran = gitshim(&ctx, &["reset", "--hard"]);
    assert_eq!(ran.refused, Some("no_binding"));
    assert!(ran.text().contains("malformed"), "{}", ran.text());
    assert!(f.repo.join("x.txt").exists(), "nothing was reset");

    let no_env = Ctx {
        instance: None,
        ..f.ctx(&f.repo)
    };
    assert_eq!(
        gitshim(&no_env, &["add", "x.txt"]).refused,
        Some("no_binding")
    );
}

#[test]
fn protected_refs_are_refused_and_main_does_not_move() {
    let f = Fixture::new("protect");
    let ctx = f.ctx(&f.worktree);
    std::fs::write(f.worktree.join("w.txt"), "w\n").unwrap();
    gitshim(&ctx, &["add", "w.txt"]).ok();
    gitshim(&ctx, &["commit", "-q", "-m", "w"]).ok();
    let before = f.protected_state();
    let head = f.head(&f.worktree, "HEAD");
    for cmd in [
        vec!["update-ref", "refs/heads/main", head.as_str()],
        vec!["update-ref", "refs/heads/release", head.as_str()],
        vec!["push", ".", "HEAD:main"],
        vec!["push", ".", "HEAD:refs/heads/master"],
        vec!["branch", "-f", "main", head.as_str()],
    ] {
        assert_eq!(
            gitshim(&ctx, &cmd).refused,
            Some("protected_ref"),
            "{cmd:?}"
        );
    }
    assert_eq!(
        f.protected_state(),
        before,
        "main/master/release did not move"
    );
    // The agent's own branch may be updated.
    gitshim(
        &ctx,
        &["update-ref", &format!("refs/heads/{}", f.branch), &head],
    )
    .ok();
}

#[test]
fn reset_hard_is_snapshotted_and_restorable() {
    let f = Fixture::new("snap");
    let ctx = f.ctx(&f.workspace);
    std::fs::write(f.worktree.join("keep.txt"), "committed\n").unwrap();
    gitshim(&ctx, &["add", "keep.txt"]).ok();
    gitshim(&ctx, &["commit", "-q", "-m", "keep"]).ok();
    std::fs::write(f.worktree.join("keep.txt"), "uncommitted edit\n").unwrap();
    std::fs::write(f.worktree.join("new.txt"), "untracked\n").unwrap();

    let ran = gitshim(&ctx, &["reset", "--hard", "HEAD~1"]);
    ran.ok();
    let text = ran.text();
    assert!(text.contains("snapshot "), "{text}");
    assert!(!f.worktree.join("keep.txt").exists(), "reset removed it");

    let undo = text
        .lines()
        .find_map(|l| l.strip_prefix("agend-shim: to undo: "))
        .expect("undo line");
    for part in undo.split(" && ") {
        let words: Vec<&str> = part.split_whitespace().skip(1).collect();
        gitshim(&ctx, &words).ok();
    }
    assert_eq!(
        std::fs::read_to_string(f.worktree.join("keep.txt")).unwrap(),
        "uncommitted edit\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.worktree.join("new.txt")).unwrap(),
        "untracked\n"
    );
    assert_eq!(git(&f.worktree, &["log", "-1", "--format=%s"]), "keep");
    let records = audit::read(&f.home);
    assert!(
        records
            .iter()
            .any(|r| r.event == "snapshot" && r.code.as_deref() == Some("reset")),
        "{records:?}"
    );
}

#[test]
fn clean_and_path_checkout_are_snapshotted() {
    let f = Fixture::new("clean");
    let ctx = f.ctx(&f.worktree);
    std::fs::write(f.worktree.join("scratch.txt"), "notes\n").unwrap();
    std::fs::write(f.worktree.join("README.md"), "edited\n").unwrap();
    let ran = gitshim(&ctx, &["clean", "-fd"]);
    ran.ok();
    assert!(
        ran.text().contains("before `git clean -fd`"),
        "{}",
        ran.text()
    );
    assert!(!f.worktree.join("scratch.txt").exists());
    let ran = gitshim(&ctx, &["checkout", "--", "."]);
    ran.ok();
    assert!(ran.text().contains("snapshot "), "{}", ran.text());
    assert_eq!(
        std::fs::read_to_string(f.worktree.join("README.md")).unwrap(),
        "hello\n"
    );
    let snaps = git(
        &f.repo,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/agend/snapshots/",
        ],
    );
    assert_eq!(snaps.lines().count(), 2, "{snaps}");
    let first = snaps.lines().next().unwrap();
    assert_eq!(
        git(&f.repo, &["show", &format!("{first}:scratch.txt")]),
        "notes"
    );
}

#[test]
fn bypass_runs_unchecked_and_is_audited() {
    let f = Fixture::new("bypass");
    let ctx = Ctx {
        bypass: true,
        ..f.ctx(&f.worktree)
    };
    gitshim(&ctx, &["branch", "feat/escape"]).ok();
    let records = audit::read(&f.home);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event, "bypass");
}

#[test]
fn foreign_repos_are_not_guarded() {
    let f = Fixture::new("foreign");
    let scratch = f.workspace.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let ctx = f.ctx(&scratch);
    gitshim(&ctx, &["init", "-q", "-b", "trunk"]).ok();
    gitshim(&ctx, &["checkout", "-q", "-b", "anything"]).ok();
    assert_eq!(git(&scratch, &["branch", "--show-current"]), "anything");
    assert!(audit::read(&f.home).is_empty());
}

#[test]
fn git_dir_of_the_bound_worktree_is_trusted() {
    // Hooks run git with GIT_DIR set to the worktree's git dir.
    let f = Fixture::new("gitdir");
    let gitdir = git(&f.worktree, &["rev-parse", "--absolute-git-dir"]);
    let ctx = Ctx {
        git_dir: Some(gitdir.into()),
        ..f.ctx(&f.worktree)
    };
    std::fs::write(f.worktree.join("h.txt"), "h\n").unwrap();
    gitshim(&ctx, &["add", "h.txt"]).ok();
    let canon = Ctx {
        git_dir: Some(f.repo.join(".git")),
        ..f.ctx(&f.workspace)
    };
    assert_eq!(
        gitshim(&canon, &["add", "h.txt"]).refused,
        Some("git_env_retarget")
    );
}
