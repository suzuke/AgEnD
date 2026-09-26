//! End-to-end behaviour of the git guard against real temporary repos (see
//! `shim_common`): a canonical checkout, a daemon-style worktree on
//! `agend/t-1/fix`, and a binding snapshot written with the shim's own
//! `Snapshot` type. Kill-guard behaviour lives in `shim_bypass_corpus.rs`, against
//! a fake kill recorder only.

#![cfg(unix)]

mod shim_common;

use agend_shim::audit;
use agend_shim::binding::snapshot_path;
use agend_shim::ctx::Ctx;
use shim_common::{Fixture, INSTANCE, git, git_base, gitshim, try_git};

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
        (&["checkout", "-b", "feat/x"][..], "branch_switch"),
        (&["switch", "-c", "feat/x"][..], "branch_switch"),
    ] {
        assert_eq!(gitshim(&ctx, cmd).refused, Some(code), "{cmd:?}");
    }
    // Branch writes are the reference-transaction hook's.
    let ran = gitshim(&ctx, &["branch", "feat/x"]);
    assert!(ran.hook_refused(), "{}", ran.text());
    assert!(
        ran.text().contains("only write branches under agend/t-1/"),
        "{}",
        ran.text()
    );
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
        vec!["update-ref", "-d", "refs/heads/master"],
        vec!["push", ".", "HEAD:main"],
        vec!["push", ".", "HEAD:refs/heads/master"],
        vec!["branch", "-f", "master", head.as_str()],
        vec!["branch", "-D", "release"],
    ] {
        let ran = gitshim(&ctx, &cmd);
        assert!(ran.hook_refused(), "{cmd:?}: {}", ran.text());
        assert!(
            ran.text().contains("it is a protected ref"),
            "{cmd:?}: {}",
            ran.text()
        );
        assert!(
            ran.text().contains("next step: "),
            "{cmd:?}: {}",
            ran.text()
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
    let records = audit::read(&f.home);
    assert_eq!(
        records
            .iter()
            .filter(|r| r.code.as_deref() == Some("protected_ref"))
            .count(),
        7,
        "hook refusals are audited: {records:?}"
    );
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

/// Round 8, findings 1–2 (owner decision 2026-09-25): `refs/stash` is one
/// list shared by the canonical checkout and every worktree. The human's
/// stash in the canonical checkout survives every agent stash write: the
/// shim refuses each spelling, and the hook refuses `refs/stash` when real
/// git runs in the agent worktree (shim bypassed). Before: the agent's
/// `stash pop` applied the human's WIP and dropped it; `stash clear`
/// emptied the human's list.
#[test]
fn human_stash_survives_agent_stash_writes() {
    let f = Fixture::new("stash");
    std::fs::write(f.repo.join("README.md"), "HUMAN-WIP\n").unwrap();
    git(&f.repo, &["stash", "push", "-q", "-m", "human"]);
    let human = || {
        let list = git(&f.repo, &["stash", "list", "--format=%H %gs"]);
        assert!(list.ends_with("On main: human"), "{list}");
        list
    };
    let before = human();
    let untouched = |what: &str| {
        assert_eq!(human(), before, "{what}: human's stash list");
        assert_eq!(git(&f.worktree, &["status", "--porcelain"]), "", "{what}");
        assert_eq!(git(&f.repo, &["status", "--porcelain"]), "", "{what}");
    };
    let sha = f.head(&f.repo, "refs/stash");
    let ctx = f.ctx(&f.worktree);
    for cmd in [
        "stash",
        "stash pop",
        "stash apply",
        "stash drop",
        "stash clear",
        "stash push -u",
        "stash save x",
        "stash branch agend/t-1/x",
        "stash create",
    ]
    .iter()
    .map(|c| c.to_string())
    .chain([format!("stash store {sha}")])
    {
        let argv: Vec<&str> = cmd.split_whitespace().collect();
        let ran = gitshim(&ctx, &argv);
        assert_eq!(ran.refused, Some("stash_shared"), "{cmd}: {}", ran.text());
        assert!(
            ran.text().contains("git commit -m \"wip: "),
            "{}",
            ran.text()
        );
        untouched(&cmd);
    }
    let listed = gitshim(&ctx, &["stash", "list"]);
    assert!(String::from_utf8_lossy(&listed.ok().stdout).contains("human"));
    // The agent's own edit is refused too, and stays in its worktree.
    std::fs::write(f.worktree.join("README.md"), "AGENT-WIP\n").unwrap();
    assert_eq!(gitshim(&ctx, &["stash"]).refused, Some("stash_shared"));
    assert_eq!(git(&f.repo, &["stash", "list"]).lines().count(), 1);
    std::fs::write(f.worktree.join("README.md"), "hello\n").unwrap();
    // Shim bypassed: real git in the agent worktree, hooks on, with and
    // without the agent's binding in the environment.
    for agent_env in [true, false] {
        for args in [
            &["stash", "clear"][..],
            &["update-ref", "refs/stash", "HEAD"][..],
            &["update-ref", "-d", "refs/stash"][..],
            &["stash", "store", "-m", "x", &sha][..],
        ] {
            let mut cmd = git_base();
            if agent_env {
                cmd.env("AGEND_HOME", &f.home)
                    .env("AGEND_INSTANCE", INSTANCE);
            }
            let out = cmd.arg("-C").arg(&f.worktree).args(args).output().unwrap();
            let err = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success(), "{args:?} ran: {err}");
            assert!(
                err.contains("agents do not write git stash"),
                "{args:?}: {err}"
            );
            untouched(&format!("real git {args:?}"));
        }
    }
}

/// A new commit on origin's main that edits README.md, fetched, and the
/// agent's uncommitted README.md edit, which conflicts with it when an
/// autostash is applied. Returns a check that nothing changed.
fn upstream_edits_readme(f: &Fixture) -> impl Fn(&str) + '_ {
    std::fs::write(f.repo.join("README.md"), "upstream\n").unwrap();
    git(&f.repo, &["commit", "-q", "-am", "upstream"]);
    git(&f.repo, &["push", "-q", "origin", "main"]);
    git(&f.repo, &["fetch", "-q", "origin"]);
    std::fs::write(f.worktree.join("README.md"), "AGENT-WIP\n").unwrap();
    let head = f.head(&f.worktree, "HEAD");
    move |what: &str| {
        assert_eq!(f.head(&f.worktree, "HEAD"), head, "{what}");
        let readme = std::fs::read_to_string(f.worktree.join("README.md")).unwrap();
        assert_eq!(readme, "AGENT-WIP\n", "{what}");
        let stash = try_git(&f.repo, &["rev-parse", "-q", "--verify", "refs/stash"]);
        assert!(stash.stdout.is_empty(), "{what}: refs/stash written");
    }
}

