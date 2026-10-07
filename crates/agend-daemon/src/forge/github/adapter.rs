//! Production Forge composition over git, GitHub API, and the durable ledger.
use super::{api::Api, client::Repository, push::push_owned};
use crate::{git::Git, store::SqliteStore};
use agend_core::{
    github::{GithubChange, GithubIdentity, GithubStore, VersionedGithubChange},
    model::task_id_of_branch,
    pipeline::ports::ExecutionError,
    traits::{Forge, MergeRequest, MergeResult, Submission, SubmittedChange},
};
use std::{path::PathBuf, sync::Arc};

pub struct GithubForge {
    pub api: Api,
    pub repo: PathBuf,
    pub git: Git,
    pub store: Arc<SqliteStore>,
}
fn blocked(s: impl Into<String>) -> ExecutionError {
    ExecutionError::Blocked(s.into())
}

impl GithubForge {
    pub(super) async fn repository(
        &self,
    ) -> Result<Repository<crate::runner::ProcessRunner>, ExecutionError> {
        let origin = self
            .git
            .run(&self.repo, &["remote", "get-url", "origin"])
            .await
            .map_err(blocked)?;
        Ok(Repository {
            name: super::api::repository_from_origin(&origin).map_err(blocked)?,
            base: "main".into(),
            api: self.api.clone(),
        })
    }
    async fn owned(&self, branch: &str) -> Result<VersionedGithubChange, ExecutionError> {
        let task = task_id_of_branch(branch)
            .ok_or_else(|| blocked("GitHub branch has no task identity"))?;
        let record = self
            .store
            .github_change(task)
            .await
            .map_err(|e| blocked(e.to_string()))?
            .ok_or_else(|| blocked("GitHub branch has no durable ownership"))?;
        if record.change.identity.branch != branch
            || record.change.identity.local_repo != self.repo.to_string_lossy()
        {
            return Err(blocked("GitHub branch differs from durable ownership"));
        }
        Ok(record)
    }
    /// Refresh the local main checkout only by a clean fast-forward. Never
    /// reset user commits or discard changes while synchronizing remote state.
    pub async fn sync_main(&self) -> Result<String, String> {
        let repository = self.repository().await.map_err(|e| e.to_string())?;
        let _ = repository
            .repository_id()
            .await
            .map_err(|e| e.to_string())?;
        let origin = self
            .git
            .run(&self.repo, &["remote", "get-url", "origin"])
            .await?;
        if super::api::repository_from_origin(&origin)? != repository.name {
            return Err("GitHub origin changed during base refresh".into());
        }
        let reference = "refs/agend/github/main";
        self.git
            .run(
                &self.repo,
                &[
                    "fetch",
                    "--no-tags",
                    "--no-write-fetch-head",
                    "--",
                    &origin,
                    "+refs/heads/main:refs/agend/github/main",
                ],
            )
            .await?;
        let remote = self
            .git
            .run(&self.repo, &["rev-parse", "--verify", reference])
            .await?;
        let local = self
            .git
            .run(&self.repo, &["rev-parse", "--verify", "refs/heads/main"])
            .await?;
        if local == remote {
            return Ok(remote);
        }
        if self.git.run(&self.repo, &["symbolic-ref", "HEAD"]).await? != "refs/heads/main"
            || !self.git.clean_worktree(&self.repo).await?
            || !self.git.ancestor(&self.repo, &local, &remote).await?
        {
            return Err(
                "GitHub main synchronization requires a clean main checkout that can fast-forward"
                    .into(),
            );
        }
        self.git
            .run(&self.repo, &["merge", "--ff-only", "--no-edit", &remote])
            .await?;
        Ok(remote)
    }

    pub async fn find_merge(
        &self,
        task: &str,
        head: &str,
    ) -> Result<Option<(String, bool)>, String> {
        let Some(record) = self
            .store
            .github_change(task)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let Some(number) = record.change.pull_number else {
            return Ok(None);
        };
        let repository = self.repository().await.map_err(|e| e.to_string())?;
        if repository.name != record.change.identity.repository
            || repository
                .repository_id()
                .await
                .map_err(|e| e.to_string())?
                != record.change.identity.repository_id
        {
            return Err("GitHub recovery repository differs from ownership".into());
        }
        let pull = repository
            .pull(number, &record.change.identity.branch)
            .await
            .map_err(|e| e.to_string())?;
        repository
            .owned_pull(&record.change, &pull)
            .map_err(|e| e.to_string())?;
        if !pull.merged {
            return Ok(None);
        }
        match repository
            .receipt(&pull, head)
            .await
            .map_err(|e| e.to_string())?
        {
            MergeResult::Merged { merge_commit } => {
                self.sync_main().await?;
                Ok(Some((merge_commit, false)))
            }
            MergeResult::HeadChanged { .. } => Err("GitHub merge receipt head changed".into()),
        }
    }
}

