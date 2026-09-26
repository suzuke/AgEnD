//! Verifier round 4, items the verifier could not reach:
//! - the everyday-work pass-through list, command by command, through the
//!   shim against real temporary repos;
//! - paths that reach the repos through a symlink, and paths with spaces,
//!   for both protected and normal commands, whichever spelling the binding
//!   snapshot, `AGEND_HOME` and the cwd use.
//!
//! Real temporary repos only (`git init` / `git clone`, via `shim_common`);
//! every call names absolute paths and its own cwd.

#![cfg(unix)]

mod shim_common;

use agend_shim::binding::snapshot_path;
use agend_shim::ctx::Ctx;
use agend_testkit::tempdir::TempDir;
use shim_common::{Fixture, INSTANCE, Ran, git, gitshim, try_git};
use std::path::{Path, PathBuf};

enum Step<'a> {
    /// Append a line to a file in the worktree.
    Edit(&'a str),
    Ok(&'a str, Vec<&'a str>),
    Refused(&'a str, Vec<&'a str>),
}

/// Where a step runs, relative to the fixture.
fn dir_of(f: &Fixture, at: &str) -> PathBuf {
    match at {
        "wt" => f.worktree.clone(),
        "sub" => f.worktree.join("sub"),
        "ws" => f.workspace.clone(),
        "repo" => f.repo.clone(),
        other => panic!("unknown dir {other}"),
    }
}

fn describe(ran: &Ran) -> String {
    ran.text()
}

fn succeeded(ran: &Ran) -> bool {
    ran.refused.is_none() && ran.output.as_ref().is_some_and(|o| o.status.success())
}

