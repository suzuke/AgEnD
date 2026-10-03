//! Local forge: durable merge intent precedes any mutation of main.
use crate::git::Git;
use crate::store::SqliteStore;
use agend_core::model::task_id_of_branch;
use agend_core::traits::{
    CasResult, Forge, MergeRequest, MergeResult, Store, StoredEvent, Submission, SubmittedChange,
};
use std::path::PathBuf;
use std::sync::Arc;

pub struct LocalForge<R = crate::runner::ProcessRunner, S = SqliteStore> {
    pub repo: PathBuf,
    pub git: Git<R>,
    pub store: Arc<S>,
    /// Base observed by the pipeline before checks/rebase reconciliation.
    pub expected_main: Option<String>,
}
pub use agend_core::pipeline::ports::ExecutionError as LocalError;
fn fail(s: String) -> LocalError {
    LocalError::Failed(s)
}

impl<R: agend_core::traits::Runner, S: Store + Send> LocalForge<R, S>
where
    R::Error: std::fmt::Display,
    S::Error: std::fmt::Display,
{
    pub async fn find_merge(
        &self,
        task: &str,
        head: &str,
    ) -> Result<Option<(String, bool)>, String> {
        if self
            .git
            .run(
                &self.repo,
                &["rev-parse", "--verify", &format!("{head}^{{commit}}")],
            )
            .await
            .ok()
            .as_deref()
            != Some(head)
        {
            return Ok(None);
        }
        let history = self
            .git
            .run(
                &self.repo,
                &[
                    "log",
                    "--first-parent",
                    "-1000",
                    "--format=%H%x00%P%x00%B%x00",
                    "main",
                ],
            )
            .await?;
        let fields = history.split('\0').collect::<Vec<_>>();
        for record in fields.chunks_exact(3) {
            let parents = record[1].split_whitespace().collect::<Vec<_>>();
            if parents.len() == 2
                && parents[1] == head
                && record[2]
                    .lines()
                    .any(|l| l == format!("Agend-Task: {task}"))
            {
                return Ok(Some((record[0].trim().into(), false)));
            }
        }
        if let Some(p) = self
            .store
            .load_task_progress(task)
            .await
            .map_err(|e| e.to_string())?
            && let Some(intent) = p.merge_intent
            && self.git.ancestor(&self.repo, &intent, "main").await?
        {
            let info = self
                .git
                .run(&self.repo, &["show", "-s", "--format=%P%n%B", &intent])
                .await?;
            let mut lines = info.lines();
            let parents = lines
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>();
            if parents.len() == 2
                && parents[1] == head
                && lines.any(|l| l == format!("Agend-Task: {task}"))
            {
                return Ok(Some((intent, false)));
            }
        }
        if self.git.ancestor(&self.repo, head, "main").await? {
            let commits = self
                .git
                .run(
                    &self.repo,
                    &["rev-list", "--first-parent", "--reverse", "main"],
                )
                .await?;
            for commit in commits.lines() {
                if self.git.ancestor(&self.repo, head, commit).await? {
                    return Ok(Some((commit.into(), true)));
                }
            }
        }
        Ok(None)
    }
}

