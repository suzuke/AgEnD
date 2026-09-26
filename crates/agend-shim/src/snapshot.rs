//! Takes a snapshot before destructive git operations (the scope of v1
//! agentic-git: `reset --hard|--merge|--keep`, `clean`, `checkout`,
//! `restore`, `switch -f|--discard-changes`, `rm -f`, `mv -f`, merge /
//! rebase / pull / cherry-pick / revert / am; see `classify`) so they can be
//! undone.
//!
//! A snapshot is a commit of the whole working tree (tracked changes plus
//! untracked, non-ignored files; built in a temporary index, so the real
//! index is untouched) whose parent is the previous HEAD, kept at
//! `refs/agend/snapshots/<instance>/<id>`. Ignored files are not captured
//! (so `classify` refuses `clean -x|-X`), nor is `refs/stash`. The shim never prunes snapshots.
//!
//! Must NOT: run for read-only commands.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Ref namespace for snapshots.
pub const REF_PREFIX: &str = "refs/agend/snapshots/";

/// A saved snapshot and how to get back to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub id: String,
    pub reference: String,
    /// HEAD before the operation (`None` in a repo without commits).
    pub previous_head: Option<String>,
}

impl Saved {
    /// Lines printed to the agent before the destructive operation runs.
    pub fn report(&self, op: &str) -> Vec<String> {
        let mut lines = vec![format!(
            "agend-shim: snapshot {} saved before `git {op}` ({})",
            self.id, self.reference
        )];
        let restore = format!("git restore --source={} -- :/", self.reference);
        lines.push(match &self.previous_head {
            Some(head) => format!("agend-shim: to undo: git reset --keep {head} && {restore}"),
            None => format!("agend-shim: to undo: {restore}"),
        });
        lines
    }
}

/// Snapshots the working tree of `worktree` with the real git at `git`.
/// `id` must be unique per instance (the caller uses time and pid); a
/// second snapshot in the same process gets `-<n>` appended.
pub fn take(git: &Path, worktree: &Path, instance: &str, id: &str) -> Result<Saved, String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = match COUNTER.fetch_add(1, Ordering::Relaxed) {
        0 => id.to_string(),
        n => format!("{id}-{n}"),
    };
    let index =
        TempIndex(std::env::temp_dir().join(format!("agend-snapshot-{instance}-{id}.index")));
    let run = |args: &[&str], with_index: bool| -> Result<String, String> {
        let mut cmd = Command::new(git);
        // The shim's own plumbing: it writes only a new snapshot ref, and
        // neither the agend hooks nor the project's hooks are for it.
        cmd.arg("-C")
            .arg(worktree)
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(args);
        for var in crate::ctx::RETARGET_ENV {
            cmd.env_remove(var);
        }
        if with_index {
            cmd.env("GIT_INDEX_FILE", &index.0);
        }
        for var in ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"] {
            cmd.env(var, "agend-shim");
        }
        for var in ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL"] {
            cmd.env(var, "agend-shim@localhost");
        }
        let out = cmd
            .output()
            .map_err(|e| format!("cannot run {}: {e}", git.display()))?;
        match out.status.success() {
            true => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
            false => Err(format!(
                "`git {}` failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
        }
    };
    let previous_head = run(&["rev-parse", "--verify", "-q", "HEAD"], false).ok();
    if let Some(head) = &previous_head {
        run(&["read-tree", head], true)?;
    }
    run(&["add", "-A"], true)?;
    let tree = run(&["write-tree"], true)?;
    let message = format!("agend snapshot {id}");
    let mut args = vec!["commit-tree", &tree, "-m", &message];
    if let Some(head) = &previous_head {
        args.extend(["-p", head]);
    }
    let commit = run(&args, false)?;
    let reference = format!("{REF_PREFIX}{instance}/{id}");
    // Create-only (empty old value): an existing snapshot is never overwritten.
    run(&["update-ref", &reference, &commit, ""], false)?;
    Ok(Saved {
        id,
        reference,
        previous_head,
    })
}

/// A temporary index file, removed on drop.
struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests;
