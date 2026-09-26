//! Submodules and nested repos against real temporary repos (verifier round
//! 9): a snapshot keeps a submodule or a nested repo only as a gitlink, so
//! what it cannot bring back is refused before git runs, and a submodule of
//! the bound worktree gets a snapshot of its own. Each test fails on the
//! round-9 code, where every call below ran (rc 0) and the work was lost.

#![cfg(unix)]

mod shim_common;

use shim_common::{Fixture, git, gitshim};
use std::path::{Path, PathBuf};

/// A committed submodule `mod` (`s.txt` = "s") in `dir`, cloned from a
/// fresh repo under the fixture root.
fn add_submodule(f: &Fixture, dir: &Path) -> PathBuf {
    let src = f.root.join("subrepo");
    if !src.exists() {
        std::fs::create_dir_all(&src).unwrap();
        git(&src, &["init", "-q", "-b", "main"]);
        std::fs::write(src.join("s.txt"), "s\n").unwrap();
        git(&src, &["add", "s.txt"]);
        git(&src, &["commit", "-q", "-m", "s"]);
    }
    let url = src.to_str().unwrap();
    let add = ["-c", "protocol.file.allow=always", "submodule", "add", "-q"];
    git(dir, &[&add[..], &[url, "mod"][..]].concat());
    git(dir, &["commit", "-q", "-m", "add mod"]);
    dir.join("mod")
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| format!("<{e}>"))
}

/// Runs the printed undo line (`git a && git b`) with `cwd`.
fn undo(f: &Fixture, cwd: &Path, text: &str) {
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix("agend-shim: to undo: "))
        .expect("undo line");
    for part in line.split(" && ") {
        let words: Vec<&str> = part.split_whitespace().skip(1).collect();
        gitshim(&f.ctx(cwd), &words).ok();
    }
}

/// Case A: recursing into a submodule discards its uncommitted work, which
/// the worktree snapshot has only as a gitlink.
#[test]
fn recursing_into_a_dirty_submodule_is_refused() {
    let f = Fixture::new("sub-recurse");
    let m = add_submodule(&f, &f.worktree);
    let ctx = f.ctx(&f.worktree);
    std::fs::write(m.join("s.txt"), "SUBWORK\n").unwrap();
    let flagged: [&[&str]; 2] = [
        &["reset", "--hard", "--recurse-submodules"],
        &["checkout", "--recurse-submodules", "."],
    ];
    for cmd in flagged {
        let ran = gitshim(&ctx, cmd);
        assert_eq!(ran.refused, Some("submodule_recurse"), "{cmd:?}");
    }
    git(&f.repo, &["config", "submodule.recurse", "true"]);
    let configured: [&[&str]; 3] = [&["reset", "--hard"], &["checkout", "."], &["restore", "."]];
    for cmd in configured {
        let ran = gitshim(&ctx, cmd);
        assert_eq!(ran.refused, Some("submodule_recurse"), "{cmd:?}");
        assert!(
            ran.text().contains("--no-recurse-submodules"),
            "{}",
            ran.text()
        );
    }
    assert_eq!(read(&m.join("s.txt")), "SUBWORK\n");
    // The way out the refusal names: the submodule is left alone.
    gitshim(&ctx, &["reset", "--hard", "--no-recurse-submodules"]).ok();
    assert_eq!(read(&m.join("s.txt")), "SUBWORK\n");
}

/// Case B: inside a submodule of the bound worktree (`-C mod`, `cd mod`)
/// the shim used to see a foreign repo and take no snapshot.
#[test]
fn destructive_commands_in_a_submodule_are_snapshotted_there() {
    let f = Fixture::new("sub-inside");
    let m = add_submodule(&f, &f.worktree);
    std::fs::write(m.join("s.txt"), "SUBWORK\n").unwrap();
    std::fs::write(m.join("new.txt"), "untracked\n").unwrap();
    let ran = gitshim(&f.ctx(&f.worktree), &["-C", "mod", "reset", "--hard"]);
    ran.ok();
    assert!(ran.text().contains("snapshot "), "{}", ran.text());
    assert_eq!(read(&m.join("s.txt")), "s\n");
    // The snapshot is in the submodule's repo; its undo line runs there.
    undo(&f, &m, &ran.text());
    assert_eq!(read(&m.join("s.txt")), "SUBWORK\n");
    assert_eq!(read(&m.join("new.txt")), "untracked\n");

    let ran = gitshim(&f.ctx(&m), &["checkout", "."]);
    ran.ok();
    assert!(ran.text().contains("snapshot "), "{}", ran.text());
    assert_eq!(read(&m.join("s.txt")), "s\n");
    undo(&f, &m, &ran.text());
    assert_eq!(read(&m.join("s.txt")), "SUBWORK\n");

    // git runs the foreach command with its own git, past the shim.
    let ran = gitshim(
        &f.ctx(&f.worktree),
        &["submodule", "foreach", "git reset --hard"],
    );
    assert_eq!(ran.refused, Some("submodule_foreach"), "{}", ran.text());
    assert_eq!(read(&m.join("s.txt")), "SUBWORK\n");
    gitshim(&f.ctx(&f.worktree), &["submodule", "status"]).ok();
}

