//! One explicit repository and pull identity; recover through reads before writes.
use super::{api::Api, pull::Pull};
use agend_core::pipeline::ports::ExecutionError;
use agend_core::traits::{MergeResult, Runner};

pub struct Repository<R> {
    pub api: Api<R>,
    pub name: String,
    pub base: String,
}

fn blocked(message: impl Into<String>) -> ExecutionError {
    ExecutionError::Blocked(message.into())
}

impl<R: Runner> Repository<R>
where
    R::Error: std::fmt::Display,
{
    pub(super) fn endpoint(&self, suffix: &str) -> Result<String, ExecutionError> {
        if super::api::repository_from_origin(&format!("https://github.com/{}", self.name))
            .as_deref()
            != Ok(self.name.as_str())
        {
            return Err(blocked("invalid GitHub repository identity"));
        }
        Ok(format!("repos/{}/{suffix}", self.name))
    }

    pub async fn pull(&self, number: u64, branch: &str) -> Result<Pull, ExecutionError> {
        let endpoint = self.endpoint(&format!("pulls/{number}"))?;
        let reply = self
            .api
            .request("GET", &endpoint, &[])
            .await
            .map_err(blocked)?;
        if reply.status != 200 {
            return Err(blocked(format!(
                "GitHub pull lookup returned HTTP {}",
                reply.status
            )));
        }
        Pull::parse(&reply.value, &self.name, number, branch, &self.base).map_err(blocked)
    }

    pub(super) async fn receipt(
        &self,
        pull: &Pull,
        approved: &str,
    ) -> Result<MergeResult, ExecutionError> {
        let sha = pull
            .receipt_for(approved)
            .map_err(blocked)?
            .ok_or_else(|| blocked("GitHub has not confirmed this merge"))?;
        let endpoint = self.endpoint(&format!("git/commits/{sha}"))?;
        let reply = self
            .api
            .request("GET", &endpoint, &[])
            .await
            .map_err(blocked)?;
        if reply.status != 200 {
            return Err(blocked(format!(
                "GitHub merge commit lookup returned HTTP {}",
                reply.status
            )));
        }
        let sha = pull
            .verify_merge_commit(approved, &reply.value)
            .map_err(blocked)?;
        Ok(MergeResult::Merged {
            merge_commit: sha.into(),
        })
    }

    /// The caller must supply its persisted PR identity, never a newly guessed PR.
    pub async fn merge(
        &self,
        number: u64,
        branch: &str,
        approved: &str,
    ) -> Result<MergeResult, ExecutionError> {
        self.merge_checked(number, branch, approved, None, async || Ok(()))
            .await
    }

    pub async fn merge_owned(
        &self,
        change: &agend_core::github::GithubChange,
        approved: &str,
    ) -> Result<MergeResult, ExecutionError> {
        self.merge_owned_before_write(change, approved, async || Ok(()))
            .await
    }

    pub(super) async fn merge_owned_before_write(
        &self,
        change: &agend_core::github::GithubChange,
        approved: &str,
        before_write: impl AsyncFnOnce() -> Result<(), ExecutionError>,
    ) -> Result<MergeResult, ExecutionError> {
        if self.name != change.identity.repository
            || self.base != change.identity.base
            || self.repository_id().await? != change.identity.repository_id
        {
            return Err(blocked("GitHub merge repository differs from ownership"));
        }
        let number = change
            .pull_number
            .ok_or_else(|| blocked("GitHub merge has no owned PR"))?;
        self.merge_checked(
            number,
            &change.identity.branch,
            approved,
            Some(change),
            before_write,
        )
        .await
    }

    async fn merge_checked(
        &self,
        number: u64,
        branch: &str,
        approved: &str,
        ownership: Option<&agend_core::github::GithubChange>,
        before_write: impl AsyncFnOnce() -> Result<(), ExecutionError>,
    ) -> Result<MergeResult, ExecutionError> {
        let before = self.pull(number, branch).await?;
        if let Some(change) = ownership {
            self.owned_pull(change, &before)?;
        }
        if before.head != approved || !super::pull::full_sha(approved) {
            return Ok(MergeResult::HeadChanged {
                actual_head: before.head,
            });
        }
        if before.merged {
            return self.receipt(&before, approved).await;
        }
        if before.closed {
            return Err(blocked("GitHub pull request was closed without merging"));
        }
        self.require_strict_base().await?;
        let endpoint = self.endpoint(&format!("pulls/{number}/merge"))?;
        before_write().await?;
        let written = self
            .api
            .request(
                "PUT",
                &endpoint,
                &[("sha", approved), ("merge_method", "merge")],
            )
            .await;
        // Even a lost or refused response can race a successful remote merge.
        // Reconcile only the same PR/head and prove the returned commit's parents.
        let after = self.pull(number, branch).await?;
        if let Some(change) = ownership {
            self.owned_pull(change, &after)?;
        }
        if after.head != approved {
            return Ok(MergeResult::HeadChanged {
                actual_head: after.head,
            });
        }
        if after.merged {
            if let Ok(reply) = &written
                && reply.status == 200
                && (reply.value["merged"].as_bool() != Some(true)
                    || reply.value["sha"].as_str() != after.merge_commit.as_deref())
            {
                return Err(blocked("GitHub merge response and durable receipt differ"));
            }
            return self.receipt(&after, approved).await;
        }
        let reason = match written {
            Ok(reply) => format!(
                "GitHub merge not confirmed (HTTP {}); inspect the original PR before retrying",
                reply.status
            ),
            Err(_) => {
                "GitHub merge outcome unknown; inspect the original PR before retrying".into()
            }
        };
        Err(blocked(reason))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::traits::CommandOutput;
    use serde_json::Value;
    use std::{collections::VecDeque, sync::Mutex};

    struct Replay {
        replies: Mutex<VecDeque<Vec<u8>>>,
        commands: Mutex<Vec<String>>,
    }
    impl Runner for Replay {
        type Error = String;
        async fn run(&self, command: &str, _: &str, _: u64) -> Result<CommandOutput, String> {
            self.commands.lock().unwrap().push(command.into());
            let stdout = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected additional request");
            Ok(CommandOutput {
                exit_code: Some(0),
                stdout,
                stderr: vec![],
                timed_out: false,
            })
        }
    }
    fn recorded() -> (Value, Value) {
        (
            serde_json::from_str(include_str!(
                "../../../tests/fixtures/github/merged-pull.json"
            ))
            .unwrap(),
            serde_json::from_str(include_str!(
                "../../../tests/fixtures/github/merge-commit.json"
            ))
            .unwrap(),
        )
    }
    fn response(value: &Value) -> Vec<u8> {
        let native = include_bytes!("../../../tests/fixtures/github/pull.http");
        let boundary = native.windows(4).position(|s| s == b"\r\n\r\n").unwrap() + 4;
        let mut bytes = native[..boundary].to_vec();
        bytes.extend(serde_json::to_vec(value).unwrap());
        bytes
    }
    fn repository(replies: Vec<Vec<u8>>) -> Repository<Replay> {
        Repository {
            name: "suzuke/AgEnD".into(),
            base: "v2".into(),
            api: Api {
                executable: "/gh".into(),
                directory: "/repo".into(),
                runner: Replay {
                    replies: Mutex::new(replies.into()),
                    commands: Mutex::new(vec![]),
                },
            },
        }
    }
    #[test]
    fn unavailable_or_weak_protection_never_sends_a_merge_put() {
        let (mut pending, _) = recorded();
        pending["merged"] = Value::Bool(false);
        pending["state"] = serde_json::json!("open");
        pending["merge_commit_sha"] = Value::Null;
        let weak: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/github/branch-protection.json"
        ))
        .unwrap();
        for policy in [
            response(&weak),
            include_bytes!("../../../tests/fixtures/github/not-found.http").to_vec(),
            vec![],
        ] {
            let repository = repository(vec![response(&pending), policy]);
            assert!(
                agend_testkit::block_on(repository.merge(
                    154,
                    "test/g12a-smoke-contract",
                    pending["head"]["sha"].as_str().unwrap()
                ))
                .is_err()
            );
            let calls = repository.api.runner.commands.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert!(calls.iter().all(|c| c.contains("'--method' 'GET'")));
        }
    }
    #[test]
    fn failed_durable_intent_never_sends_the_put() {
        let (mut pending, _) = recorded();
        pending["merged"] = Value::Bool(false);
        pending["state"] = serde_json::json!("open");
        pending["merge_commit_sha"] = Value::Null;
        let repository = repository(vec![
            response(&pending),
            response(&super::super::protection::tests::protected()),
        ]);
        let called = std::sync::atomic::AtomicBool::new(false);
        let result = agend_testkit::block_on(repository.merge_checked(
            154,
            "test/g12a-smoke-contract",
            pending["head"]["sha"].as_str().unwrap(),
            None,
            async || {
                called.store(true, std::sync::atomic::Ordering::SeqCst);
                Err(blocked("injected CAS failure"))
            },
        ));
        assert!(result.is_err());
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
        let calls = repository.api.runner.commands.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|c| c.contains("'--method' 'GET'")));
    }
    #[test]
    fn a_recorded_merged_pull_recovers_without_any_mutation() {
        let (pull, commit) = recorded();
        let repository = repository(vec![response(&pull), response(&commit)]);
        let result = agend_testkit::block_on(repository.merge(
            154,
            "test/g12a-smoke-contract",
            pull["head"]["sha"].as_str().unwrap(),
        ))
        .unwrap();
        assert_eq!(
            result,
            MergeResult::Merged {
                merge_commit: "a6cdb4c64b244ca86a23de0a2139a4eee29e5d54".into()
            }
        );
        let commands = repository.api.runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 2);
        assert!(commands.iter().all(|s| s.contains("'--method' 'GET'")));
    }
    #[test]
    fn a_stale_approval_reports_actual_head_without_a_put() {
        let (pull, _) = recorded();
        let repository = repository(vec![response(&pull)]);
        let result =
            agend_testkit::block_on(repository.merge(154, "test/g12a-smoke-contract", "1234567"))
                .unwrap();
        assert_eq!(
            result,
            MergeResult::HeadChanged {
                actual_head: pull["head"]["sha"].as_str().unwrap().into()
            }
        );
        assert_eq!(repository.api.runner.commands.lock().unwrap().len(), 1);
    }
    #[test]
    fn a_lost_put_response_is_reconciled_from_the_same_native_receipt() {
        let (pull, commit) = recorded();
        let mut pending = pull.clone();
        pending["merged"] = Value::Bool(false);
        pending["state"] = Value::String("open".into());
        pending["merge_commit_sha"] = Value::Null;
        let repository = repository(vec![
            response(&pending),
            response(&super::super::protection::tests::protected()),
            vec![],
            response(&pull),
            response(&commit),
        ]);
        let result = agend_testkit::block_on(repository.merge(
            154,
            "test/g12a-smoke-contract",
            pull["head"]["sha"].as_str().unwrap(),
        ))
        .unwrap();
        assert!(matches!(result, MergeResult::Merged { .. }));
        let commands = repository.api.runner.commands.lock().unwrap();
        assert_eq!(
            commands
                .iter()
                .filter(|s| s.contains("'--method' 'PUT'"))
                .count(),
            1
        );
        assert!(commands[2].contains(&format!("'sha={}'", pull["head"]["sha"].as_str().unwrap())));
    }
    #[test]
    fn owned_merge_refuses_a_recreated_repository_before_writing() {
        use agend_core::github::{GithubChange, GithubIdentity};
        let (mut pull, _) = recorded();
        pull["body"] = serde_json::json!("<!-- agend:t-1:nonce -->");
        let id = pull["head"]["repo"]["id"].as_u64().unwrap();
        let change = GithubChange {
            identity: GithubIdentity {
                task_id: "t-1".into(),
                local_repo: "/repo".into(),
                repository: "suzuke/AgEnD".into(),
                repository_id: id,
                base: "v2".into(),
                branch: "test/g12a-smoke-contract".into(),
                nonce: "nonce".into(),
            },
            pull_number: Some(154),
            pushed_head: Some(pull["head"]["sha"].as_str().unwrap().into()),
            push_intent: None,
            create_attempted: true,
            merge_head: None,
            cleanup: Default::default(),
        };
        let mut repo: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/github/repository.json"
        ))
        .unwrap();
        repo["id"] = serde_json::json!(id + 1);
        let repository = repository(vec![response(&repo)]);
        assert!(
            agend_testkit::block_on(
                repository.merge_owned(&change, change.pushed_head.as_deref().unwrap())
            )
            .is_err()
        );
        assert_eq!(repository.api.runner.commands.lock().unwrap().len(), 1);
    }

    #[test]
    fn an_owned_merge_cannot_accept_a_foreign_receipt_after_the_put() {
        use agend_core::github::{GithubChange, GithubIdentity};
        let (mut pull, _) = recorded();
        pull["body"] = serde_json::json!("<!-- agend:t-1:nonce -->");
        let change = GithubChange {
            identity: GithubIdentity {
                task_id: "t-1".into(),
                local_repo: "/repo".into(),
                repository: "suzuke/AgEnD".into(),
                repository_id: pull["head"]["repo"]["id"].as_u64().unwrap(),
                base: "v2".into(),
                branch: "test/g12a-smoke-contract".into(),
                nonce: "nonce".into(),
            },
            pull_number: Some(154),
            pushed_head: Some(pull["head"]["sha"].as_str().unwrap().into()),
            push_intent: None,
            create_attempted: true,
            merge_head: None,
            cleanup: Default::default(),
        };
        let repo: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/github/repository.json"
        ))
        .unwrap();
        let mut pending = pull.clone();
        pending["merged"] = Value::Bool(false);
        pending["state"] = serde_json::json!("open");
        pending["merge_commit_sha"] = Value::Null;
        pull["body"] = serde_json::json!("foreign receipt");
        let repository = repository(vec![
            response(&repo),
            response(&pending),
            response(&super::super::protection::tests::protected()),
            vec![],
            response(&pull),
        ]);
        assert!(
            agend_testkit::block_on(
                repository.merge_owned(&change, change.pushed_head.as_deref().unwrap())
            )
            .is_err()
        );
        assert_eq!(
            repository
                .api
                .runner
                .commands
                .lock()
                .unwrap()
                .iter()
                .filter(|s| s.contains("'--method' 'PUT'"))
                .count(),
            1
        );
    }
}
