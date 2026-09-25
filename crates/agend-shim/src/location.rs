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
    /// A foreign repo inside the bound worktree (a submodule, a nested
    /// clone): writes run there, destructive ones snapshotted in that repo.
    Nested,
    /// Not inside any repo and not `Workspace` (`/tmp/x`, `~`, `-C typo`).
    NoRepo,
    /// Not inside any repo, in the agent's own workspace
    /// (`<AGEND_HOME>/workspace/<instance>` or below), without `-C`,
    /// `--git-dir`, `--work-tree` or `GIT_*`: stands for the bound worktree.
    Workspace,
    /// Not resolved: no usable snapshot, or a call that does not need it.
    /// Treated as the team's repo.
    Unknown,
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

/// Classifies git's answer (no answer, no repo: the caller's `NoRepo` / `Workspace`). `explicit` is whether the caller chose the git
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
        _ if anchors.worktree().is_some_and(|w| r.root().starts_with(w)) => Location::Nested,
        // A submodule of another checkout keeps its git dir in the team's.
        Some(team) if r.git_dir.starts_with(&team) => Location::OtherWorktree,
        _ => Location::Foreign,
    }
}

#[cfg(test)]
mod tests;
