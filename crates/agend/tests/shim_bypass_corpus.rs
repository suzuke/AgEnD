//! The verifier's bypass corpus (PR #107 rounds 1–6), replayed against
//! real temporary repos with the agend hooks installed. Each case runs
//! optional setup with the harness's git (state that could exist before the
//! agent acts), then one command through the shim, then checks the
//! invariants the guard exists for:
//! - main / master / release did not move, locally or on origin;
//! - the bound worktree is still on `agend/t-1/fix`, which is a real branch;
//! - no branch outside `agend/t-1/` appeared;
//! - the canonical checkout's uncommitted work is intact;
//! - the worktree's uncommitted work is intact, or a snapshot holds it.
//!
//! Since the hook refactor most ref cases are refused by an agend hook
//! (git reports the real destination), not by the shim: `Refused` accepts
//! either, and nothing else (a git error is not a refusal). Setups make
//! each case really try to move a ref (an up-to-date ref is not a write).
//!
//! Kill forms run against a fake `kill` recorder only (never a real kill),
//! with targets that are this test's own children.

#![cfg(unix)]

mod shim_common;

use agend_shim::Tool;
use agend_shim::ctx::Ctx;
use shim_common::{Fixture, git, gitshim, shim, shim_input, try_git};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Refused by the shim or by an agend hook.
    Refused,
    /// The shim runs it, after taking a snapshot.
    Snapshot,
    /// Runs, and changes nothing the invariants guard (a config write whose
    /// effect is refused in the case that follows it).
    Harmless,
    /// git itself refuses first (it never fetches into a branch checked out
    /// in any worktree, here the canonical `main`).
    GitRefuses,
}

struct Case {
    /// Harness git commands, run in the worktree before the shim call.
    setup: &'static [&'static str],
    /// The command given to the shim (whitespace split; placeholders
    /// `{repo}`, `{origin}`, `{head}`).
    cmd: &'static str,
    /// `GIT_WORK_TREE` / `GIT_INDEX_FILE` for the shim call, if any.
    env: Option<(&'static str, &'static str)>,
    /// stdin of the real git.
    input: &'static str,
    expect: Expect,
}

const fn case(setup: &'static [&'static str], cmd: &'static str, expect: Expect) -> Case {
    Case {
        setup,
        cmd,
        env: None,
        input: "",
        expect,
    }
}

const fn refused(cmd: &'static str) -> Case {
    case(&[], cmd, Expect::Refused)
}

const fn after(setup: &'static [&'static str], cmd: &'static str) -> Case {
    case(setup, cmd, Expect::Refused)
}

const fn harmless(cmd: &'static str) -> Case {
    case(&[], cmd, Expect::Harmless)
}

const fn snapshot(cmd: &'static str) -> Case {
    case(&[], cmd, Expect::Snapshot)
}

const fn with_env(env: (&'static str, &'static str), cmd: &'static str) -> Case {
    Case {
        env: Some(env),
        ..refused(cmd)
    }
}

/// origin's main one commit ahead (the worktree's `ahead`), so a fetch
/// that maps it onto a local branch really moves that branch.
const ORIGIN_AHEAD: &str = "push -q origin HEAD:refs/heads/main";