impl<R: agend_core::traits::Runner, S: Store + Send> Forge for LocalForge<R, S>
where
    R::Error: std::fmt::Display,
    S::Error: std::fmt::Display,
{
    type Error = LocalError;
    async fn submit(&self, change: &Submission) -> Result<SubmittedChange, LocalError> {
        Ok(SubmittedChange {
            id: None,
            url: None,
            head: self.head(&change.branch).await?,
        })
    }
    async fn head(&self, branch: &str) -> Result<String, LocalError> {
        self.git
            .run(
                &self.repo,
                &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
            )
            .await
            .map_err(fail)
    }
    async fn merge_if_head_is(&self, request: &MergeRequest) -> Result<MergeResult, LocalError> {
        let task = task_id_of_branch(&request.branch)
            .ok_or_else(|| fail("branch is outside agend namespace".into()))?;
        if let Some((merge_commit, outside)) = self
            .find_merge(task, &request.expected_head)
            .await
            .map_err(fail)?
        {
            crate::log::line(&format!(
                "{task}: merge found on main {}({merge_commit}); not merged again",
                if outside {
                    "(merged outside agend) "
                } else {
                    "by trailer "
                }
            ));
            return Ok(MergeResult::Merged { merge_commit });
        }
        let actual = self.head(&request.branch).await?;
        if actual != request.expected_head {
            return Ok(MergeResult::HeadChanged {
                actual_head: actual,
            });
        }
        let main = self
            .git
            .run(&self.repo, &["rev-parse", "main"])
            .await
            .map_err(fail)?;
        if self
            .expected_main
            .as_ref()
            .is_some_and(|expected| expected != &main)
        {
            return Err(fail("main advanced; rebase required".into()));
        }
        let worktrees = self
            .git
            .run(&self.repo, &["worktree", "list", "--porcelain"])
            .await
            .map_err(fail)?;
        let mut main_checkout = None;
        for block in worktrees.split("\n\n") {
            if block.lines().any(|l| l == "branch refs/heads/main") {
                let path = block
                    .lines()
                    .find_map(|l| l.strip_prefix("worktree "))
                    .ok_or_else(|| fail("invalid worktree list".into()))?;
                let path = PathBuf::from(path)
                    .canonicalize()
                    .map_err(|e| fail(e.to_string()))?;
                if path != self.repo.canonicalize().map_err(|e| fail(e.to_string()))? {
                    return Err(LocalError::Blocked(format!(
                        "main is checked out at {}",
                        path.display()
                    )));
                }
                if !self.git.clean_worktree(&path).await.map_err(fail)? {
                    return Err(LocalError::Blocked(format!(
                        "canonical checkout {} is dirty or has concealed index entries; commit or stash changes and clear index flags",
                        path.display()
                    )));
                }
                main_checkout = Some(path);
            }
        }
        let tree = self
            .git
            .run(&self.repo, &["merge-tree", "--write-tree", &main, &actual])
            .await
            .map_err(fail)?;
        let tree = tree
            .lines()
            .next()
            .ok_or_else(|| fail("merge-tree returned no tree".into()))?;
        let row = self
            .store
            .load_task(task)
            .await
            .map_err(|e| fail(e.to_string()))?
            .ok_or_else(|| fail("unknown task".into()))?;
        let message = format!(
            "Merge {}: {}\n\nAgend-Task: {task}",
            request.branch, row.task.title
        );
        let commit = self
            .git
            .run(
                &self.repo,
                &[
                    "commit-tree",
                    tree,
                    "-p",
                    &main,
                    "-p",
                    &actual,
                    "-m",
                    &message,
                ],
            )
            .await
            .map_err(fail)?;
        let mut progress = self
            .store
            .load_task_progress(task)
            .await
            .map_err(|e| fail(e.to_string()))?
            .ok_or_else(|| fail("missing pipeline snapshot".into()))?;
        progress.merge_intent = Some(commit.clone());
        let event = StoredEvent {
            id: format!("merge-intent:{commit}"),
            occurred_at_unix_ms: crate::log::now_unix_ms(),
            kind: "merge_intent".into(),
            detail: commit.clone(),
        };
        if !matches!(
            self.store
                .advance_task(&row.task, row.version, &progress, &event)
                .await
                .map_err(|e| fail(e.to_string()))?,
            CasResult::Written { .. }
        ) {
            return Err(fail("merge intent CAS conflict".into()));
        }
        failpoint("after-merge-intent");
        // Recheck both refs immediately before the mutation; an intervening
        // non-ancestor main is rejected by ff-only, and a bare main uses CAS.
        if self
            .git
            .run(&self.repo, &["rev-parse", "main"])
            .await
            .map_err(fail)?
            != main
            || self.head(&request.branch).await? != actual
        {
            return Err(fail("refs moved before merge".into()));
        }
        if let Some(path) = main_checkout {
            self.git
                .run(&path, &["merge", "--ff-only", &commit])
                .await
                .map_err(fail)?;
        } else {
            self.git
                .run(
                    &self.repo,
                    &["update-ref", "refs/heads/main", &commit, &main],
                )
                .await
                .map_err(fail)?;
        }
        crate::log::line(&format!("{task}: merge: main moved to {commit}"));
        failpoint("after-main-moved");
        Ok(MergeResult::Merged {
            merge_commit: commit,
        })
    }
}
fn failpoint(name: &str) {
    #[cfg(debug_assertions)]
    if std::env::var("AGEND_FAILPOINT").ok().as_deref() == Some(name) {
        crate::log::line(&format!("aborted at failpoint {name}"));
        std::process::abort();
    }
    #[cfg(not(debug_assertions))]
    let _ = name;
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::pipeline::task::Task;
    use agend_core::traits::TaskProgress;
    use agend_testkit::{
        block_on,
        fakes::{FakeRunner, FakeStore, ScriptedCommand},
        tempdir::TempDir,
    };
    #[test]
    fn failed_intent_storage_never_mutates_main() {
        let dir = TempDir::new("g10-forge-failure").unwrap();
        let store = Arc::new(FakeStore::new());
        let task = Task::new("t-1", "Work", "general", "code", 1);
        block_on(store.create_task(&task)).unwrap();
        let progress = TaskProgress {
            pipeline: "{}".into(),
            stage_entered_at_unix_ms: 123,
            merge_intent: None,
            block_reason: None,
        };
        let event = StoredEvent {
            id: "initial".into(),
            occurred_at_unix_ms: 123,
            kind: "pipeline".into(),
            detail: "".into(),
        };
        block_on(store.advance_task(&task, 1, &progress, &event)).unwrap();
        let runner = FakeRunner::new();
        let script = |args: &[&str], code, output: &str| {
            let mut command =
                "'git' -c core.hooksPath=/dev/null -c core.fsmonitor=false".to_string();
            for arg in args {
                command.push(' ');
                command.push_str(&crate::runner::quote(arg));
            }
            runner.on(
                &command,
                ScriptedCommand::exits(code).stdout(output.as_bytes().to_vec()),
            );
        };
        script(&["rev-parse", "--verify", "h^{commit}"], 0, "h");
        script(
            &[
                "log",
                "--first-parent",
                "-1000",
                "--format=%H%x00%P%x00%B%x00",
                "main",
            ],
            0,
            "",
        );
        script(&["merge-base", "--is-ancestor", "h", "main"], 1, "");
        script(
            &["rev-parse", "--verify", "refs/heads/agend/t-1/work"],
            0,
            "h",
        );
        script(&["rev-parse", "main"], 0, "m");
        script(&["worktree", "list", "--porcelain"], 0, "");
        script(&["merge-tree", "--write-tree", "m", "h"], 0, "tree");
        script(
            &[
                "commit-tree",
                "tree",
                "-p",
                "m",
                "-p",
                "h",
                "-m",
                "Merge agend/t-1/work: Work\n\nAgend-Task: t-1",
            ],
            0,
            "merge",
        );
        let forge = LocalForge {
            repo: dir.path().into(),
            git: Git {
                executable: "git".into(),
                runner,
            },
            store: store.clone(),
            expected_main: Some("m".into()),
        };
        store.fail_next("advance_task", "disk full");
        let error = block_on(forge.merge_if_head_is(&MergeRequest {
            branch: "agend/t-1/work".into(),
            expected_head: "h".into(),
        }))
        .unwrap_err();
        assert!(error.to_string().contains("disk full"), "{error}");
        assert_eq!(
            block_on(store.load_task_progress("t-1")).unwrap(),
            Some(progress)
        );
        assert_eq!(store.events("t-1").len(), 1);
        assert!(
            forge
                .git
                .runner
                .calls()
                .iter()
                .all(|c| !c.command.contains("'update-ref'") && !c.command.contains("'merge' '")),
            "main moved before the intent was durable"
        );
    }
}
