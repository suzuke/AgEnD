//! Single-attempt PR creation, reconciled by durable ownership after lost replies.
use super::{client::Repository, pull::Pull};
use agend_core::{
    github::{GithubChange, GithubStore, VersionedGithubChange},
    pipeline::ports::ExecutionError,
    traits::Runner,
};

fn blocked(reason: impl Into<String>) -> ExecutionError {
    ExecutionError::Blocked(reason.into())
}

fn marker(change: &GithubChange) -> String {
    format!(
        "<!-- agend:{}:{} -->",
        change.identity.task_id, change.identity.nonce
    )
}

impl<R: Runner> Repository<R>
where
    R::Error: std::fmt::Display,
{
    /// Names alone do not identify a repository after deletion/recreation.
    pub async fn repository_id(&self) -> Result<u64, ExecutionError> {
        let endpoint = self.endpoint("pulls")?;
        let endpoint = endpoint.strip_suffix("/pulls").unwrap();
        let reply = self
            .api
            .request("GET", endpoint, &[])
            .await
            .map_err(blocked)?;
        if reply.status != 200
            || reply.value["full_name"].as_str() != Some(&self.name)
            || reply.value["html_url"].as_str()
                != Some(&format!("https://github.com/{}", self.name))
        {
            return Err(blocked("GitHub repository identity lookup failed"));
        }
        reply.value["id"]
            .as_u64()
            .filter(|id| *id > 0 && *id <= i64::MAX as u64)
            .ok_or_else(|| blocked("GitHub repository identity missing"))
    }

    pub(super) fn owned_pull(
        &self,
        change: &GithubChange,
        pull: &Pull,
    ) -> Result<(), ExecutionError> {
        if pull.repository_id != change.identity.repository_id
            || !pull
                .body
                .as_deref()
                .is_some_and(|body| body.lines().any(|line| line == marker(change)))
        {
            return Err(blocked(
                "GitHub PR ownership differs; no mutation permitted",
            ));
        }
        Ok(())
    }

    /// Search only the fixed branch/base. Never adopt a foreign PR, even when
    /// it has the same head. A saturated search is inconclusive, not absent.
    pub(super) async fn find_owned_pull(
        &self,
        change: &GithubChange,
    ) -> Result<Option<Pull>, ExecutionError> {
        let owner = self
            .name
            .split_once('/')
            .ok_or_else(|| blocked("invalid repository"))?
            .0;
        let head = format!("{owner}:{}", change.identity.branch);
        let mut found = None;
        for page in 1..=10 {
            let page_text = page.to_string();
            let reply = self
                .api
                .request(
                    "GET",
                    &self.endpoint("pulls")?,
                    &[
                        ("state", "all"),
                        ("head", &head),
                        ("base", &self.base),
                        ("per_page", "100"),
                        ("page", &page_text),
                    ],
                )
                .await
                .map_err(blocked)?;
            if reply.status != 200 {
                return Err(blocked("GitHub PR search failed"));
            }
            let list = reply
                .value
                .as_array()
                .ok_or_else(|| blocked("invalid GitHub PR list"))?;
            for item in list {
                let number = item["number"]
                    .as_u64()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| blocked("GitHub PR list has no number"))?;
                let pull = self.pull(number, &change.identity.branch).await?;
                self.owned_pull(change, &pull)?;
                if found.is_some() {
                    return Err(blocked(
                        "multiple PRs match owned branch; inspect before proceeding",
                    ));
                }
                found = Some(pull);
            }
            if list.len() < 100 {
                return Ok(found);
            }
        }
        Err(blocked(
            "GitHub PR search exceeded bounded pages; no mutation permitted",
        ))
    }

    /// The branch must already have been pushed and confirmed in the ledger.
    pub async fn submit_pull<S: GithubStore>(
        &self,
        store: &S,
        mut record: VersionedGithubChange,
        title: &str,
        body: &str,
    ) -> Result<Pull, ExecutionError>
    where
        S::Error: std::fmt::Display,
    {
        let identity = &record.change.identity;
        if identity.repository != self.name
            || identity.base != self.base
            || self.repository_id().await? != identity.repository_id
            || record.change.push_intent.is_some()
            || record.change.pushed_head.is_none()
        {
            return Err(blocked(
                "GitHub submission identity or pushed head is not confirmed",
            ));
        }
        let branch = identity.branch.clone();
        let pull = if let Some(number) = record.change.pull_number {
            let pull = self.pull(number, &branch).await?;
            self.owned_pull(&record.change, &pull)?;
            pull
        } else if let Some(pull) = self.find_owned_pull(&record.change).await? {
            if !record.change.create_attempted {
                return Err(blocked("unattempted GitHub creation has an unexpected PR"));
            }
            pull
        } else {
            if record.change.create_attempted {
                return Err(blocked(
                    "GitHub PR creation outcome unknown; original attempt will not be replayed",
                ));
            }
            record.change.create_attempted = true;
            if !store
                .save_github_change(Some(record.revision), &record.change)
                .await
                .map_err(|e| blocked(e.to_string()))?
            {
                return Err(blocked("GitHub submission ownership revision changed"));
            }
            record.revision += 1;
            let body = format!("{body}\n\n{}", marker(&record.change));
            // A malformed/lost response is never retried. Read the same branch
            // and marker instead; recovery on the next boot follows that path.
            let _reply = self
                .api
                .request(
                    "POST",
                    &self.endpoint("pulls")?,
                    &[
                        ("title", title),
                        ("body", &body),
                        ("head", &branch),
                        ("base", &self.base),
                    ],
                )
                .await;
            self.find_owned_pull(&record.change).await?.ok_or_else(|| {
                blocked("GitHub PR creation not confirmed; original attempt retained")
            })?
        };
        if record.change.pull_number.is_none() {
            record.change.pull_number = Some(pull.number);
            if !store
                .save_github_change(Some(record.revision), &record.change)
                .await
                .map_err(|e| blocked(e.to_string()))?
            {
                return Err(blocked("GitHub PR receipt ownership revision changed"));
            }
        }
        if pull.closed || Some(&pull.head) != record.change.pushed_head.as_ref() {
            return Err(blocked("GitHub submitted PR is closed or its head changed"));
        }
        Ok(pull)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{forge::github::api::Api, store::SqliteStore};
    use agend_core::{
        github::GithubIdentity,
        pipeline::task::Task,
        traits::{CommandOutput, Store},
    };
    use agend_testkit::{block_on, tempdir::TempDir};
    use serde_json::{Value, json};
    use std::{collections::VecDeque, sync::Mutex};

    struct Replay {
        replies: Mutex<VecDeque<Option<Value>>>,
        writes: Mutex<usize>,
    }
    impl Runner for Replay {
        type Error = String;
        async fn run(&self, command: &str, _: &str, _: u64) -> Result<CommandOutput, String> {
            if command.contains("'--method' 'POST'") {
                *self.writes.lock().unwrap() += 1;
            }
            let value = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected request")
                .ok_or("injected lost response")?;
            let native = include_bytes!("../../../tests/fixtures/github/pull.http");
            let at = native.windows(4).position(|s| s == b"\r\n\r\n").unwrap() + 4;
            let mut stdout = native[..at].to_vec();
            stdout.extend(serde_json::to_vec(&value).unwrap());
            Ok(CommandOutput {
                stdout,
                stderr: vec![],
                exit_code: Some(0),
                timed_out: false,
            })
        }
    }
    fn native_repo() -> Value {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/github/repository.json"
        ))
        .unwrap()
    }
    fn native_pull(change: &GithubChange) -> Value {
        let bytes = include_bytes!("../../../tests/fixtures/github/pull.http");
        let at = bytes.windows(4).position(|s| s == b"\r\n\r\n").unwrap() + 4;
        let mut value: Value = serde_json::from_slice(&bytes[at..]).unwrap();
        value["body"] = Value::String(marker(change));
        value
    }
    fn repository(replies: Vec<Option<Value>>) -> Repository<Replay> {
        Repository {
            name: "suzuke/AgEnD".into(),
            base: "v2".into(),
            api: Api {
                executable: "/gh".into(),
                directory: "/repo".into(),
                runner: Replay {
                    replies: Mutex::new(replies.into()),
                    writes: Mutex::new(0),
                },
            },
        }
    }
    async fn prepared(store: &SqliteStore) -> VersionedGithubChange {
        store
            .create_task(&Task::new("t-1", "test", "general", "code", 1))
            .await
            .unwrap();
        let mut change = GithubChange {
            identity: GithubIdentity {
                task_id: "t-1".into(),
                local_repo: "/repo".into(),
                repository: "suzuke/AgEnD".into(),
                repository_id: native_repo()["id"].as_u64().unwrap(),
                base: "v2".into(),
                branch: "feat/g12b-opencode".into(),
                nonce: "test-owned".into(),
            },
            pull_number: None,
            pushed_head: None,
            push_intent: None,
            create_attempted: false,
            cleanup: Default::default(),
        };
        store.save_github_change(None, &change).await.unwrap();
        change.push_intent = Some(native_pull(&change)["head"]["sha"].as_str().unwrap().into());
        store.save_github_change(Some(1), &change).await.unwrap();
        change.pushed_head = change.push_intent.take();
        store.save_github_change(Some(2), &change).await.unwrap();
        store.github_change("t-1").await.unwrap().unwrap()
    }
    #[test]
    fn lost_creation_reply_recovers_same_pr_once_and_reopen_only_reads() {
        block_on(async {
            let dir = TempDir::new("github-create").unwrap();
            let store = SqliteStore::open(dir.path(), 0).unwrap();
            let record = prepared(&store).await;
            let pull = native_pull(&record.change);
            let repo = repository(vec![
                Some(native_repo()),
                Some(json!([])),
                None,
                Some(json!([pull.clone()])),
                Some(pull.clone()),
            ]);
            assert_eq!(
                repo.submit_pull(&store, record, "test", "body")
                    .await
                    .unwrap()
                    .number,
                155
            );
            assert_eq!(*repo.api.runner.writes.lock().unwrap(), 1);
            drop(store);
            let store = SqliteStore::open(dir.path(), 1).unwrap();
            let restored = store.github_change("t-1").await.unwrap().unwrap();
            assert_eq!(restored.change.pull_number, Some(155));
            let repo = repository(vec![Some(native_repo()), Some(pull)]);
            assert_eq!(
                repo.submit_pull(&store, restored, "test", "body")
                    .await
                    .unwrap()
                    .number,
                155
            );
            assert_eq!(*repo.api.runner.writes.lock().unwrap(), 0);
        });
    }
    #[test]
    fn unknown_creation_is_not_replayed_and_foreign_pr_is_not_adopted() {
        block_on(async {
            let dir = TempDir::new("github-unknown-create").unwrap();
            let store = SqliteStore::open(dir.path(), 0).unwrap();
            let record = prepared(&store).await;
            let mut foreign = native_pull(&record.change);
            foreign["body"] = json!("foreign");
            let repo = repository(vec![
                Some(native_repo()),
                Some(json!([foreign.clone()])),
                Some(foreign),
            ]);
            assert!(
                repo.submit_pull(&store, record.clone(), "test", "body")
                    .await
                    .is_err()
            );
            assert_eq!(*repo.api.runner.writes.lock().unwrap(), 0);
            assert!(
                !store
                    .github_change("t-1")
                    .await
                    .unwrap()
                    .unwrap()
                    .change
                    .create_attempted
            );
            let repo = repository(vec![
                Some(native_repo()),
                Some(json!([])),
                None,
                Some(json!([])),
            ]);
            assert!(
                repo.submit_pull(&store, record, "test", "body")
                    .await
                    .is_err()
            );
            assert_eq!(*repo.api.runner.writes.lock().unwrap(), 1);
            drop(store);
            let store = SqliteStore::open(dir.path(), 1).unwrap();
            let record = store.github_change("t-1").await.unwrap().unwrap();
            let repo = repository(vec![Some(native_repo()), Some(json!([]))]);
            assert!(
                repo.submit_pull(&store, record, "test", "body")
                    .await
                    .is_err()
            );
            assert_eq!(*repo.api.runner.writes.lock().unwrap(), 0);
        });
    }
}
