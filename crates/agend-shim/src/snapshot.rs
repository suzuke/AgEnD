//! Takes a snapshot before destructive git operations (`reset --hard`,
//! `clean -f`, `checkout -- <paths>`, `restore`, `switch --discard-changes`)
//! so they can be undone.
//!
//! A snapshot is a commit of the whole working tree (tracked changes plus
//! untracked, non-ignored files; built in a temporary index, so the real
//! index is untouched) whose parent is the previous HEAD, kept at
//! `refs/agend/snapshots/<instance>/<id>`. Ignored files (`clean -x`) are not
//! captured. The shim never prunes snapshots.
//!
//! Must NOT: run for read-only commands.

use std::ffi::OsStr;
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
    pub commit: String,
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
/// `id` must be unique per instance (the caller uses time and pid).
pub fn take(git: &Path, worktree: &Path, instance: &str, id: &str) -> Result<Saved, String> {
    let run = |args: &[&OsStr], index: Option<&Path>| -> Result<String, String> {
        let mut cmd = Command::new(git);
        cmd.arg("-C").arg(worktree).args(args);
        for var in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
        ] {
            cmd.env_remove(var);
        }
        if let Some(index) = index {
            cmd.env("GIT_INDEX_FILE", index);
        }
        cmd.env("GIT_AUTHOR_NAME", "agend-shim")
            .env("GIT_AUTHOR_EMAIL", "agend-shim@localhost")
            .env("GIT_COMMITTER_NAME", "agend-shim")
            .env("GIT_COMMITTER_EMAIL", "agend-shim@localhost");
        let out = cmd
            .output()
            .map_err(|e| format!("cannot run {}: {e}", git.display()))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            Err(format!(
                "`git {}` failed: {}",
                args.iter()
                    .map(|a| a.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    };
    let os = |s: &'static str| OsStr::new(s);

    let previous_head = run(
        &[os("rev-parse"), os("--verify"), os("-q"), os("HEAD")],
        None,
    )
    .ok();
    let index = TempIndex::new(instance, id);
    let result = (|| {
        if let Some(head) = &previous_head {
            run(&[os("read-tree"), OsStr::new(head)], Some(&index.0))?;
        }
        run(&[os("add"), os("-A")], Some(&index.0))?;
        let tree = run(&[os("write-tree")], Some(&index.0))?;
        let message = format!("agend snapshot {id}");
        let mut args = vec![
            os("commit-tree"),
            OsStr::new(&tree),
            os("-m"),
            OsStr::new(&message),
        ];
        if let Some(head) = &previous_head {
            args.extend([os("-p"), OsStr::new(head)]);
        }
        let commit = run(&args, None)?;
        // Create-only (empty old value), so an existing snapshot is never
        // overwritten; on a clash try `<id>-2`, `<id>-3`, ...
        let mut last_err = String::new();
        for n in 1..=5 {
            let id = if n == 1 {
                id.to_string()
            } else {
                format!("{id}-{n}")
            };
            let reference = format!("{REF_PREFIX}{instance}/{id}");
            match run(
                &[
                    os("update-ref"),
                    OsStr::new(&reference),
                    OsStr::new(&commit),
                    os(""),
                ],
                None,
            ) {
                Ok(_) => {
                    return Ok(Saved {
                        id,
                        reference,
                        commit,
                        previous_head: previous_head.clone(),
                    });
                }
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    })();
    drop(index);
    result
}

/// A temporary index file, removed on drop.
struct TempIndex(PathBuf);

impl TempIndex {
    /// Unique per call: `id` alone repeats within one second of one process.
    fn new(instance: &str, id: &str) -> TempIndex {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        TempIndex(
            std::env::temp_dir().join(format!("agend-snapshot-{instance}-{id}-{nanos}-{n}.index")),
        )
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_names_the_undo_command() {
        let saved = Saved {
            id: "1-2".into(),
            reference: format!("{REF_PREFIX}dev-1/1-2"),
            commit: "c0ffee".into(),
            previous_head: Some("abc".into()),
        };
        let lines = saved.report("reset --hard HEAD~1");
        assert!(lines[0].contains("snapshot 1-2 saved before `git reset --hard HEAD~1`"));
        assert_eq!(
            lines[1],
            "agend-shim: to undo: git reset --keep abc && git restore --source=refs/agend/snapshots/dev-1/1-2 -- :/"
        );
    }
}