/// Class 1: destinations that come from config, `-c`, `--refmap` or `pull`.
/// The shim no longer reads config: git resolves the destination and the
/// pre-push / reference-transaction hook refuses it.
const CONFIG_DESTINATIONS: &[Case] = &[
    refused("-c remote.origin.push=HEAD:refs/heads/main push origin"),
    harmless("config remote.origin.push +HEAD:refs/heads/master"),
    after(
        &["config remote.origin.push +HEAD:refs/heads/master"],
        "push origin",
    ),
    after(
        &["config remote.origin.push refs/heads/agend/t-1/fix:refs/heads/main"],
        "push origin agend/t-1/fix",
    ),
    after(
        &[
            "config branch.agend/t-1/fix.merge refs/heads/main",
            "config branch.agend/t-1/fix.remote origin",
        ],
        "-c push.default=upstream push",
    ),
    after(
        &[
            "config branch.agend/t-1/fix.merge refs/heads/main",
            "config branch.agend/t-1/fix.remote origin",
            "config push.default upstream",
        ],
        "push origin agend/t-1/fix",
    ),
    // matching pushes local branches that differ from origin's.
    after(
        &["update-ref refs/heads/master HEAD"],
        "-c push.default=matching push origin",
    ),
    harmless("config branch.agend/t-1/fix.merge refs/heads/main"),
    harmless("config push.default upstream"),
    // The worktree's own `core.hooksPath` (config.worktree) wins over the
    // shared config: the agend hook still runs.
    harmless("config core.hooksPath hooks"),
    after(
        &["config core.hooksPath hooks"],
        "update-ref refs/heads/main HEAD",
    ),
    harmless("config alias.co checkout"),
    after(&["config alias.co checkout"], "co main"),
    harmless("config remote.origin.fetch +refs/heads/main:refs/heads/release"),
    after(
        &[
            ORIGIN_AHEAD,
            "config remote.origin.fetch +refs/heads/main:refs/heads/release",
        ],
        "fetch origin",
    ),
    after(
        &[
            ORIGIN_AHEAD,
            "config remote.origin.fetch +refs/heads/main:refs/heads/release",
        ],
        "fetch origin main",
    ),
    case(
        &[],
        "-c remote.origin.fetch=+refs/heads/*:refs/heads/* fetch origin",
        Expect::GitRefuses,
    ),
    refused("-c remote.origin.fetch=+refs/heads/*:refs/heads/copy/* fetch origin"),
    after(
        &[ORIGIN_AHEAD],
        "fetch origin main --refmap +refs/heads/main:refs/heads/release",
    ),
    after(
        &[ORIGIN_AHEAD],
        "fetch origin main --refmap=+refs/heads/*:refs/heads/copy/*",
    ),
    refused("pull . HEAD:master"),
    refused("pull --no-rebase . HEAD:master"),
    after(&[ORIGIN_AHEAD], "pull origin main:release"),
    harmless("remote add --mirror=fetch mirror {origin}"),
    case(
        &["remote add --mirror=fetch mirror {origin}"],
        "fetch mirror",
        Expect::GitRefuses,
    ),
    after(
        &["remote add --mirror=fetch mirror {origin}"],
        "fetch mirror refs/heads/other-feature:refs/heads/other-feature",
    ),
    refused("push origin +HEAD:main"),
    refused("push origin HEAD:refs/heads/HEAD"),
    refused("push --no-verify origin HEAD:main"),
    refused("-c core.hooksPath=/dev/null push origin HEAD:main"),
];

/// Class 2: abbreviated or attached-value options. Branch switches, copies
/// and renames are the shim's (matched generously); ref writes are the
/// hooks'; destructive ones are snapshotted.
const ABBREVIATED: &[Case] = &[
    refused("push --mirr origin"),
    refused("push --al origin"),
    after(
        &["push -q origin HEAD:refs/heads/agend/t-1/fix"],
        "push -fd origin agend/t-1/fix",
    ),
    Case {
        input: "update refs/heads/main {head}\n",
        ..refused("update-ref --stdi")
    },
    Case {
        input: "delete refs/heads/release\n",
        ..refused("update-ref --std")
    },
    refused("branch --mov agend/t-1/renamed"),
    refused("branch --cop agend/t-1/copy"),
    refused("checkout --orph orphan1"),
    refused("checkout --deta"),
    refused("checkout -bfoo"),
    refused("switch --deta"),
    refused("switch -cfoo"),
    // Round 7, finding 5: git takes any unambiguous prefix of `--detach`.
    refused("checkout --de"),
    refused("checkout --d"),
    refused("switch --de"),
    snapshot("reset --har"),
    snapshot("checkout --forc"),
    snapshot("switch --discard agend/t-1/fix"),
    snapshot("clean --forc -d"),
    harmless("config --ad remote.origin.push HEAD:refs/heads/main"),
    after(
        &["config --add remote.origin.push HEAD:refs/heads/main"],
        "push origin",
    ),
    snapshot("clean -fd -enone"),
];

/// Class 3: symbolic refs that alias a protected branch, made by someone
/// else (the shim refuses `symbolic-ref` writes, round 7: git 2.39 does not
/// report them to the hook); every write through one reaches the hook as
/// the real target.
const SYMREFS: &[Case] = &[
    after(
        &["symbolic-ref refs/heads/agend/t-1/alias refs/heads/master"],
        "update-ref refs/heads/agend/t-1/alias {head}",
    ),
    after(
        &["symbolic-ref refs/heads/agend/t-1/alias refs/heads/release"],
        "branch -f agend/t-1/alias {head}",
    ),
    after(
        &["symbolic-ref refs/heads/agend/t-1/alias refs/heads/main"],
        "push . HEAD:refs/heads/agend/t-1/alias",
    ),
    after(
        &["symbolic-ref refs/heads/agend/t-1/alias refs/heads/main"],
        "fetch . HEAD:refs/heads/agend/t-1/alias",
    ),
];

/// Class 4: a work tree other than the bound one.
const WORK_TREE: &[Case] = &[
    refused("--work-tree={repo} reset --hard"),
    refused("--work-tree={repo} checkout -- ."),
    refused("--git-dir={repo}/.git --work-tree={repo} reset --hard"),
    with_env(("GIT_WORK_TREE", "{repo}"), "checkout -- ."),
    with_env(("GIT_WORK_TREE", "{repo}"), "reset --hard"),
    with_env(("GIT_INDEX_FILE", "{repo}/.git/index"), "add wip.txt"),
];

