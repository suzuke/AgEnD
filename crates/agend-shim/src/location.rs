//! Where a git invocation acts, decided from git's own answer. `git::resolve`
//! asks the real git (`rev-parse` with the caller's global options, cwd and
//! `GIT_*` environment); this module classifies that answer against the
//! binding: the bound worktree, the canonical checkout, another worktree of
//! the team repo, a foreign repo, or no repo at all.
//!
//! Because git resolves the location, every spelling git accepts (a `.git`
//! gitfile, the real git dir, `-C`, `GIT_DIR`, `GIT_COMMON_DIR`, relative
//! paths, subdirectories) lands on the same answer.
//!
//! Must NOT: look for repos on its own; git is the only source of truth.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// The agent's bound worktree: git acts on its git dir.
    Worktree,
    /// The canonical checkout of the team's repo (`source_repo`).
    Canonical,
    /// Another worktree of the same repo (a sibling's, or a stale one).
    OtherWorktree,
    /// A repo other than the team's checkout; the shim guards only what can
    /// reach the team repo from it (see `team`).
    Foreign,
    /// Not inside any repo (e.g. the agent's workspace directory).
    NoRepo,
    /// Not resolved: no usable snapshot, or a call that does not need it.
    /// Treated as the team's repo.
    Unknown,
}

impl Location {
    /// Whether the location belongs to the team's repo (or might).
    pub fn is_ours(self) -> bool {
        matches!(
            self,
            Location::Worktree | Location::Canonical | Location::OtherWorktree | Location::Unknown
        )
    }
}

/// What git answered for the call, with canonical paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    /// The work tree git would use; `None` without one (a bare repo).
    pub work_tree: Option<PathBuf>,
    /// Where the call runs inside `work_tree` (git's `--show-prefix`:
    /// relative, empty at the top or outside the work tree). Routing keeps
    /// it, so a relative pathspec names the same directory in the bound
    /// worktree.
    pub prefix: PathBuf,
}

impl Resolved {
    /// From the output lines of `rev-parse --absolute-git-dir
    /// --git-common-dir --show-toplevel --show-prefix` run in `base` (the
    /// directory git runs in, after `-C`), which relative answers are
    /// relative to. Without a work tree git prints the first two and then
    /// fails on `--show-toplevel`; with no repo it prints nothing. The
    /// prefix line is empty at the top of the work tree (so `lines` must
    /// keep empty lines); a missing prefix line means an empty prefix.
    pub fn from_lines(lines: &[PathBuf], base: &Path) -> Option<Resolved> {
        let canon = |p: &PathBuf| std::fs::canonicalize(base.join(p)).ok();
        let (g, c, w, prefix) = match lines {
            [g, c] => (g, c, None, PathBuf::new()),
            [g, c, w] => (g, c, Some(w), PathBuf::new()),
            [g, c, w, p] => (g, c, Some(w), p.clone()),
            _ => return None,
        };
        Some(Resolved {
            git_dir: canon(g)?,
            common_dir: canon(c)?,
            work_tree: match w {
                Some(w) => Some(canon(w)?),
                None => None,
            },
            prefix,
        })
    }

    /// The directory git works from: the work tree, else the git dir.
    pub fn root(&self) -> &Path {
        self.work_tree.as_deref().unwrap_or(&self.git_dir)
    }
}

/// What the classification compares against, asked of git lazily.
pub trait Anchors {
    /// The bound worktree (canonical path), if bound and present.
    fn worktree(&self) -> Option<&Path>;
    /// The canonical checkout (`source_repo`, canonical path).
    fn source_repo(&self) -> Option<&Path>;
    /// Git dir and common dir of the bound worktree.
    fn worktree_dirs(&self) -> Option<(PathBuf, PathBuf)>;
    /// Common dir of the canonical checkout (`source_repo`).
    fn team_common_dir(&self) -> Option<PathBuf>;
}

/// Classifies git's answer (no answer, no repo: the caller's `NoRepo`). `explicit` is whether the caller chose the git
/// dir, work tree or common dir (`--git-dir`, `--work-tree`, `--bare`,
/// `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`); without that, git found the
/// repo by walking up to the work tree, so a work tree equal to the bound
/// worktree (or the canonical checkout) means its own git dir, and git is not
/// asked about the anchor.
pub fn locate(r: &Resolved, explicit: bool, anchors: &dyn Anchors) -> Location {
    if let Some(wt) = anchors.worktree() {
        let own = if explicit {
            anchors
                .worktree_dirs()
                .is_some_and(|(g, c)| g == r.git_dir && c == r.common_dir)
        } else {
            r.work_tree.as_deref() == Some(wt)
        };
        if own {
            return Location::Worktree;
        }
    }
    if !explicit && r.work_tree.is_some() && r.work_tree.as_deref() == anchors.source_repo() {
        return if r.git_dir == r.common_dir {
            Location::Canonical
        } else {
            Location::OtherWorktree
        };
    }
    match anchors.team_common_dir() {
        Some(team) if team == r.common_dir && r.git_dir == team => Location::Canonical,
        Some(team) if team == r.common_dir => Location::OtherWorktree,
        _ => Location::Foreign,
    }
}

#[cfg(test)]
mod tests {
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
        // GIT_COMMON_DIR relabelling the worktree's refs is not the worktree.
        let relabelled = r("/repo/.git/worktrees/t", "/x/origin.git", Some("/h/wt"));
        assert_eq!(locate(&relabelled, true, &a), Location::Foreign);
        let scratch = r("/s/.git", "/s/.git", Some("/s"));
        assert_eq!(locate(&scratch, false, &a), Location::Foreign);
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
        let sub =
            Resolved::from_lines(&[t.clone(), t.clone(), t.clone(), "x/".into()], &t).unwrap();
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
}
