//! Where a git invocation would act, found from the filesystem alone (no git
//! process): the bound worktree, the canonical checkout, another checkout of
//! the same repo, a foreign repo, or no repo at all.
//!
//! Must NOT: spawn git; this runs on every call.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// The agent's bound worktree.
    Worktree,
    /// The canonical checkout of the team's repo (`source_repo`).
    Canonical,
    /// Another worktree of the same repo (a sibling's, or a stale one).
    OtherWorktree,
    /// A repo unrelated to the team's repo; the shim does not guard it.
    Foreign,
    /// Not inside any repo (e.g. the agent's workspace directory).
    NoRepo,
    /// Cannot tell (no usable snapshot); treated as the team's repo.
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

/// What the location check needs to know about the agent.
pub struct Anchors<'a> {
    pub source_repo: Option<&'a Path>,
    pub worktree: Option<&'a Path>,
    /// False when the snapshot is unusable: every repo is then `Unknown`.
    pub snapshot_ok: bool,
}

/// Resolves the location of a git call made in `dir` (the caller's cwd with
/// any `-C` applied), with `git_dir` from `--git-dir`/`GIT_DIR` if given.
pub fn locate(dir: &Path, git_dir: Option<&Path>, anchors: &Anchors) -> Location {
    let gitdir = match git_dir {
        Some(g) => Some(dir.join(g)),
        None => discover_gitdir(dir),
    };
    let Some(gitdir) = gitdir.and_then(|g| std::fs::canonicalize(g).ok()) else {
        return Location::NoRepo;
    };
    if !anchors.snapshot_ok {
        return Location::Unknown;
    }
    if let Some(wt) = anchors.worktree
        && gitdir_of_checkout(wt).as_deref() == Some(gitdir.as_path())
    {
        return Location::Worktree;
    }
    let Some(source_common) = anchors
        .source_repo
        .and_then(gitdir_of_checkout)
        .map(|g| common_dir(&g))
    else {
        return Location::Foreign;
    };
    if common_dir(&gitdir) != source_common {
        Location::Foreign
    } else if gitdir == source_common {
        Location::Canonical
    } else {
        Location::OtherWorktree
    }
}

/// Walks up from `dir` to the first `.git` (directory or `gitdir:` file).
fn discover_gitdir(dir: &Path) -> Option<PathBuf> {
    let start = std::fs::canonicalize(dir).ok()?;
    start
        .ancestors()
        .find_map(|d| resolve_dot_git(&d.join(".git")))
}

/// Canonical git dir of the checkout rooted at `root`.
pub fn gitdir_of_checkout(root: &Path) -> Option<PathBuf> {
    resolve_dot_git(&root.join(".git")).and_then(|g| std::fs::canonicalize(g).ok())
}

/// A `.git` entry: a directory is the git dir; a file holds `gitdir: <path>`.
fn resolve_dot_git(dot_git: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(dot_git).ok()?;
    if meta.is_dir() {
        return Some(dot_git.to_path_buf());
    }
    let text = std::fs::read_to_string(dot_git).ok()?;
    let target = text.trim().strip_prefix("gitdir:")?.trim();
    Some(dot_git.parent()?.join(target))
}

/// The shared git dir (`commondir` file of a linked worktree), canonical.
fn common_dir(gitdir: &Path) -> PathBuf {
    std::fs::read_to_string(gitdir.join("commondir"))
        .ok()
        .and_then(|c| std::fs::canonicalize(gitdir.join(c.trim())).ok())
        .unwrap_or_else(|| gitdir.to_path_buf())
}