impl Forge for GithubForge {
    type Error = ExecutionError;
    async fn submit(&self, submission: &Submission) -> Result<SubmittedChange, ExecutionError> {
        if task_id_of_branch(&submission.branch) != Some(submission.task_id.as_str()) {
            return Err(blocked("GitHub submission branch does not belong to task"));
        }
        let repository = self.repository().await?;
        let id = repository.repository_id().await?;
        if self
            .store
            .github_change(&submission.task_id)
            .await
            .map_err(|e| blocked(e.to_string()))?
            .is_none()
        {
            let change = GithubChange {
                identity: GithubIdentity {
                    task_id: submission.task_id.clone(),
                    local_repo: self.repo.to_string_lossy().into_owned(),
                    repository: repository.name.clone(),
                    repository_id: id,
                    base: repository.base.clone(),
                    branch: submission.branch.clone(),
                    nonce: crate::store::instances::new_session_id()
                        .map_err(|e| blocked(e.to_string()))?,
                },
                pull_number: None,
                pushed_head: None,
                push_intent: None,
                create_attempted: false,
                cleanup: Default::default(),
            };
            // A concurrent claimant wins; load and validate that identity below.
            let _ = self
                .store
                .save_github_change(None, &change)
                .await
                .map_err(|e| blocked(e.to_string()))?;
        }
        let record = self.owned(&submission.branch).await?;
        if record.change.identity.repository != repository.name
            || record.change.identity.repository_id != id
        {
            return Err(blocked("GitHub repository differs from durable ownership"));
        }
        let head = self
            .git
            .run(
                &self.repo,
                &[
                    "rev-parse",
                    "--verify",
                    &format!("refs/heads/{}^{{commit}}", submission.branch),
                ],
            )
            .await
            .map_err(blocked)?;
        let record = push_owned(&self.git, self.store.as_ref(), record, &head)
            .await
            .map_err(blocked)?;
        let pull = repository
            .submit_pull(
                self.store.as_ref(),
                record,
                &submission.title,
                &submission.body,
            )
            .await?;
        Ok(SubmittedChange {
            id: Some(pull.number.to_string()),
            url: Some(format!(
                "https://github.com/{}/pull/{}",
                repository.name, pull.number
            )),
            head: pull.head,
        })
    }
    async fn head(&self, branch: &str) -> Result<String, ExecutionError> {
        let task = task_id_of_branch(branch)
            .ok_or_else(|| blocked("GitHub branch has no task identity"))?;
        if self
            .store
            .github_change(task)
            .await
            .map_err(|e| blocked(e.to_string()))?
            .is_none()
        {
            return self
                .git
                .run(
                    &self.repo,
                    &[
                        "rev-parse",
                        "--verify",
                        &format!("refs/heads/{branch}^{{commit}}"),
                    ],
                )
                .await
                .map_err(blocked);
        }
        let record = self.owned(branch).await?;
        let repository = self.repository().await?;
        if repository.name != record.change.identity.repository
            || repository.repository_id().await? != record.change.identity.repository_id
        {
            return Err(blocked("GitHub repository differs from durable ownership"));
        }
        let number = record
            .change
            .pull_number
            .ok_or_else(|| blocked("GitHub head has no owned PR"))?;
        let pull = repository.pull(number, branch).await?;
        repository.owned_pull(&record.change, &pull)?;
        Ok(pull.head)
    }
    async fn merge_if_head_is(
        &self,
        request: &MergeRequest,
    ) -> Result<MergeResult, ExecutionError> {
        let record = self.owned(&request.branch).await?;
        if record.change.push_intent.is_some() {
            return Err(blocked("GitHub push has an unresolved outcome"));
        }
        let result = self
            .repository()
            .await?
            .merge_owned(&record.change, &request.expected_head)
            .await?;
        if matches!(result, MergeResult::Merged { .. }) {
            self.sync_main().await.map_err(blocked)?;
        }
        Ok(result)
    }
}
