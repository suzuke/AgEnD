//! Verifier round 5, finding 1: routing into the bound worktree keeps the
//! caller's directory inside its checkout, so a relative pathspec (`.`)
//! typed in `<canonical>/src/sub` means `<worktree>/src/sub`, not the whole
//! worktree. A call from another agent's worktree, or from a directory the
//! bound worktree lacks, is refused instead of routed.
//!
//! The verifier's repros E (`git rm -rf .`) and B (`git clean -fdx .`) are
//! replayed as typed, plus `checkout -- .`, `add .` and `restore .`; each
//! from a canonical subdirectory and from another worktree's subdirectory.
//!
//! Real temporary repos only (`common`); every call names absolute paths
//! and its own cwd.

#![cfg(unix)]

mod common;

use common::{Fixture, git, gitshim, try_git};
use std::path::{Path, PathBuf};

/// The fixture plus `src/sub` in main, the bound worktree and a second
/// agent's worktree (`t-2`), with the verifier's dirty state in the bound
/// worktree: uncommitted edits at the root and in `src/sub`, ignored
/// `.env` and `target/` at the root, untracked files in both places.
struct Lab {
    f: Fixture,
    wt2: PathBuf,
}

fn lab(label: &str) -> Lab {
    let f = Fixture::new(label);
    std::fs::create_dir_all(f.repo.join("src/sub")).unwrap();
    std::fs::write(f.repo.join("src/sub/f.txt"), "s\n").unwrap();
    std::fs::write(f.repo.join(".gitignore"), "target/\n.env\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-q", "-m", "sub"]);
    git(&f.worktree, &["merge", "-q", "--ff-only", "main"]);
    let wt2 = f.home.join("worktrees").join("t-2");
    git(
        &f.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "agend/t-2/x",
            wt2.to_str().unwrap(),
            "main",
        ],
    );
    let wt = &f.worktree;
    std::fs::write(wt.join("README.md"), "hello\nunsaved-root-edit\n").unwrap();
    std::fs::write(wt.join("src/sub/f.txt"), "s\nunsaved-sub-edit\n").unwrap();
    std::fs::write(wt.join(".env"), "secret\n").unwrap();
    std::fs::create_dir_all(wt.join("target")).unwrap();
    std::fs::write(wt.join("target/build.bin"), "big\n").unwrap();
    std::fs::write(wt.join("NOTES.md"), "notes\n").unwrap();
    std::fs::create_dir_all(wt.join("src/sub/target")).unwrap();
    std::fs::write(wt.join("src/sub/target/o.bin"), "o\n").unwrap();
    std::fs::write(wt.join("src/sub/tmp.txt"), "scratch\n").unwrap();
    Lab { f, wt2 }
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

/// Everything at the bound worktree's root is exactly as `lab` left it.
fn assert_root_untouched(l: &Lab, what: &str) {
    let wt = &l.f.worktree;
    assert_eq!(
        read(&wt.join("README.md")).as_deref(),
        Some("hello\nunsaved-root-edit\n"),
        "{what}: root README.md"
    );
    for p in [".env", "target/build.bin", "NOTES.md", ".gitignore"] {
        assert!(wt.join(p).exists(), "{what}: root {p} is gone");
    }
    let staged = git(wt, &["diff", "--cached", "--name-only"]);
    assert!(
        !staged.lines().any(|l| !l.starts_with("src/sub/")),
        "{what}: staged outside src/sub: {staged}"
    );
}

/// The canonical checkout and the second worktree are as they were.
fn assert_others_untouched(l: &Lab, what: &str) {
    assert_eq!(git(&l.f.repo, &["status", "--porcelain"]), "", "{what}");
    assert_eq!(git(&l.wt2, &["status", "--porcelain"]), "", "{what}");
    assert_eq!(
        read(&l.f.repo.join("src/sub/f.txt")).as_deref(),
        Some("s\n"),
        "{what}"
    );
}

fn snapshots(f: &Fixture) -> Vec<String> {
    git(
        &f.repo,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/agend/snapshots/",
        ],
    )
    .lines()
    .map(String::from)
    .collect()
}

/// E: `git rm -rf .` from `<canonical>/src/sub` removes `src/sub` of the
/// bound worktree only, after a snapshot that keeps the uncommitted edit.
#[test]
fn repro_e_rm_rf_from_a_canonical_subdir_stays_in_that_subdir() {
    let l = lab("scope-e");
    let ran = gitshim(
        &l.f.ctx(&l.f.repo.join("src/sub")),
        &["rm", "-rq", "-f", "."],
    );
    ran.ok();
    assert!(
        ran.text().contains(&format!(
            "running in your bound worktree {}",
            l.f.worktree.join("src/sub").display()
        )),
        "{}",
        ran.text()
    );
    assert!(!l.f.worktree.join("src/sub/f.txt").exists(), "rm ran");
    assert_root_untouched(&l, "E");
    assert_others_untouched(&l, "E");
    let snaps = snapshots(&l.f);
    assert_eq!(snaps.len(), 1, "rm -f is snapshotted: {snaps:?}");
    let saved = git(&l.f.repo, &["show", &format!("{}:src/sub/f.txt", snaps[0])]);
    assert!(saved.contains("unsaved-sub-edit"), "{saved}");
}

