//! The verifier's bypass corpus (PR #107 round 1), replayed against real
//! temporary repos. Each case runs optional setup with the real git (state
//! that could exist before the shim is asked), then one command through the
//! shim, then checks the invariants the guard exists for:
//! - main / master / release did not move, locally or on origin;
//! - the bound worktree is still on `agend/t-1/fix`, which is a real branch;
//! - no branch outside `agend/t-1/` appeared;
//! - the canonical checkout's uncommitted work is intact;
//! - the worktree's uncommitted work is intact, or a snapshot holds it.
//!
//! Kill forms run against a fake `kill` recorder only (never a real kill),
//! with targets that are this test's own children.

#![cfg(unix)]

mod common;

use agend_shim::Tool;
use agend_shim::ctx::Ctx;
use common::{Fixture, git, gitshim, shim, try_git};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// The shim refuses.
    Refused,
    /// The shim runs it, after taking a snapshot.
    Snapshot,
}

struct Case {
    /// Real-git setup commands, run in the worktree before the shim call.
    setup: &'static [&'static str],
    /// The command given to the shim (whitespace split; placeholders
    /// `{repo}`, `{origin}`, `{head}`).
    cmd: &'static str,
    /// `GIT_WORK_TREE` / `GIT_INDEX_FILE` for the shim call, if any.
    env: Option<(&'static str, &'static str)>,
    expect: Expect,
}

const fn refused(cmd: &'static str) -> Case {
    Case {
        setup: &[],
        cmd,
        env: None,
        expect: Expect::Refused,
    }
}

const fn after(setup: &'static [&'static str], cmd: &'static str) -> Case {
    Case {
        setup,
        cmd,
        env: None,
        expect: Expect::Refused,
    }
}

const fn with_env(env: (&'static str, &'static str), cmd: &'static str) -> Case {
    Case {
        setup: &[],
        cmd,
        env: Some(env),
        expect: Expect::Refused,
    }
}

/// Class 1: destinations that come from config, `-c`, `--refmap` or `pull`.
const CONFIG_DESTINATIONS: &[Case] = &[
    refused("-c remote.origin.push=HEAD:refs/heads/main push origin"),
    refused("config remote.origin.push +HEAD:refs/heads/master"),
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
    refused("-c push.default=matching push origin"),
    refused("config branch.agend/t-1/fix.merge refs/heads/main"),
    refused("config push.default upstream"),
    refused("config core.hooksPath hooks"),
    refused("config alias.co checkout"),
    refused("config remote.origin.fetch +refs/heads/main:refs/heads/release"),
    after(
        &["config remote.origin.fetch +refs/heads/main:refs/heads/release"],
        "fetch origin",
    ),
    after(
        &["config remote.origin.fetch +refs/heads/main:refs/heads/release"],
        "fetch origin main",
    ),
    refused("-c remote.origin.fetch=+refs/heads/*:refs/heads/* fetch origin"),
    refused("fetch origin master --refmap +refs/heads/master:refs/heads/release"),
    refused("fetch origin main --refmap=+refs/heads/*:refs/heads/*"),
    refused("pull . HEAD:master"),
    refused("pull --no-rebase . HEAD:master"),
    refused("pull origin main:release"),
    refused("remote add --mirror=fetch mirror {origin}"),
    refused("push origin +HEAD:main"),
    refused("push origin HEAD:HEAD"),
];

/// Class 2: abbreviated or attached-value options.
const ABBREVIATED: &[Case] = &[
    refused("push --mirr origin"),
    refused("push --al origin"),
    refused("push -fd origin agend/t-1/fix"),
    refused("update-ref --stdi"),
    refused("update-ref --std"),
    refused("branch --mov agend/t-1/renamed"),
    refused("branch --cop agend/t-1/copy"),
    refused("checkout --orph orphan1"),
    refused("checkout --deta"),
    refused("checkout -bfoo"),
    refused("switch --deta"),
    refused("switch -cfoo"),
    refused("reset --har"),
    refused("checkout --forc"),
    refused("switch --discard agend/t-1/fix"),
    refused("clean --forc -d"),
    refused("config --ad remote.origin.push HEAD:refs/heads/main"),
    Case {
        setup: &[],
        cmd: "clean -fd -enone",
        env: None,
        expect: Expect::Snapshot,
    },
];