/// Everyday work passes; the few deliberate refusals give the exact next
/// step. Hooks run.
#[test]
fn everyday_work_passes_through() {
    let f = Fixture::new("everyday");
    std::fs::create_dir_all(f.worktree.join("sub")).unwrap();
    std::fs::write(f.worktree.join("sub").join("s.txt"), "s\n").unwrap();
    git(&f.worktree, &["add", "sub/s.txt"]);
    git(&f.worktree, &["commit", "-q", "-m", "sub"]);
    // origin/main one commit ahead, for the rebase and the pull.
    git(
        &f.repo,
        &["commit", "-q", "--allow-empty", "-m", "upstream"],
    );
    git(&f.repo, &["push", "-q", "origin", "main"]);
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
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    }
    let own = format!("HEAD:refs/heads/{}", f.branch);
    let own_to_own = format!("{0}:{0}", f.branch);
    let typo = f.root.join("typo.git");
    let typo = typo.to_str().unwrap();
    let steps: Vec<Step> = vec![
        Step::Ok("wt", vec!["status"]),
        Step::Ok("wt", vec!["log", "--oneline", "-3"]),
        Step::Edit("README.md"),
        Step::Ok("wt", vec!["diff"]),
        Step::Ok("wt", vec!["add", "README.md"]),
        Step::Ok("wt", vec!["commit", "-q", "-m", "c1"]),
        Step::Ok("wt", vec!["commit", "-q", "--amend", "--no-edit"]),
        Step::Ok(
            "wt",
            vec!["-c", "core.editor=true", "commit", "-q", "--amend"],
        ),
        Step::Ok("wt", vec!["fetch", "-q"]),
        Step::Ok("wt", vec!["fetch", "-q", "origin"]),
        Step::Ok("wt", vec!["rebase", "-q", "origin/main"]),
        Step::Ok("wt", vec!["push", "-q", "origin", &own]),
        Step::Ok("wt", vec!["push", "-q", "-u", "origin", &own]),
        Step::Ok(
            "wt",
            vec!["push", "-q", "--force-with-lease", "origin", &own],
        ),
        Step::Ok("wt", vec!["push", "-q", "origin", &own_to_own]),
        // T7 (owner decision 2026-09-25): a push git resolves to the bound
        // branch runs; the pre-push hook sees where git really pushes.
        Step::Ok("wt", vec!["push", "-q", "-u", "origin", &f.branch]),
        Step::Ok("wt", vec!["push", "-q"]),
        Step::Ok("wt", vec!["push", "-q", "origin", "HEAD"]),
        // Refused by the pre-push hook: git reports refs/heads/main.
        Step::Refused("wt", vec!["push", "origin", "HEAD:main"]),
        Step::Ok("wt", vec!["pull", "-q", "--rebase"]),
        Step::Ok("wt", vec!["pull", "-q", "--rebase", "origin", "main"]),
        Step::Ok("wt", vec!["branch", "agend/t-1/x"]),
        Step::Ok("wt", vec!["switch", "-q", &f.branch]),
        Step::Ok("wt", vec!["checkout", "-q", &f.branch]),
        // Round 5: `-` resolves to the bound branch here, so it runs.
        Step::Ok("wt", vec!["checkout", "-q", "-"]),
        Step::Ok("wt", vec!["switch", "-q", "-"]),
        Step::Ok(
            "wt",
            vec![
                "-c",
                "sequence.editor=true",
                "rebase",
                "-q",
                "-i",
                "origin/main",
            ],
        ),
        Step::Refused("wt", vec!["switch", "agend/t-1/x"]),
        Step::Edit("README.md"),
        // Owner decision 2026-09-25: refs/stash is shared with the canonical
        // checkout, so agents save work as a wip commit instead.
        Step::Refused("wt", vec!["stash", "push", "-q", "-m", "wip"]),
        Step::Refused("wt", vec!["stash", "pop", "-q"]),
        Step::Ok("wt", vec!["stash", "list"]),
        Step::Ok("wt", vec!["commit", "-q", "-am", "wip: c2"]),
        Step::Ok("wt", vec!["worktree", "list"]),
        // Round 11: remotes live in the config the canonical checkout shares.
        Step::Ok("wt", vec!["remote", "-v"]),
        Step::Ok("wt", vec!["remote", "get-url", "origin"]),
        Step::Ok("wt", vec!["remote", "show", "origin"]),
        Step::Ok("wt", vec!["remote", "update"]),
        Step::Refused("wt", vec!["remote", "set-url", "origin", typo]),
        Step::Refused("wt", vec!["remote", "remove", "origin"]),
        Step::Refused("repo", vec!["remote", "rename", "origin", "up"]),
        Step::Refused("ws", vec!["remote", "add", "mine", typo]),
        Step::Refused("wt", vec!["remote", "update", "--prune"]),
        Step::Ok("wt", vec!["-C", "sub", "status"]),
        Step::Edit("sub/s.txt"),
        Step::Ok("wt", vec!["-C", "sub", "add", "s.txt"]),
        Step::Ok("wt", vec!["-C", "sub", "commit", "-q", "-m", "c3"]),
        Step::Ok("sub", vec!["status"]),
        Step::Edit("sub/s.txt"),
        Step::Ok("sub", vec!["commit", "-q", "-am", "c4"]),
        Step::Ok("ws", vec!["status"]),
        Step::Edit("README.md"),
        Step::Ok("ws", vec!["commit", "-q", "-am", "c5"]),
        Step::Ok("repo", vec!["status"]),
        Step::Ok("ws", vec!["push", "-q", "origin", &own]),
    ];
    let mut failures = Vec::new();
    for step in &steps {
        let (at, cmd, want_ok) = match step {
            Step::Edit(p) => {
                let path = f.worktree.join(p);
                let mut s = std::fs::read_to_string(&path).unwrap();
                s.push_str("more\n");
                std::fs::write(path, s).unwrap();
                continue;
            }
            Step::Ok(at, cmd) => (at, cmd, true),
            Step::Refused(at, cmd) => (at, cmd, false),
        };
        let ran = gitshim(&f.ctx(&dir_of(&f, at)), cmd);
        let good = if want_ok {
            succeeded(&ran)
        } else {
            ran.is_refused() && ran.text().contains("next step: ")
        };
        if !good {
            failures.push(format!("({at}) git {}: {}", cmd.join(" "), describe(&ran)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert_eq!(git(&f.worktree, &["branch", "--show-current"]), f.branch);
    assert_eq!(
        git(&f.worktree, &["rev-parse", "HEAD"]),
        git(&f.origin, &["rev-parse", &f.branch]),
        "own branch pushed"
    );
    let remotes = git(&f.repo, &["remote", "-v"]);
    assert_eq!(remotes.lines().count(), 2, "{remotes}");
    let url = git(&f.repo, &["remote", "get-url", "origin"]);
    assert_eq!(Path::new(&url), f.origin, "canonical remote unchanged");
    git(
        &f.repo,
        &["rev-parse", "--verify", "-q", "refs/remotes/origin/main"],
    );
    let log = std::fs::read_to_string(&marker).unwrap();
    assert!(log.matches("pre-commit").count() >= 5, "{log}");
    assert!(log.contains("post-commit"), "{log}");
}

/// A spelling of the fixture root that reaches it through a symlink.
fn linked(f: &Fixture, links: &TempDir) -> PathBuf {
    let dir = std::fs::canonicalize(links.path()).unwrap();
    let alt = dir.join("via link");
    std::os::unix::fs::symlink(&f.root, &alt).unwrap();
    alt
}

fn respell(p: &Path, from: &Path, to: &Path) -> PathBuf {
    to.join(p.strip_prefix(from).unwrap())
}

/// Symlinked paths and paths with spaces: the snapshot, `AGEND_HOME` and
/// the cwd each use either spelling; protected actions are refused, normal
/// work passes, destructive ones snapshot first.
#[test]
fn symlinked_and_spaced_paths_resolve_alike() {
    let mut failures = Vec::new();
    for combo in 0..8u8 {
        let (snap_alt, home_alt, cwd_alt) = (combo & 1 != 0, combo & 2 != 0, combo & 4 != 0);
        let f = Fixture::new("sym with space");
        assert!(f.root.to_str().unwrap().contains(' '));
        let links = TempDir::new("links").unwrap();
        let alt = linked(&f, &links);
        let spell = |p: &Path, use_alt: bool| {
            if use_alt {
                respell(p, &f.root, &alt)
            } else {
                p.to_path_buf()
            }
        };
        let mut snap = f.snapshot(true);
        snap.source_repo = Some(spell(&f.repo, snap_alt));
        if let Some(agend_shim::binding::Binding::Work { worktree, .. }) = snap.binding.as_mut() {
            *worktree = spell(&f.worktree, snap_alt);
        }
        std::fs::write(
            snapshot_path(&f.home, INSTANCE),
            serde_json::to_string(&snap).unwrap(),
        )
        .unwrap();
        let ctx = |dir: &Path| Ctx {
            home: Some(spell(&f.home, home_alt)),
            ..f.ctx(&spell(dir, cwd_alt))
        };
        let wt = f.worktree.clone();
        let sub = wt.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let origin = spell(&f.origin, cwd_alt).display().to_string();
        let wt_s = spell(&wt, cwd_alt).display().to_string();
        let own = format!("HEAD:refs/heads/{}", f.branch);
        let label = format!("snapshot alt={snap_alt} home alt={home_alt} cwd alt={cwd_alt}");
        // One commit ahead, so a push to main would really move it.
        git(&wt, &["commit", "-q", "--allow-empty", "-m", "ahead"]);
        let before = protected(&f);

        let mut check = |ran: Ran, want_ok: bool, what: &str| {
            let good = if want_ok {
                succeeded(&ran)
            } else {
                ran.is_refused()
            };
            if !good {
                failures.push(format!("{label}: {what}: {}", describe(&ran)));
            }
        };
        for dir in [&wt, &f.workspace, &f.repo, &sub] {
            let c = ctx(dir);
            let at = dir.display();
            check(gitshim(&c, &["status"]), true, &format!("status in {at}"));
            for cmd in [
                &["push", "origin", "HEAD:main"][..],
                &["push", &origin, "HEAD:refs/heads/main"],
                &["checkout", "-b", "x"],
                &["branch", "-D", "release"],
                &["update-ref", "refs/heads/main", "HEAD"],
                &["worktree", "add", "../zz", "main"],
                &["-C", &origin, "worktree", "add", "../zm", "main"],
                &["-C", &origin, "branch", "-f", "main", "HEAD"],
                &["checkout", "master"],
            ] {
                check(gitshim(&c, cmd), false, &format!("{cmd:?} in {at}"));
            }
        }
        let c = ctx(&f.workspace);
        check(gitshim(&c, &["-C", &wt_s, "status"]), true, "-C worktree");
        std::fs::write(wt.join("n.txt"), "n\n").unwrap();
        check(gitshim(&ctx(&wt), &["add", "n.txt"]), true, "add");
        check(
            gitshim(&ctx(&f.workspace), &["commit", "-q", "-m", "n1"]),
            true,
            "commit from the workspace",
        );
        std::fs::write(sub.join("s.txt"), "s\n").unwrap();
        check(gitshim(&ctx(&sub), &["add", "s.txt"]), true, "add in sub");
        check(
            gitshim(&ctx(&sub), &["commit", "-q", "-m", "n2"]),
            true,
            "commit in sub",
        );
        check(
            gitshim(&ctx(&wt), &["push", "-q", "origin", &own]),
            true,
            "push own branch",
        );
        let snaps_before = count_snapshots(&f);
        std::fs::write(wt.join("n.txt"), "dirty\n").unwrap();
        check(
            gitshim(&ctx(&wt), &["reset", "-q", "--hard"]),
            true,
            "reset --hard",
        );
        if count_snapshots(&f) != snaps_before + 1 {
            failures.push(format!("{label}: reset --hard took no snapshot"));
        }
        if protected(&f) != before {
            failures.push(format!("{label}: a protected ref moved"));
        }
        if f.root.join("zm").exists() || f.origin.join("worktrees").exists() {
            failures.push(format!("{label}: a worktree was added to origin.git"));
        }
        if git(&f.worktree, &["log", "-1", "--format=%s"]) != "n2" {
            failures.push(format!("{label}: commits did not land in the worktree"));
        }
        if !git(&f.repo, &["status", "--porcelain"]).is_empty() {
            failures.push(format!("{label}: canonical checkout changed"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

fn count_snapshots(f: &Fixture) -> usize {
    let out = try_git(&f.repo, &["for-each-ref", "refs/agend/snapshots"]);
    String::from_utf8_lossy(&out.stdout).lines().count()
}

/// main / master / release, locally and on origin (the agent's own branch
/// on origin may move).
fn protected(f: &Fixture) -> String {
    let refs = ["refs/heads/main", "refs/heads/master", "refs/heads/release"];
    [&f.repo, &f.origin]
        .iter()
        .map(|dir| {
            let mut args = vec!["for-each-ref", "--format=%(refname) %(objectname)"];
            args.extend(refs);
            git(dir, &args)
        })
        .collect::<Vec<_>>()
        .join("\n--origin--\n")
}
