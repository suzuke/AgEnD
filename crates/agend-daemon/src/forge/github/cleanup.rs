//! Remote cleanup is independently durable; unknown deletes never replay.
use super::{GithubForge, push::remote_head};
use agend_core::github::{GithubStore, VersionedGithubChange};

impl GithubForge {
    async fn save_cleanup(&self, record: &mut VersionedGithubChange) -> Result<(), String> {
        if !self
            .store
            .save_github_change(Some(record.revision), &record.change)
            .await
            .map_err(|e| e.to_string())?
        {
            return Err("GitHub cleanup ownership revision changed".into());
        }
        record.revision += 1;
        Ok(())
    }

    pub async fn cleanup(&self, task: &str, merged: bool) -> Result<(), String> {
        let Some(mut record) = self
            .store
            .github_change(task)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(());
        };
        if record.change.cleanup.complete {
            return Ok(());
        }
        if record.change.identity.local_repo != self.repo.to_string_lossy() {
            return Err("GitHub cleanup local repository differs".into());
        }
        let repository = self.repository().await.map_err(|e| e.to_string())?;
        if repository.name != record.change.identity.repository
            || repository
                .repository_id()
                .await
                .map_err(|e| e.to_string())?
                != record.change.identity.repository_id
        {
            return Err("GitHub cleanup repository identity differs".into());
        }
        let branch = record.change.identity.branch.clone();
        let origin = self
            .git
            .run(&self.repo, &["remote", "get-url", "origin"])
            .await?;
        if super::api::repository_from_origin(&origin)? != record.change.identity.repository {
            return Err("GitHub cleanup origin changed".into());
        }
        let pull = if let Some(number) = record.change.pull_number {
            Some(
                repository
                    .pull(number, &branch)
                    .await
                    .map_err(|e| e.to_string())?,
            )
        } else if record.change.create_attempted {
            let pull = repository
                .find_owned_pull(&record.change)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("GitHub PR creation outcome remains unknown; preserve remote branch")?;
            record.change.pull_number = Some(pull.number);
            self.save_cleanup(&mut record).await?;
            Some(pull)
        } else {
            None
        };
        if let Some(mut pull) = pull {
            repository
                .owned_pull(&record.change, &pull)
                .map_err(|e| e.to_string())?;
            if merged {
                let approved = record
                    .change
                    .pushed_head
                    .as_deref()
                    .ok_or("GitHub cleanup has no confirmed head")?;
                repository
                    .receipt(&pull, approved)
                    .await
                    .map_err(|e| e.to_string())?;
            } else if pull.merged {
                return Err(
                    "GitHub PR merged outside task completion; inspect before cleanup".into(),
                );
            } else if !pull.closed {
                if record.change.pushed_head.as_deref() != Some(&pull.head)
                    && record.change.push_intent.as_deref() != Some(&pull.head)
                {
                    return Err("GitHub PR head changed; cleanup will not close it".into());
                }
                if record.change.cleanup.close_attempted {
                    return Err(
                        "GitHub PR close outcome unknown; close the owned PR before retrying"
                            .into(),
                    );
                }
                record.change.cleanup.close_attempted = true;
                self.save_cleanup(&mut record).await?;
                let _reply = repository
                    .api
                    .request(
                        "PATCH",
                        &repository
                            .endpoint(&format!("pulls/{}", pull.number))
                            .map_err(|e| e.to_string())?,
                        &[("state", "closed")],
                    )
                    .await;
                pull = repository
                    .pull(pull.number, &branch)
                    .await
                    .map_err(|e| e.to_string())?;
                repository
                    .owned_pull(&record.change, &pull)
                    .map_err(|e| e.to_string())?;
                if !pull.closed || pull.merged {
                    return Err("GitHub PR close not confirmed; remote branch retained".into());
                }
            }
        } else if merged {
            return Err("GitHub completed task has no owned PR".into());
        }
        let remote = remote_head(&self.git, &self.repo, &origin, &branch).await?;
        if let Some(head) = remote {
            if record.change.cleanup.delete_attempted {
                return Err("GitHub branch deletion outcome unknown or branch recreated; no repeat deletion".into());
            }
            if let Some(intent) = &record.change.push_intent {
                if intent != &head {
                    return Err(
                        "GitHub pending push differs from remote branch; preserve it".into(),
                    );
                }
                record.change.pushed_head = record.change.push_intent.take();
                self.save_cleanup(&mut record).await?;
            }
            if record.change.pushed_head.as_deref() != Some(&head) {
                return Err("GitHub remote branch changed; cleanup will not delete it".into());
            }
            record.change.cleanup.delete_attempted = true;
            self.save_cleanup(&mut record).await?;
            let reference = format!("refs/heads/{branch}");
            let lease = format!("--force-with-lease={reference}:{head}");
            let delete = format!(":{reference}");
            let _reply = self
                .git
                .output(
                    &self.repo,
                    &[
                        "push",
                        "--porcelain",
                        "--no-follow-tags",
                        "--no-mirror",
                        &lease,
                        "--",
                        &origin,
                        &delete,
                    ],
                )
                .await;
            if remote_head(&self.git, &self.repo, &origin, &branch)
                .await?
                .is_some()
            {
                return Err(
                    "GitHub branch deletion not confirmed; original attempt retained".into(),
                );
            }
        }
        record.change.cleanup.complete = true;
        self.save_cleanup(&mut record).await
    }
}