/// Class 3: symbolic refs that alias a protected branch.
const SYMREFS: &[Case] = &[
    refused("symbolic-ref refs/heads/agend/t-1/fix refs/heads/main"),
    refused("symbolic-ref refs/heads/agend/t-1/alias refs/heads/master"),
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
    refused("checkout other-feature"),
    refused("checkout other-feature --"),
    refused("checkout master --"),
    refused("checkout --track origin/other-feature"),
    refused("switch other-feature"),
    refused("rebase main feature"),
    refused("rebase --update-refs main"),
    refused("stash branch newb"),
];

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

/// The invariants; returns the violations.
fn violations(f: &Fixture, before: &str, expect: Expect, ran: &common::Ran) -> Vec<String> {
    let mut bad = Vec::new();
    match (expect, ran.refused) {
        (Expect::Refused, None) => bad.push("was not refused".to_string()),
        (Expect::Snapshot, Some(code)) => bad.push(format!("refused ({code}), expected to run")),
        (Expect::Snapshot, None) if snapshots(f) == 0 => bad.push("no snapshot taken".into()),
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
        if !matches!(b, "main" | "master" | "release" | "feature") && !b.starts_with("agend/t-1/") {
            bad.push(format!("new branch {b}"));
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

fn run_corpus(label: &str, cases: &[Case]) {
    let mut failures = Vec::new();
    for case in cases {
        let f = Fixture::new(label);
        dirty(&f);
        let head = f.head(&f.worktree, "HEAD");
        for s in case.setup {
            let words: Vec<String> = s.split_whitespace().map(String::from).collect();
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
        let ran = gitshim(&ctx, &argv);
        let bad = violations(&f, &before, case.expect, &ran);
        if !bad.is_empty() {
            failures.push(format!(
                "`git {}` (setup {:?}, env {:?}):\n  - {}\n  shim said: {}",
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
    run_corpus("c-config", CONFIG_DESTINATIONS);
}

#[test]
fn corpus_abbreviated_options() {
    run_corpus("c-abbrev", ABBREVIATED);
}

#[test]
fn corpus_symbolic_refs() {
    run_corpus("c-symref", SYMREFS);
}

#[test]
fn corpus_work_tree_retargeting() {
    run_corpus("c-worktree", WORK_TREE);
}

#[test]
fn corpus_leaving_the_bound_branch() {
    run_corpus("c-leave", LEAVE_BRANCH);
}

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
    assert!(ran.refused.is_some(), "{}", ran.text());
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

#[test]
fn push_from_a_scratch_repo_to_the_team_url_is_refused() {
    let f = Fixture::new("t5-scratch");
    let scratch = f.workspace.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    git(&scratch, &["init", "-q", "-b", "trunk"]);
    git(&scratch, &["commit", "-q", "--allow-empty", "-m", "s"]);
    let before = f.protected_state();
    let ctx = f.ctx(&scratch);
    for dest in [
        f.origin.to_str().unwrap().to_string(),
        format!("file://{}", f.origin.display()),
        f.repo.to_str().unwrap().to_string(),
    ] {
        let ran = gitshim(&ctx, &["push", &dest, "HEAD:refs/heads/main"]);
        assert!(ran.refused.is_some(), "{dest}: {}", ran.text());
    }
    assert_eq!(f.protected_state(), before);
    // The scratch repo itself stays the agent's business.
    gitshim(&ctx, &["checkout", "-q", "-b", "anything"]).ok();
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