/// B: `git clean -fdx .` from `<canonical>/src/sub` cleans `src/sub` of
/// the bound worktree only; the root's ignored `.env` and `target/` stay.
#[test]
fn repro_b_clean_fdx_from_a_canonical_subdir_stays_in_that_subdir() {
    let l = lab("scope-b");
    gitshim(&l.f.ctx(&l.f.repo.join("src/sub")), &["clean", "-fdx", "."]).ok();
    let wt = &l.f.worktree;
    assert!(!wt.join("src/sub/tmp.txt").exists(), "clean ran");
    assert!(!wt.join("src/sub/target").exists(), "clean -x ran");
    assert_root_untouched(&l, "B");
    assert_others_untouched(&l, "B");
}

/// C, `restore .`, D: the other path-scoped writes from a canonical subdir.
#[test]
fn checkout_restore_and_add_from_a_canonical_subdir_stay_in_that_subdir() {
    for cmd in [&["checkout", "--", "."][..], &["restore", "."][..]] {
        let l = lab("scope-c");
        gitshim(&l.f.ctx(&l.f.repo.join("src/sub")), cmd).ok();
        assert_eq!(
            read(&l.f.worktree.join("src/sub/f.txt")).as_deref(),
            Some("s\n"),
            "{cmd:?} reverted src/sub"
        );
        assert_root_untouched(&l, &format!("{cmd:?}"));
        assert_others_untouched(&l, &format!("{cmd:?}"));
        assert_eq!(snapshots(&l.f).len(), 1, "{cmd:?} is snapshotted");
    }
    let l = lab("scope-d");
    gitshim(&l.f.ctx(&l.f.repo.join("src/sub")), &["add", "."]).ok();
    let staged = git(&l.f.worktree, &["diff", "--cached", "--name-only"]);
    assert_eq!(staged, "src/sub/f.txt\nsrc/sub/tmp.txt", "D: {staged}");
    assert_root_untouched(&l, "D");
    assert_others_untouched(&l, "D");
}

/// `-C <canonical>/src/sub` from the workspace is the same caller directory.
#[test]
fn chdir_into_a_canonical_subdir_keeps_the_subdir_too() {
    let l = lab("scope-c-flag");
    let sub = l.f.repo.join("src/sub");
    gitshim(
        &l.f.ctx(&l.f.workspace),
        &["-C", sub.to_str().unwrap(), "clean", "-fdx", "."],
    )
    .ok();
    assert!(!l.f.worktree.join("src/sub/tmp.txt").exists(), "clean ran");
    assert_root_untouched(&l, "-C");
    assert_others_untouched(&l, "-C");
}

/// F and the rest from another agent's worktree: refused, nothing changes,
/// and the message names the directory to use.
#[test]
fn writes_from_another_worktree_are_refused_not_routed() {
    let l = lab("scope-f");
    let there = l.wt2.join("src/sub");
    for cmd in [
        &["rm", "-rq", "-f", "."][..],
        &["clean", "-fdx", "."][..],
        &["checkout", "--", "."][..],
        &["add", "."][..],
        &["restore", "."][..],
        &["commit", "-q", "--allow-empty", "-m", "x"][..],
    ] {
        let ran = gitshim(&l.f.ctx(&there), cmd);
        assert_eq!(
            ran.refused,
            Some("other_worktree"),
            "{cmd:?}: {}",
            ran.text()
        );
        assert!(
            ran.text().contains(&format!(
                "next step: cd {} and run it there",
                l.f.worktree.join("src/sub").display()
            )),
            "{}",
            ran.text()
        );
        assert!(l.f.worktree.join("src/sub/tmp.txt").exists(), "{cmd:?}");
        assert!(l.f.worktree.join("src/sub/f.txt").exists(), "{cmd:?}");
        assert_root_untouched(&l, &format!("{cmd:?}"));
        assert_others_untouched(&l, &format!("{cmd:?}"));
    }
    assert!(snapshots(&l.f).is_empty());
    // A read there shows that worktree, as typed.
    let ran = gitshim(&l.f.ctx(&there), &["branch", "--show-current"]);
    assert_eq!(
        String::from_utf8_lossy(&ran.ok().stdout).trim(),
        "agend/t-2/x"
    );
}

/// A canonical directory the bound worktree does not have: refused, named.
#[test]
fn a_directory_missing_from_the_worktree_is_refused() {
    let l = lab("scope-missing");
    let only = l.f.repo.join("only-canon");
    std::fs::create_dir_all(&only).unwrap();
    for cmd in [&["clean", "-fdx", "."][..], &["status"][..]] {
        let ran = gitshim(&l.f.ctx(&only), cmd);
        assert_eq!(ran.refused, Some("route_dir_missing"), "{cmd:?}");
        let text = ran.text();
        assert!(text.contains("(only-canon/)"), "{text}");
        assert!(
            text.contains(&format!("next step: cd {}", l.f.worktree.display())),
            "{text}"
        );
    }
    assert_root_untouched(&l, "missing");
    assert!(l.f.worktree.join("src/sub/tmp.txt").exists());
    // The agent's own subdirectory still works as typed.
    let own = gitshim(
        &l.f.ctx(&l.f.worktree.join("src/sub")),
        &["clean", "-fdx", "."],
    );
    own.ok();
    assert!(!l.f.worktree.join("src/sub/tmp.txt").exists());
    assert_root_untouched(&l, "own subdir");
    let out = try_git(&l.f.worktree, &["status", "--porcelain"]);
    assert!(out.status.success());
}