/// A submodule of the canonical checkout is the human's work, not a
/// scratch repo: writes there are refused like another worktree's.
#[test]
fn a_submodule_of_the_canonical_checkout_is_not_the_agents() {
    let f = Fixture::new("sub-canon");
    let m = add_submodule(&f, &f.repo);
    std::fs::write(m.join("s.txt"), "HUMAN\n").unwrap();
    let ran = gitshim(&f.ctx(&m), &["reset", "--hard"]);
    assert_eq!(ran.refused, Some("other_worktree"), "{}", ran.text());
    assert_eq!(read(&m.join("s.txt")), "HUMAN\n");
    gitshim(&f.ctx(&m), &["status", "--short"]).ok();
}

/// Round 9, finding 2: `clean -ff` deletes an untracked nested repo with its
/// local commits; the snapshot has it only as a gitlink.
#[test]
fn clean_forced_twice_keeps_nested_repos() {
    let f = Fixture::new("clean-ff");
    let lib = f.worktree.join("vendor").join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    git(&lib, &["init", "-q", "-b", "main"]);
    std::fs::write(lib.join("l.txt"), "local\n").unwrap();
    git(&lib, &["add", "l.txt"]);
    git(&lib, &["commit", "-q", "-m", "local only"]);
    let local = git(&lib, &["rev-parse", "HEAD"]);
    let ctx = f.ctx(&f.worktree);
    let forms: [&[&str]; 4] = [
        &["clean", "-ffd"],
        &["clean", "-fdf"],
        &["clean", "-f", "-f", "-d"],
        &["clean", "--force", "--force", "-d"],
    ];
    for cmd in forms {
        let ran = gitshim(&ctx, cmd);
        assert_eq!(ran.refused, Some("clean_unsnapshotted"), "{cmd:?}");
        assert!(ran.text().contains("git clean -fd"), "{}", ran.text());
    }
    // `clean -fd` (snapshotted) skips nested repos.
    std::fs::write(f.worktree.join("junk.txt"), "junk\n").unwrap();
    gitshim(&ctx, &["clean", "-fd"]).ok();
    assert!(!f.worktree.join("junk.txt").exists());
    assert_eq!(git(&lib, &["rev-parse", "HEAD"]), local);
}

/// Round 10: in canonical the submodule is not initialised (`mod/` is an
/// empty directory), in the bound worktree it is. Routing from
/// `<canonical>/mod` kept the prefix and ran git in `<worktree>/mod`, which
/// is the submodule, another repo, after a worktree snapshot that has it
/// only as a gitlink: its work was lost. Routing into a nested repo is
/// refused; the verifier's `-C mod` from canonical, with no `mod/` there
/// at all, is refused as git would fail (`shim_route_scope.rs`).
#[test]
fn routing_from_canonical_never_lands_in_a_submodule() {
    let f = Fixture::new("sub-route");
    add_submodule(&f, &f.repo);
    git(&f.worktree, &["merge", "-q", "--ff-only", "main"]);
    let init = ["-c", "protocol.file.allow=always", "submodule", "update"];
    git(&f.worktree, &[&init[..], &["-q", "--init"][..]].concat());
    git(&f.repo, &["submodule", "deinit", "-q", "-f", "mod"]);
    assert!(f.repo.join("mod").is_dir(), "empty mod/ in canonical");
    let m = f.worktree.join("mod");
    std::fs::write(m.join("s.txt"), "SUBWORK\n").unwrap();
    for (cwd, cmd) in [
        (f.repo.clone(), &["-C", "mod", "checkout", "."][..]),
        (f.repo.clone(), &["-C", "mod", "clean", "-fd"][..]),
        (f.repo.join("mod"), &["reset", "--hard"][..]),
        (f.repo.join("mod"), &["restore", "."][..]),
        (f.repo.join("mod"), &["status"][..]),
    ] {
        let ran = gitshim(&f.ctx(&cwd), cmd);
        assert_eq!(
            ran.refused,
            Some("route_dir_missing"),
            "{cmd:?}: {}",
            ran.text()
        );
        assert!(
            ran.text().contains("submodule or nested repo"),
            "{}",
            ran.text()
        );
        assert_eq!(read(&m.join("s.txt")), "SUBWORK\n", "{cmd:?}");
    }
}
