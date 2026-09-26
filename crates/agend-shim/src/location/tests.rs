//! Unit tests for `location`.

use super::*;

struct Fixed {
    wt: Option<PathBuf>,
    src: Option<PathBuf>,
    wt_dirs: Option<(PathBuf, PathBuf)>,
    team: Option<PathBuf>,
}

impl Anchors for Fixed {
    fn worktree(&self) -> Option<&Path> {
        self.wt.as_deref()
    }
    fn source_repo(&self) -> Option<&Path> {
        self.src.as_deref()
    }
    fn worktree_dirs(&self) -> Option<(PathBuf, PathBuf)> {
        self.wt_dirs.clone()
    }
    fn team_common_dir(&self) -> Option<PathBuf> {
        self.team.clone()
    }
}

fn r(g: &str, c: &str, w: Option<&str>) -> Resolved {
    Resolved {
        git_dir: g.into(),
        common_dir: c.into(),
        work_tree: w.map(PathBuf::from),
        prefix: PathBuf::new(),
    }
}

#[test]
fn classifies_git_answers_against_the_binding() {
    let a = Fixed {
        wt: Some("/h/wt".into()),
        src: Some("/repo".into()),
        wt_dirs: Some(("/repo/.git/worktrees/t".into(), "/repo/.git".into())),
        team: Some("/repo/.git".into()),
    };
    let own = r("/repo/.git/worktrees/t", "/repo/.git", Some("/h/wt"));
    assert_eq!(locate(&own, false, &a), Location::Worktree);
    assert_eq!(locate(&own, true, &a), Location::Worktree);
    // The worktree's git dir with another work tree is still the bound
    // worktree's repo; `classify` refuses writes whose work tree differs.
    let elsewhere = r("/repo/.git/worktrees/t", "/repo/.git", Some("/h/ws"));
    assert_eq!(locate(&elsewhere, true, &a), Location::Worktree);
    // Canonical git dir with the bound work tree: not the worktree.
    let canon_wt = r("/repo/.git", "/repo/.git", Some("/h/wt"));
    assert_eq!(locate(&canon_wt, true, &a), Location::Canonical);
    let canon = r("/repo/.git", "/repo/.git", Some("/repo"));
    assert_eq!(locate(&canon, false, &a), Location::Canonical);
    let bare_canon = r("/repo/.git", "/repo/.git", None);
    assert_eq!(locate(&bare_canon, true, &a), Location::Canonical);
    let sibling = r("/repo/.git/worktrees/u", "/repo/.git", Some("/h/u"));
    assert_eq!(locate(&sibling, false, &a), Location::OtherWorktree);
    // GIT_COMMON_DIR relabelling the worktree's refs is not the worktree:
    // a foreign repo with its work tree there (destructive calls snapshot
    // the worktree, with the relabelling env removed).
    let relabelled = r("/repo/.git/worktrees/t", "/x/origin.git", Some("/h/wt"));
    assert_eq!(locate(&relabelled, true, &a), Location::Nested);
    let scratch = r("/s/.git", "/s/.git", Some("/s"));
    assert_eq!(locate(&scratch, false, &a), Location::Foreign);
    // Round 9: a repo inside the bound worktree (a submodule, whose git dir
    // is under the worktree's, or a nested clone) is the agent's own.
    let sub = r(
        "/repo/.git/worktrees/t/modules/m",
        "/repo/.git/worktrees/t/modules/m",
        Some("/h/wt/m"),
    );
    assert_eq!(locate(&sub, false, &a), Location::Nested);
    let clone = r(
        "/h/wt/vendor/lib/.git",
        "/h/wt/vendor/lib/.git",
        Some("/h/wt/vendor/lib"),
    );
    assert_eq!(locate(&clone, false, &a), Location::Nested);
    // The team's own git dir with a work tree in the worktree is not.
    let team_there = r("/repo/.git", "/repo/.git", Some("/h/wt/m"));
    assert_eq!(locate(&team_there, true, &a), Location::Canonical);
    // A submodule of the canonical checkout or of a sibling's worktree keeps
    // its git dir in the team's: not a scratch repo.
    let canon_sub = r(
        "/repo/.git/modules/m",
        "/repo/.git/modules/m",
        Some("/repo/m"),
    );
    assert_eq!(locate(&canon_sub, false, &a), Location::OtherWorktree);
    let sibling_sub = r(
        "/repo/.git/worktrees/u/modules/m",
        "/repo/.git/worktrees/u/modules/m",
        Some("/h/u/m"),
    );
    assert_eq!(locate(&sibling_sub, false, &a), Location::OtherWorktree);
}

#[test]
fn unbound_agents_still_know_the_canonical_checkout() {
    let a = Fixed {
        wt: None,
        src: Some("/repo".into()),
        wt_dirs: None,
        team: Some("/repo/.git".into()),
    };
    let canon = r("/repo/.git", "/repo/.git", Some("/repo"));
    assert_eq!(locate(&canon, false, &a), Location::Canonical);
    // Found from the canonical checkout itself: git is not asked again.
    let none = Fixed { team: None, ..a };
    assert_eq!(locate(&canon, false, &none), Location::Canonical);
    assert_eq!(locate(&canon, true, &none), Location::Foreign);
}

/// Hermetic: every path is made in a fresh temp dir (round 4: `x` used
/// to be `$TMPDIR/x`, which existed only on one machine).
#[test]
fn parses_rev_parse_output() {
    let dir = agend_testkit::tempdir::TempDir::new("rev-parse").unwrap();
    let tmp = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::create_dir(tmp.join("x")).unwrap();
    let t = tmp.clone();
    let full = Resolved::from_lines(&[t.clone(), t.clone(), t.clone()], &t).unwrap();
    assert_eq!(full.work_tree.as_deref(), Some(tmp.as_path()));
    assert_eq!(full.prefix, PathBuf::new());
    let sub = Resolved::from_lines(&[t.clone(), t.clone(), t.clone(), "x/".into()], &t).unwrap();
    assert_eq!(sub.prefix, PathBuf::from("x/"));
    let top = Resolved::from_lines(&[t.clone(), t.clone(), t.clone(), "".into()], &t).unwrap();
    assert_eq!(top.prefix, PathBuf::new());
    let bare = Resolved::from_lines(&[t.clone(), t.clone()], &t).unwrap();
    assert_eq!(bare.work_tree, None);
    assert_eq!(bare.root(), tmp.as_path());
    // `--git-common-dir` may be relative to where git ran.
    let rel = Resolved::from_lines(&[t.clone(), "..".into()], &t.join("x")).unwrap();
    assert_eq!(rel.common_dir, tmp);
    assert_eq!(Resolved::from_lines(&[], &t), None);
    assert_eq!(Resolved::from_lines(std::slice::from_ref(&t), &t), None);
    assert_eq!(
        Resolved::from_lines(&[t.clone(), t.join("no-such-dir")], &t),
        None
    );
}