/// Owner decision 2026-09-25: agents do not autostash. Before: `pull
/// --rebase --autostash` with a conflicting edit moved the branch, then git
/// could not store the autostash (`refs/stash`, refused by the hook) and
/// left conflict markers in README.md. Now refused before git runs.
#[test]
fn autostash_is_refused_before_git_runs() {
    let f = Fixture::new("autostash");
    let unchanged = upstream_edits_readme(&f);
    let ctx = f.ctx(&f.worktree);
    for cmd in [
        "pull --rebase --autostash origin main",
        "pull --autost origin main",
        "rebase --autostash origin/main",
        "merge --autostash origin/main",
        "-c rebase.autoStash=true pull --rebase origin main",
        "-c MERGE.AUTOSTASH=true merge origin/main",
    ] {
        let argv: Vec<&str> = cmd.split_whitespace().collect();
        let ran = gitshim(&ctx, &argv);
        assert_eq!(ran.refused, Some("autostash"), "{cmd}: {}", ran.text());
        assert!(
            ran.text().contains("git commit -m \"wip: "),
            "{}",
            ran.text()
        );
        unchanged(cmd);
    }
}

/// `rebase.autoStash` / `merge.autoStash` in the repo config: a plain
/// `pull --rebase` / `rebase` / `merge` is refused the same way (git would
/// autostash); with `--no-autostash` git itself refuses the dirty worktree
/// (nothing stashed, nothing changed), and runs once the edit is committed.
#[test]
fn config_autostash_is_refused_unless_no_autostash() {
    let f = Fixture::new("autostash-config");
    let unchanged = upstream_edits_readme(&f);
    git(&f.repo, &["config", "rebase.autoStash", "true"]);
    git(&f.repo, &["config", "merge.autoStash", "true"]);
    let ctx = f.ctx(&f.worktree);
    for cmd in [
        "rebase origin/main",
        "pull --rebase origin main",
        "pull origin main",
        "merge origin/main",
    ] {
        let argv: Vec<&str> = cmd.split_whitespace().collect();
        let ran = gitshim(&ctx, &argv);
        assert_eq!(ran.refused, Some("autostash"), "{cmd}: {}", ran.text());
        assert!(ran.text().contains("--no-autostash"), "{}", ran.text());
        unchanged(cmd);
    }
    let ran = gitshim(&ctx, &["rebase", "--no-autostash", "origin/main"]);
    assert_eq!(ran.refused, None, "{}", ran.text());
    assert!(!ran.output.as_ref().unwrap().status.success());
    unchanged("rebase --no-autostash");
    gitshim(&ctx, &["commit", "-q", "-am", "wip: readme"]).ok();
    let ran = gitshim(&ctx, &["rebase", "--no-autostash", "origin/main"]);
    assert_eq!(ran.refused, None, "{}", ran.text());
    assert!(ran.text().contains("could not apply"), "{}", ran.text());
    gitshim(&ctx, &["rebase", "--abort"]).ok();
    assert_eq!(
        git(&f.worktree, &["log", "-1", "--format=%s"]),
        "wip: readme"
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

/// The bypass skips the shim only; the hooks still guard refs.
#[test]
fn bypass_runs_unchecked_and_is_audited() {
    let f = Fixture::new("bypass");
    let ctx = Ctx {
        bypass: true,
        ..f.ctx(&f.worktree)
    };
    gitshim(&ctx, &["checkout", "-q", "-b", "agend/t-1/escape"]).ok();
    assert!(gitshim(&ctx, &["branch", "feat/escape"]).hook_refused());
    let records = audit::read(&f.home);
    assert_eq!(records.len(), 3, "{records:?}");
    assert_eq!(records[0].event, "bypass");
    assert_eq!(records[2].code.as_deref(), Some("ref_not_yours"));
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
