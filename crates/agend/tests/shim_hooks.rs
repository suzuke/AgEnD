//! The agend git hooks themselves (`agend_shim::install_hooks`), against
//! real temporary repos only (`shim_common`; hooks are installed only in
//! the fixture's own worktrees):
//! - only the agent worktree gets them: the canonical checkout has no
//!   `core.hooksPath`, and a commit on main there still works;
//! - the project's own hooks keep running from the agent worktree
//!   (chaining, with arguments, stdin and exit code);
//! - install refuses the main worktree, is idempotent, and uninstall
//!   removes them (after which the shim refuses writes);
//! - without the agent's binding the hook fails closed.

#![cfg(unix)]

mod shim_common;

use shim_common::{Fixture, git, git_base, gitshim, install_hooks, try_git};
use std::path::Path;

/// Real git with the fixture's isolation, hooks NOT skipped (unlike
/// `git`/`try_git`): what a user or tool running git there would get.
fn plain_git(dir: &Path, args: &[&str]) -> std::process::Output {
    git_base()
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=U", "-c", "user.email=u@example.com"])
        .args(args)
        .output()
        .unwrap()
}

fn write_hook(dir: &Path, name: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

#[test]
fn canonical_checkout_has_no_agend_hook() {
    let f = Fixture::new("hooks-canonical");
    let hooks = agend_shim::hooks_dir(&f.home);
    // The agent worktree: its own config.worktree names the agend hooks.
    let out = plain_git(&f.worktree, &["config", "--show-origin", "core.hooksPath"]);
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(shown.contains("config.worktree"), "{shown}");
    assert!(shown.contains(hooks.to_str().unwrap()), "{shown}");
    // The canonical checkout: nothing.
    let out = plain_git(&f.repo, &["config", "core.hooksPath"]);
    assert!(!out.status.success(), "canonical has core.hooksPath");
    assert!(out.stdout.is_empty());
    // A human commit on main there succeeds (no agend hook refuses it).
    let before = f.head(&f.repo, "main");
    let out = plain_git(&f.repo, &["commit", "-q", "--allow-empty", "-m", "human"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(f.head(&f.repo, "main"), before, "main moved");
    // The shared config gained only the capability switch.
    let shared = std::fs::read_to_string(f.repo.join(".git/config")).unwrap();
    assert!(!shared.contains("hooksPath"), "{shared}");
    assert!(!shared.contains("packRefs"), "{shared}");
    assert!(shared.contains("worktreeConfig = true"), "{shared}");
    // Hook entries are symlinks to the agend binary.
    let link = std::fs::read_link(hooks.join("reference-transaction")).unwrap();
    assert_eq!(link, Path::new(shim_common::AGEND));
}

#[test]
fn project_hooks_still_run_from_the_agent_worktree() {
    let f = Fixture::new("hooks-chain");
    let log = f.root.join("project-hooks.log");
    let hooks = f.repo.join(".git/hooks");
    let l = log.display();
    write_hook(&hooks, "pre-commit", &format!("echo pre-commit >> '{l}'"));
    write_hook(
        &hooks,
        "commit-msg",
        &format!("echo \"commit-msg $(cat \"$1\")\" >> '{l}'"),
    );
    write_hook(
        &hooks,
        "reference-transaction",
        &format!("echo \"rt $1\" >> '{l}'; sed 's/^/  /' >> '{l}'"),
    );
    write_hook(
        &hooks,
        "pre-push",
        &format!("echo \"pre-push $1\" >> '{l}'; sed 's/^/  /' >> '{l}'"),
    );
    let ctx = f.ctx(&f.worktree);
    gitshim(&ctx, &["commit", "-q", "--allow-empty", "-m", "chained"]).ok();
    let own = format!("HEAD:refs/heads/{}", f.branch);
    gitshim(&ctx, &["push", "-q", "origin", &own]).ok();
    let text = std::fs::read_to_string(&log).unwrap();
    for want in [
        "pre-commit\n",
        "commit-msg chained\n",
        "rt prepared\n",
        "rt committed\n",
        &format!("refs/heads/{}\n", f.branch),
        "pre-push origin\n",
        &format!("refs/heads/{} ", f.branch),
    ] {
        assert!(text.contains(want), "{want:?} missing from:\n{text}");
    }
    // A failing project hook still fails the command.
    write_hook(&hooks, "pre-commit", "echo 'project says no' >&2; exit 3");
    let ran = gitshim(&ctx, &["commit", "-q", "--allow-empty", "-m", "no"]);
    assert!(!ran.output.as_ref().unwrap().status.success());
    assert!(ran.text().contains("project says no"), "{}", ran.text());
    assert!(!ran.hook_refused(), "not an agend refusal");
}

/// A `core.hooksPath` the project set before the agent's worktree was
/// bound (e.g. `.githooks`) is where the agend hooks chain to.
#[test]
fn a_previous_hooks_path_is_chained() {
    let f = Fixture::new("hooks-previous");
    agend_shim::uninstall_hooks(&git_base, &f.worktree).unwrap();
    git(&f.repo, &["config", "core.hooksPath", ".githooks"]);
    let log = f.root.join("githooks.log");
    write_hook(
        &f.worktree.join(".githooks"),
        "pre-commit",
        &format!("echo from-githooks >> '{}'", log.display()),
    );
    install_hooks(&f.home, &f.worktree);
    // Re-installing must not make the agend hooks chain to themselves.
    install_hooks(&f.home, &f.worktree);
    gitshim(
        &f.ctx(&f.worktree),
        &["commit", "-q", "--allow-empty", "-m", "x"],
    )
    .ok();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "from-githooks\n");
}

#[test]
fn install_refuses_the_canonical_checkout_and_uninstall_removes_the_hooks() {
    let f = Fixture::new("hooks-install");
    let hooks = agend_shim::hooks_dir(&f.home);
    let err = agend_shim::install_hooks(&git_base, &hooks, Path::new(shim_common::AGEND), &f.repo)
        .unwrap_err();
    assert!(err.contains("not a linked agent worktree"), "{err}");
    assert!(
        !plain_git(&f.repo, &["config", "core.hooksPath"])
            .status
            .success()
    );

    agend_shim::uninstall_hooks(&git_base, &f.worktree).unwrap();
    agend_shim::uninstall_hooks(&git_base, &f.worktree).unwrap();
    assert!(
        !plain_git(&f.worktree, &["config", "core.hooksPath"])
            .status
            .success()
    );
    // Without the hooks the shim refuses writes (nothing would guard refs).
    let ran = gitshim(&f.ctx(&f.worktree), &["commit", "--allow-empty", "-m", "x"]);
    assert_eq!(ran.refused, Some("hooks_missing"), "{}", ran.text());
    gitshim(&f.ctx(&f.worktree), &["status"]).ok();
}

/// git run in the agent worktree without the agent's binding (the env
/// stripped, or a tool with its own env): the hook refuses branch and
/// protected-ref writes, and lets remote-tracking refs through.
#[test]
fn without_the_binding_the_hook_fails_closed() {
    let f = Fixture::new("hooks-closed");
    let before = f.protected_state();
    for args in [
        &["update-ref", "refs/heads/main", "HEAD"][..],
        &["commit", "-q", "--allow-empty", "-m", "x"][..],
        &["branch", "-D", "release"][..],
    ] {
        let out = plain_git(&f.worktree, args);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?} ran");
        assert!(err.contains("cannot read your binding"), "{args:?}: {err}");
    }
    assert_eq!(f.protected_state(), before);
    let out = plain_git(&f.worktree, &["fetch", "-q", "origin"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// git 2.39's `pack-refs` reports every ref (main too) to the hook as if it
/// were written, so the agent worktree does not pack refs (`gc.packRefs`
/// false in its config.worktree): `git gc` there works; the canonical side
/// packs.
#[test]
fn gc_runs_in_the_agent_worktree() {
    let f = Fixture::new("hooks-gc");
    let before = f.protected_state();
    let ctx = f.ctx(&f.worktree);
    for i in 0..3 {
        gitshim(
            &ctx,
            &["commit", "-q", "--allow-empty", "-m", &format!("c{i}")],
        )
        .ok();
    }
    gitshim(&ctx, &["gc", "-q"]).ok();
    assert_eq!(f.protected_state(), before);
    // The canonical checkout still packs refs itself.
    let out = plain_git(&f.repo, &["pack-refs", "--all"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `rebase --update-refs` moves every branch pointing into the rebased
/// commits; the hook refuses the ones that are not the agent's.
#[test]
fn rebase_update_refs_cannot_move_other_branches() {
    let f = Fixture::new("hooks-update-refs");
    git(
        &f.worktree,
        &["commit", "-q", "--allow-empty", "-m", "mine"],
    );
    git(&f.worktree, &["branch", "-f", "feature", "HEAD"]);
    git(
        &f.repo,
        &["commit", "-q", "--allow-empty", "-m", "upstream"],
    );
    git(&f.repo, &["push", "-q", "origin", "main"]);
    git(&f.worktree, &["fetch", "-q", "origin"]);
    let feature = f.head(&f.repo, "feature");
    let ran = gitshim(
        &f.ctx(&f.worktree),
        &["rebase", "-q", "--update-refs", "origin/main"],
    );
    assert!(ran.hook_refused(), "{}", ran.text());
    assert!(ran.text().contains("feature"), "{}", ran.text());
    assert_eq!(f.head(&f.repo, "feature"), feature, "feature did not move");
    let _ = try_git(&f.worktree, &["rebase", "--abort"]);
}
