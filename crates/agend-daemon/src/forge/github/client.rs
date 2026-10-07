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

    async fn receipt(&self, pull: &Pull, approved: &str) -> Result<MergeResult, ExecutionError> {
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
        let before = self.pull(number, branch).await?;
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
        let endpoint = self.endpoint(&format!("pulls/{number}/merge"))?;
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
        assert!(commands[1].contains(&format!("'sha={}'", pull["head"]["sha"].as_str().unwrap())));
    }
}