/// Class 5: leaving the bound branch by DWIM or a side door.
const LEAVE_BRANCH: &[Case] = &[
    // DWIM from a remote-only branch: git creates `other-feature`, the hook
    // refuses that branch.
    refused("checkout other-feature"),
    refused("checkout other-feature --"),
    refused("checkout master --"),
    refused("checkout --track origin/other-feature"),
    refused("switch other-feature"),
    after(
        &[
            "stash push -q -m s -- README.md",
            "checkout -q stash@{0} -- README.md",
        ],
        "stash branch newb",
    ),
];

/// Known limits (gate page): not a plausible mistake, or git before 2.46
/// does not report the change to the hook. Checked for what still holds:
/// no protected ref moves and no work is lost (git 2.46+ may refuse).
const KNOWN_LIMITS: &[Case] = &[case(&[], "rebase main feature", Expect::Harmless)];

fn fill(s: &str, f: &Fixture, head: &str) -> String {
    s.replace("{repo}", f.repo.to_str().unwrap())
        .replace("{origin}", f.origin.to_str().unwrap())
        .replace("{head}", head)
}

/// Dirty state in both checkouts, so lost work is visible.
fn dirty(f: &Fixture) {
    std::fs::write(f.repo.join("canon-wip.txt"), "canonical wip\n").unwrap();
    std::fs::write(f.repo.join("README.md"), "canonical edit\n").unwrap();
    // An ahead commit, so a leak to main/origin is a visible move.
    std::fs::write(f.worktree.join("ahead.txt"), "ahead\n").unwrap();
    git(&f.worktree, &["add", "ahead.txt"]);
    git(&f.worktree, &["commit", "-q", "-m", "ahead"]);
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

/// The invariants; returns the violations. Known limits check only that
/// no protected ref moved and no work was lost.
fn violations(
    f: &Fixture,
    before: &str,
    expect: Expect,
    ran: &shim_common::Ran,
    limit: bool,
) -> Vec<String> {
    let mut bad = Vec::new();
    let ok = ran.output.as_ref().is_some_and(|o| o.status.success());
    match expect {
        Expect::Refused if !ran.is_refused() => bad.push("was not refused (shim or hook)".into()),
        Expect::Snapshot if !ok => bad.push("did not run".into()),
        Expect::Snapshot if snapshots(f) == 0 => bad.push("no snapshot taken".into()),
        Expect::Harmless if !ok && !limit => bad.push("did not run".into()),
        Expect::GitRefuses if ok || ran.is_refused() => {
            bad.push("expected git itself to refuse".into())
        }
        _ => {}
    }
    if f.protected_state() != before {
        bad.push(format!("protected refs moved:\n{}", f.protected_state()));
    }
    if !limit {
        let head = try_git(&f.worktree, &["symbolic-ref", "-q", "HEAD"]);
        let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
        if head != format!("refs/heads/{}", f.branch) {
            bad.push(format!("worktree HEAD is {head:?}"));
        }
        let own = format!("refs/heads/{}", f.branch);
        if try_git(&f.repo, &["symbolic-ref", "-q", &own])
            .status
            .success()
        {
            bad.push(format!("{own} became a symbolic ref"));
        }
        for b in git(
            &f.repo,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
        )
        .lines()
        {
            if !matches!(b, "main" | "master" | "release" | "feature")
                && !b.starts_with("agend/t-1/")
            {
                bad.push(format!("new branch {b}"));
            }
        }
    }
    if !f.repo.join("canon-wip.txt").exists()
        || std::fs::read_to_string(f.repo.join("README.md")).unwrap() != "canonical edit\n"
    {
        bad.push("canonical checkout lost uncommitted work".into());
    }
    let wip_intact = f.worktree.join("wip.txt").exists()
        && std::fs::read_to_string(f.worktree.join("README.md")).unwrap_or_default()
            == "worktree edit\n";
    if !wip_intact && snapshots(f) == 0 {
        bad.push("worktree lost uncommitted work without a snapshot".into());
    }
    bad
}

fn run_cases(label: &str, cases: &[Case], limit: bool) {
    let mut failures = Vec::new();
    for case in cases {
        let f = Fixture::new(label);
        dirty(&f);
        let head = f.head(&f.worktree, "HEAD");
        for s in case.setup {
            let words: Vec<String> = fill(s, &f, &head)
                .split_whitespace()
                .map(String::from)
                .collect();
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            git(&f.worktree, &words);
        }
        let before = f.protected_state();
        let cmd = fill(case.cmd, &f, &head);
        let argv: Vec<&str> = cmd.split_whitespace().collect();
        let mut ctx = f.ctx(&f.worktree);
        if let Some((var, value)) = case.env {
            let value = PathBuf::from(fill(value, &f, &head));
            match var {
                "GIT_WORK_TREE" => ctx.git_work_tree = Some(value),
                "GIT_INDEX_FILE" => ctx.git_index_file = Some(value),
                other => panic!("unsupported env {other}"),
            }
        }
        let input = fill(case.input, &f, &head);
        let ran = shim_input(&ctx, Tool::Git, &argv, input.as_bytes());
        let bad = violations(&f, &before, case.expect, &ran, limit);
        if !bad.is_empty() {
            failures.push(format!(
                "`git {}` (setup {:?}, env {:?}):\n  - {}\n  said: {}",
                case.cmd,
                case.setup,
                case.env,
                bad.join("\n  - "),
                ran.text()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases broke the guard:\n\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n\n")
    );
}

#[test]
fn corpus_config_destinations() {
    run_cases("c-config", CONFIG_DESTINATIONS, false);
}

#[test]
fn corpus_abbreviated_options() {
    run_cases("c-abbrev", ABBREVIATED, false);
}

#[test]
fn corpus_symbolic_refs() {
    run_cases("c-symref", SYMREFS, false);
}

#[test]
fn corpus_work_tree_retargeting() {
    run_cases("c-worktree", WORK_TREE, false);
}

#[test]
fn corpus_leaving_the_bound_branch() {
    run_cases("c-leave", LEAVE_BRANCH, false);
}

#[test]
fn corpus_known_limits_keep_protected_refs() {
    run_cases("c-limits", KNOWN_LIMITS, true);
}

/// Round 2, class 4: a git dir without a work tree makes the cwd the work
/// tree. With the bound worktree's own git dir and the cwd in the canonical
/// checkout, `clean` / `reset --hard` would wipe the canonical checkout.
#[test]
fn own_git_dir_with_the_cwd_elsewhere_is_refused() {
    let gitdir_of = |f: &Fixture| git(&f.worktree, &["rev-parse", "--absolute-git-dir"]);
    for (how, cmd) in [
        ("env", &["clean", "-fd"][..]),
        ("env", &["reset", "--hard"][..]),
        ("env", &["checkout", "--", "."][..]),
        ("env", &["add", "-A"][..]),
        ("argv", &["clean", "-fd"][..]),
        ("argv", &["reset", "--hard"][..]),
    ] {
        let f = Fixture::new("c-gitdir-cwd");
        dirty(&f);
        let before = f.protected_state();
        let gitdir = gitdir_of(&f);
        let ran = match how {
            "env" => {
                let ctx = Ctx {
                    git_dir: Some(PathBuf::from(&gitdir)),
                    ..f.ctx(&f.repo)
                };
                gitshim(&ctx, cmd)
            }
            _ => {
                let flag = format!("--git-dir={gitdir}");
                let argv: Vec<&str> = std::iter::once(flag.as_str())
                    .chain(cmd.iter().copied())
                    .collect();
                gitshim(&f.ctx(&f.repo), &argv)
            }
        };
        let bad = violations(&f, &before, Expect::Refused, &ran, false);
        assert!(bad.is_empty(), "{how} {cmd:?}: {bad:?}\n{}", ran.text());
        assert_eq!(ran.refused, Some("work_tree_retarget"), "{how} {cmd:?}");
    }
    // Hooks keep working: the git dir with the cwd in the bound worktree.
    let f = Fixture::new("c-gitdir-hook");
    let ctx = Ctx {
        git_dir: Some(PathBuf::from(gitdir_of(&f))),
        ..f.ctx(&f.worktree)
    };
    std::fs::write(f.worktree.join("h.txt"), "h\n").unwrap();
    gitshim(&ctx, &["add", "h.txt"]).ok();
    gitshim(&ctx, &["reset", "-q", "--hard"]).ok();
}

/// A bound branch made a symbolic ref to main (by hand, a known limit):
/// every commit through it would move main, and the hook refuses it.
#[test]
fn bound_branch_that_is_a_symref_refuses_commits() {
    let f = Fixture::new("c-own-symref");
    git(
        &f.repo,
        &[
            "symbolic-ref",
            &format!("refs/heads/{}", f.branch),
            "refs/heads/main",
        ],
    );
    let main_before = f.head(&f.repo, "main");
    let ran = gitshim(&f.ctx(&f.worktree), &["commit", "--allow-empty", "-m", "x"]);
    assert!(ran.hook_refused(), "{}", ran.text());
    assert!(ran.text().contains("refs/heads/main"), "{}", ran.text());
    assert_eq!(f.head(&f.repo, "main"), main_before);
}

// ── T5: the team repo is recognised by destination, not only by cwd ─────

#[test]
fn clone_of_the_team_remote_is_guarded() {
    let f = Fixture::new("t5-clone");
    let clone = f.workspace.join("clone");
    git(
        &f.workspace,
        &["clone", "-q", f.origin.to_str().unwrap(), "clone"],
    );
    let before = f.protected_state();
    let ctx = f.ctx(&clone);
    for cmd in [
        &["push", "origin", "HEAD:main"][..],
        &["push", "origin", "HEAD:refs/heads/release"][..],
        &["commit", "--allow-empty", "-m", "x"][..],
    ] {
        let ran = gitshim(&ctx, cmd);
        assert!(ran.refused.is_some(), "{cmd:?}: {}", ran.text());
    }
    gitshim(&ctx, &["log", "--oneline", "-1"]).ok();
    assert_eq!(f.protected_state(), before);
}

#[test]
fn clone_of_the_canonical_checkout_is_guarded() {
    let f = Fixture::new("t5-canon-clone");
    git(
        &f.workspace,
        &["clone", "-q", f.repo.to_str().unwrap(), "clone"],
    );
    let clone = f.workspace.join("clone");
    let before = f.protected_state();
    let ran = gitshim(&f.ctx(&clone), &["push", "origin", "HEAD:master"]);
    assert!(ran.refused.is_some(), "{}", ran.text());
    assert_eq!(f.protected_state(), before);
}

/// Round 4: `worktree` subcommands other than `list` are writes in the
/// team's bare remote and in team clones too. The verifier's repro, run
/// from the workspace, used to add a worktree on `main` inside `origin.git`,
/// after which a normal `git push origin main` failed ("branch is currently
/// checked out").
#[test]
fn worktree_changes_in_the_team_remote_and_clones_are_refused() {
    let f = Fixture::new("t5-worktree");
    git(
        &f.workspace,
        &["clone", "-q", f.origin.to_str().unwrap(), "clone"],
    );
    let clone = f.workspace.join("clone");
    let scratch = f.workspace.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    git(&scratch, &["init", "-q", "-b", "trunk"]);
    git(&scratch, &["commit", "-q", "--allow-empty", "-m", "s"]);
    let before = f.protected_state();
    let origin = f.origin.to_str().unwrap();
    let ws = f.ctx(&f.workspace);
    // The exact repro: `git -C <team bare>.git worktree add ../zm main`.
    let ran = gitshim(&ws, &["-C", origin, "worktree", "add", "../zm", "main"]);
    assert_eq!(ran.refused, Some("team_clone"), "{}", ran.text());
    let zm = f.root.join("zm");
    let at_origin: &[&[&str]] = &[
        &["worktree", "add", "-b", "zb", "../zm"],
        &["worktree", "remove", "../zm"],
        &["worktree", "move", "../zm", "../zn"],
        &["worktree", "prune"],
        &["worktree", "lock", "../zm"],
        &["worktree", "unlock", "../zm"],
        &["worktree", "repair"],
    ];
    for cmd in at_origin {
        let argv: Vec<&str> = ["-C", origin].iter().chain(cmd.iter()).copied().collect();
        let ran = gitshim(&ws, &argv);
        assert_eq!(ran.refused, Some("team_clone"), "{argv:?}: {}", ran.text());
        let git_dir = format!("--git-dir={origin}");
        let argv: Vec<&str> = [git_dir.as_str()]
            .iter()
            .chain(cmd.iter())
            .copied()
            .collect();
        let ran = gitshim(&ws, &argv);
        assert_eq!(ran.refused, Some("team_clone"), "{argv:?}: {}", ran.text());
    }
    let ran = gitshim(&f.ctx(&clone), &["worktree", "add", "../zc", "main"]);
    assert_eq!(ran.refused, Some("team_clone"), "{}", ran.text());
    // Reading the worktree list stays allowed everywhere.
    gitshim(&ws, &["-C", origin, "worktree", "list"]).ok();
    gitshim(&f.ctx(&clone), &["worktree", "list", "--porcelain"]).ok();
    // A truly foreign repo stays the agent's business (T5).
    gitshim(
        &f.ctx(&scratch),
        &["worktree", "add", "-q", "../scratch-wt"],
    )
    .ok();
    assert!(f.workspace.join("scratch-wt").is_dir());

    assert!(!zm.exists(), "worktree created next to origin.git");
    assert!(
        !f.workspace.join("zc").exists(),
        "worktree created from the clone"
    );
    assert!(
        !f.origin.join("worktrees").exists(),
        "origin.git has worktrees"
    );
    assert_eq!(
        try_git(&f.origin, &["rev-parse", "-q", "--verify", "refs/heads/zb"])
            .status
            .code(),
        Some(1),
        "branch created on the team remote"
    );
    assert_eq!(f.protected_state(), before);
    // The symptom: the daemon's normal push to main still works.
    git(&f.repo, &["commit", "-q", "--allow-empty", "-m", "daemon"]);
    git(&f.repo, &["push", "-q", "origin", "main"]);
}

#[test]
fn push_from_a_scratch_repo_to_the_team_url_is_refused() {
    let f = Fixture::new("t5-scratch");
    let scratch = f.workspace.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    git(&scratch, &["init", "-q", "-b", "trunk"]);
    git(&scratch, &["commit", "-q", "--allow-empty", "-m", "s"]);
    let before = f.protected_state();
    let ctx = f.ctx(&scratch);
    // Round 2: git also tries `<path>.git` (and `<path>/.git`), so the team
    // remote without its `.git` suffix is the same destination.
    let bare = f.origin.with_extension("");
    let rel = format!(
        "../../../../{}",
        bare.file_name().unwrap().to_str().unwrap()
    );
    assert!(scratch.join(&rel).with_extension("git").is_dir(), "{rel}");
    for dest in [
        f.origin.to_str().unwrap().to_string(),
        format!("file://{}", f.origin.display()),
        f.repo.to_str().unwrap().to_string(),
        bare.to_str().unwrap().to_string(),
        format!("{}/", bare.display()),
        format!("file://{}", bare.display()),
        rel,
    ] {
        let ran = gitshim(&ctx, &["push", &dest, "HEAD:refs/heads/probe"]);
        assert!(ran.refused.is_some(), "{dest}: {}", ran.text());
        let ran = gitshim(&ctx, &["push", &dest, "HEAD:refs/heads/main"]);
        assert!(ran.refused.is_some(), "{dest}: {}", ran.text());
    }
    // The scratch repo itself stays the agent's business.
    gitshim(&ctx, &["checkout", "-q", "-b", "anything"]).ok();
    // A remote saved with the suffix-less URL makes it a team clone.
    git(&scratch, &["remote", "add", "mine", bare.to_str().unwrap()]);
    let ran = gitshim(&ctx, &["push", "mine", "HEAD:refs/heads/probe"]);
    assert!(ran.refused.is_some(), "remote mine: {}", ran.text());
    assert_eq!(f.protected_state(), before);
    let probe = try_git(
        &f.origin,
        &["rev-parse", "-q", "--verify", "refs/heads/probe"],
    );
    assert!(!probe.status.success(), "probe branch reached origin");
}

#[test]
fn workspace_nested_in_another_repo_is_still_the_workspace() {
    let f = Fixture::new("t5-nested");
    // A repo that encloses AGEND_HOME (e.g. a dotfiles repo in $HOME).
    git(&f.root, &["init", "-q", "-b", "dotfiles"]);
    std::fs::write(f.worktree.join("n.txt"), "n\n").unwrap();
    let ctx = f.ctx(&f.workspace);
    gitshim(&ctx, &["add", "n.txt"]).ok();
    gitshim(&ctx, &["commit", "-q", "-m", "nested"]).ok();
    assert_eq!(git(&f.worktree, &["log", "-1", "--format=%s"]), "nested");
    let outer = try_git(&f.root, &["rev-parse", "-q", "--verify", "HEAD"]);
    assert!(
        !outer.status.success(),
        "nothing was committed to the outer repo"
    );
}

/// Round 6, findings 1–2: git 2.39 copies / renames a branch outside a ref
/// transaction, so the hook never saw the new name (`branch -C master`
/// moved master), and a hook refusal halfway through a rename deleted the
/// old branch and left `.git/logs/refs/.tmp-renamed-log`, after which every
/// `branch -c` in the repo failed. The shim now refuses every form before
/// git runs, from every place it routes from; nothing changes on disk.
#[test]
fn branch_copy_and_rename_are_refused_before_git_runs() {
    let f = Fixture::new("r6-branch-copy");
    git(&f.worktree, &["commit", "-q", "--allow-empty", "-m", "own"]);
    for b in ["agend/t-1/side", "agend/t-1/side2"] {
        git(&f.worktree, &["branch", b]);
    }
    git(&f.repo, &["branch", "userwip", "main"]);
    let tmp_log = f.repo.join(".git/logs/refs/.tmp-renamed-log");
    let state = || {
        let refs = git(
            &f.repo,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        );
        let reflog = git(&f.repo, &["reflog", "show", &f.branch]);
        format!("{refs}\n--reflog--\n{reflog}")
    };
    let before = state();
    let wt = f.worktree.to_str().unwrap();
    for cmd in [
        "branch -C master",
        "branch -C agend/t-1/side release",
        "branch -c agend/t-1/side feat/copy",
        "branch -m agend/t-1/side2 feat/renamed",
        "branch -M agend/t-1/side master",
        "branch -m agend/t-1/better-name",
        "branch -C agend/t-1/fix userwip",
        "branch -fm agend/t-1/side master",
        "branch --move agend/t-1/side agend/t-1/moved",
        "branch --cop agend/t-1/side agend/t-1/copied",
    ] {
        for (at, prefix) in [
            (&f.worktree, None),
            (&f.repo, None),
            (&f.workspace, None),
            (&f.workspace, Some(wt)),
        ] {
            let mut argv: Vec<&str> = prefix.map(|p| vec!["-C", p]).unwrap_or_default();
            argv.extend(cmd.split_whitespace());
            let ran = gitshim(&f.ctx(at), &argv);
            assert!(!tmp_log.exists(), "{argv:?} left {tmp_log:?}");
            assert_eq!(state(), before, "{argv:?} in {at:?}");
            assert_eq!(ran.refused, Some("branch_copy"), "{argv:?} in {at:?}");
            assert!(ran.text().contains("git branch agend/t-1/<name> <old>"));
        }
    }
    // The alternative the refusal gives works, and the human's own copy in
    // the canonical checkout is not blocked by a leftover.
    let ctx = f.ctx(&f.worktree);
    gitshim(&ctx, &["branch", "agend/t-1/renamed", "agend/t-1/side"]).ok();
    gitshim(&ctx, &["branch", "-D", "agend/t-1/side"]).ok();
    git(&f.repo, &["branch", "-c", "userwip", "userwip-copy"]);
    assert!(!tmp_log.exists());
}

/// Round 7, findings 1, 2 and 4 (the verifier's commands): git 2.39 runs
/// `symbolic-ref <name> <target>` and `reflog delete|expire` outside a ref
/// transaction, so the hook never saw them: `symbolic-ref refs/heads/main
/// refs/heads/agend/t-1/fix` repointed main at the agent's commit,
/// `reflog delete --updateref main@{0}` moved main, and `reflog expire
/// --expire=now --all` wiped every branch's reflog. The shim refuses them
/// before git runs, from every place it routes from; reads still run.
#[test]
fn ref_writes_outside_the_hook_are_refused_before_git_runs() {
    let f = Fixture::new("r7-raw-refs");
    git(&f.worktree, &["commit", "-q", "--allow-empty", "-m", "own"]);
    // main (checked out in the canonical checkout) and master each get a
    // second reflog entry, as in the repro.
    git(
        &f.repo,
        &["commit", "-q", "--allow-empty", "-m", "human-on-main"],
    );
    let main = f.head(&f.repo, "main");
    git(
        &f.repo,
        &["update-ref", "-m", "h2", "refs/heads/master", &main],
    );
    let state = || {
        let refs = git(
            &f.repo,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname) %(symref)",
            ],
        );
        let logs: Vec<String> = ["main", "master", "release", "feature", &f.branch]
            .iter()
            .map(|r| git(&f.repo, &["reflog", "show", "--format=%H %gs", r]))
            .collect();
        let status = git(&f.repo, &["status", "--porcelain"]);
        format!(
            "{refs}\n--reflogs--\n{}\n--status--\n{status}",
            logs.join("\n")
        )
    };
    let before = state();
    let wt = f.worktree.to_str().unwrap();
    for cmd in [
        "symbolic-ref refs/heads/main refs/heads/agend/t-1/fix",
        "symbolic-ref refs/heads/master refs/heads/agend/t-1/fix",
        "symbolic-ref refs/heads/release refs/heads/agend/t-1/fix",
        "symbolic-ref HEAD refs/heads/master",
        "reflog delete --updateref main@{0}",
        "reflog delete --updateref --rewrite master@{0}",
        "reflog expire --updateref --rewrite --expire=now master",
        "reflog expire --expire=now --all",
    ] {
        for (at, prefix) in [
            (&f.worktree, None),
            (&f.repo, None),
            (&f.workspace, None),
            (&f.workspace, Some(wt)),
        ] {
            let mut argv: Vec<&str> = prefix.map(|p| vec!["-C", p]).unwrap_or_default();
            argv.extend(cmd.split_whitespace());
            let ran = gitshim(&f.ctx(at), &argv);
            assert_eq!(ran.refused, Some("ref_outside_hook"), "{argv:?} in {at:?}");
            assert!(ran.output.is_none(), "{argv:?}: git ran");
            assert_eq!(state(), before, "{argv:?} in {at:?}");
        }
    }
    let ctx = f.ctx(&f.worktree);
    let out = gitshim(&ctx, &["symbolic-ref", "--short", "HEAD"]);
    assert_eq!(String::from_utf8_lossy(&out.ok().stdout).trim(), f.branch);
    assert!(
        !gitshim(&ctx, &["reflog", "show", "main"])
            .ok()
            .stdout
            .is_empty()
    );
}

/// Round 7, finding 3: after `git mv` fails with "destination exists", the
/// reflex `git mv -f` overwrites the destination's uncommitted edits. The
/// shim snapshots first (like `rm -f`), and the snapshot holds the edits.
#[test]
fn mv_force_over_uncommitted_edits_takes_a_snapshot() {
    let f = Fixture::new("r7-mv-force");
    std::fs::write(f.worktree.join("a.txt"), "a\n").unwrap();
    std::fs::write(f.worktree.join("b.txt"), "b\n").unwrap();
    git(&f.worktree, &["add", "a.txt", "b.txt"]);
    git(&f.worktree, &["commit", "-q", "-m", "ab"]);
    std::fs::write(f.worktree.join("b.txt"), "precious\n").unwrap();
    let ctx = f.ctx(&f.worktree);
    let plain = gitshim(&ctx, &["mv", "a.txt", "b.txt"]);
    assert!(plain.refused.is_none() && !plain.output.unwrap().status.success());
    gitshim(&ctx, &["mv", "-f", "a.txt", "b.txt"]).ok();
    assert_eq!(
        std::fs::read_to_string(f.worktree.join("b.txt")).unwrap(),
        "a\n"
    );
    let snaps = git(
        &f.repo,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/agend/snapshots/",
        ],
    );
    assert_eq!(snaps.lines().count(), 1, "{snaps}");
    assert_eq!(
        git(&f.repo, &["show", &format!("{snaps}:b.txt")]),
        "precious"
    );
}

// ── T9: kill forms, against a fake recorder only ─────────────────────────

/// A directory holding fake `kill`/`pkill`/`killall` that only record their
/// argv; the shim's "real tool" resolves here and never to /bin/kill.
fn recorder(dir: &Path) -> (PathBuf, PathBuf) {
    let bin = dir.join("fakebin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.join("kill.log");
    for tool in ["kill", "pkill", "killall"] {
        let p = bin.join(tool);
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\nprintf '%s %s\\n' {tool} \"$*\" >> '{}'\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }
    (bin, log)
}

#[test]
fn kill_forms_that_reach_holders_or_groups_are_refused() {
    let f = Fixture::new("t9");
    let (bin, log) = recorder(&f.root);
    let fake_holder = f.root.join("holder").join("agend");
    std::fs::create_dir_all(fake_holder.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("/bin/sleep", &fake_holder).unwrap();
    let mut holder = Command::new(&fake_holder).arg("60").spawn().unwrap();
    let mut other = Command::new("/bin/sleep").arg("60").spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let ctx = Ctx {
        path: bin.into_os_string(),
        ..f.ctx(&f.workspace)
    };
    let h = holder.id().to_string();
    let o = other.id().to_string();
    let spaced = format!(" {h}");
    let trailing = format!("{h} ");
    let tabbed = format!("\t{h}\n");
    let plus = format!("+{h}");
    let zero = format!("0{h}");
    let group = format!("-{h}");
    let cases: Vec<Vec<&str>> = vec![
        vec![&h],
        vec![&spaced],
        vec![&trailing],
        vec![&tabbed],
        vec![&plus],
        vec![&zero],
        vec!["-9", &spaced],
        vec!["-s", "KILL", &h],
        vec!["-KILL", &h],
        vec!["-n", "9", &h],
        vec!["--", &h],
        vec![&o, &h],
        vec!["0"],
        vec!["-9", "0"],
        vec!["-1"],
        vec!["-9", "-1"],
        vec!["--", "-1"],
        vec!["-s", "9", "-1"],
        vec!["-9", "--", &group],
        vec!["agend"],
        vec!["%1"],
        vec!["-9", "-a", &h],
    ];
    for argv in &cases {
        let ran = shim(&ctx, Tool::Kill, argv);
        assert!(ran.refused.is_some(), "kill {argv:?} was not refused");
    }
    for (tool, argv) in [
        (Tool::Pkill, vec!["agend"]),
        (Tool::Pkill, vec!["-9", "-f", "agend"]),
        (Tool::Killall, vec!["agend"]),
    ] {
        assert!(shim(&ctx, tool, &argv).refused.is_some(), "{argv:?}");
    }
    assert_eq!(
        std::fs::read_to_string(&log).unwrap_or_default(),
        "",
        "the fake kill was never reached"
    );
    // An explicit pid of a normal process goes to the (fake) real kill.
    shim(&ctx, Tool::Kill, &[&o]).ok();
    shim(&ctx, Tool::Kill, &["-l"]).ok();
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        format!("kill {o}\nkill -l\n")
    );
    assert!(holder.try_wait().unwrap().is_none(), "holder still alive");
    holder.kill().unwrap();
    holder.wait().unwrap();
    other.kill().unwrap();
    other.wait().unwrap();
}
