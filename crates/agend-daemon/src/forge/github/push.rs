//! Push an owned branch with an explicit lease and a durable pre-write intent.
use crate::git::Git;
use agend_core::{
    github::{GithubStore, VersionedGithubChange},
    traits::Runner,
};
use std::path::Path;

pub async fn push_owned<R: Runner, S: GithubStore>(
    git: &Git<R>,
    store: &S,
    record: VersionedGithubChange,
    head: &str,
) -> Result<VersionedGithubChange, String>
where
    R::Error: std::fmt::Display,
    S::Error: std::fmt::Display,
{
    let repo = Path::new(&record.change.identity.local_repo);
    let origin = git.run(repo, &["remote", "get-url", "origin"]).await?;
    if super::api::repository_from_origin(&origin)? != record.change.identity.repository {
        return Err("GitHub origin differs from durable ownership".into());
    }
    push_to(git, store, record, head, &origin).await
}

pub(super) async fn remote_head<R: Runner>(
    git: &Git<R>,
    repo: &Path,
    origin: &str,
    branch: &str,
) -> Result<Option<String>, String>
where
    R::Error: std::fmt::Display,
{
    let reference = format!("refs/heads/{branch}");
    let output = git
        .output(repo, &["ls-remote", "--refs", "--", origin, &reference])
        .await?;
    if output.timed_out || output.exit_code != Some(0) || output.stdout.len() > 4096 {
        return Err("cannot determine owned remote branch head".into());
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| "invalid remote ref response")?;
    if text.is_empty() {
        return Ok(None);
    }
    let mut lines = text.lines();
    let (sha, actual_ref) = lines
        .next()
        .and_then(|line| line.split_once('\t'))
        .ok_or("invalid remote ref response")?;
    if lines.next().is_some() || actual_ref != reference || !super::pull::full_sha(sha) {
        return Err("remote ref response differs from requested branch".into());
    }
    Ok(Some(sha.into()))
}

async fn save<S: GithubStore>(store: &S, record: &mut VersionedGithubChange) -> Result<(), String>
where
    S::Error: std::fmt::Display,
{
    if !store
        .save_github_change(Some(record.revision), &record.change)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err("GitHub push ownership revision changed".into());
    }
    record.revision += 1;
    Ok(())
}

async fn push_to<R: Runner, S: GithubStore>(
    git: &Git<R>,
    store: &S,
    mut record: VersionedGithubChange,
    head: &str,
    origin: &str,
) -> Result<VersionedGithubChange, String>
where
    R::Error: std::fmt::Display,
    S::Error: std::fmt::Display,
{
    if record.change.cleanup != Default::default() || record.change.merge_head.is_some() {
        return Err("GitHub merge or cleanup has begun; no new push permitted".into());
    }
    let repo_path = record.change.identity.local_repo.clone();
    let repo = Path::new(&repo_path);
    let branch = record.change.identity.branch.clone();
    let reference = format!("refs/heads/{branch}");
    git.run(repo, &["check-ref-format", &reference]).await?;
    if !super::pull::full_sha(head)
        || git
            .run(
                repo,
                &["rev-parse", "--verify", &format!("{head}^{{commit}}")],
            )
            .await?
            != head
    {
        return Err("push requires a complete existing commit id".into());
    }
    let remote = remote_head(git, repo, origin, &branch).await?;
    if let Some(intent) = &record.change.push_intent {
        if remote.as_ref() != Some(intent) {
            return Err(
                "GitHub push outcome unknown or remote head changed; original attempt retained"
                    .into(),
            );
        }
        record.change.pushed_head = record.change.push_intent.take();
        save(store, &mut record).await?;
        if record.change.pushed_head.as_deref() != Some(head) {
            return Err("previous push reconciled; resubmit current head separately".into());
        }
        return Ok(record);
    }
    if remote != record.change.pushed_head {
        return Err("remote branch differs from owned head; refusing to overwrite it".into());
    }
    if remote.as_deref() == Some(head) {
        return Ok(record);
    }
    record.change.push_intent = Some(head.into());
    save(store, &mut record).await?;
    let lease = format!(
        "--force-with-lease={reference}:{}",
        record.change.pushed_head.as_deref().unwrap_or("")
    );
    let refspec = format!("{head}:{reference}");
    // No automatic retry, including timeout/nonzero exit. The exact remote
    // reference decides whether this persisted intent completed.
    let _reply = git
        .output(
            repo,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--no-mirror",
                &lease,
                "--",
                origin,
                &refspec,
            ],
        )
        .await;
    if remote_head(git, repo, origin, &branch).await?.as_deref() != Some(head) {
        return Err("GitHub push not confirmed; original attempt retained".into());
    }
    record.change.pushed_head = record.change.push_intent.take();
    save(store, &mut record).await?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{runner::ProcessRunner, store::SqliteStore};
    use agend_core::{
        github::{GithubChange, GithubIdentity},
        pipeline::task::Task,
        traits::{CommandOutput, Store},
    };
    use agend_testkit::tempdir::TempDir;
    use std::{path::PathBuf, sync::Mutex};

    struct Faults {
        runner: ProcessRunner,
        git: PathBuf,
        race: Mutex<Option<(PathBuf, String)>>,
        pushes: Mutex<usize>,
    }
    impl Runner for Faults {
        type Error = String;
        async fn run(
            &self,
            command: &str,
            directory: &str,
            timeout: u64,
        ) -> Result<CommandOutput, String> {
            let push = command.contains("'push'");
            if push {
                *self.pushes.lock().unwrap() += 1;
                let race = self.race.lock().unwrap().take();
                if let Some((repo, foreign)) = race {
                    let git = Git {
                        executable: self.git.clone(),
                        runner: self.runner.clone(),
                    };
                    git.run(&repo, &["update-ref", "refs/heads/agend/task", &foreign])
                        .await?;
                }
            }
            let output = self
                .runner
                .run(command, directory, timeout)
                .await
                .map_err(|e| e.to_string())?;
            if push {
                Err("injected lost push reply".into())
            } else {
                Ok(output)
            }
        }
    }
    #[test]
    fn real_git_lost_reply_recovers_and_explicit_lease_rejects_a_racing_writer() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let dir = TempDir::new("github-push").unwrap();
                let local = dir.path().join("repo");
                std::fs::create_dir(&local).unwrap();
                let bare = dir.path().join("remote");
                let plain = Git::discover(dir.path()).unwrap();
                plain.run(&local, &["init", "-b", "main"]).await.unwrap();
                plain
                    .run(&local, &["config", "user.name", "Agend fixture"])
                    .await
                    .unwrap();
                plain
                    .run(&local, &["config", "user.email", "fixture@example.invalid"])
                    .await
                    .unwrap();
                plain
                    .run(&local, &["commit", "--allow-empty", "-m", "first"])
                    .await
                    .unwrap();
                let first = plain.run(&local, &["rev-parse", "HEAD"]).await.unwrap();
                plain
                    .run(dir.path(), &["init", "--bare", bare.to_str().unwrap()])
                    .await
                    .unwrap();
                let store = SqliteStore::open(&dir.path().join("home"), 0).unwrap();
                store
                    .create_task(&Task::new("t-1", "test", "general", "code", 1))
                    .await
                    .unwrap();
                let change = GithubChange {
                    identity: GithubIdentity {
                        task_id: "t-1".into(),
                        local_repo: local.to_str().unwrap().into(),
                        repository: "fixture/repo".into(),
                        repository_id: 42,
                        base: "main".into(),
                        branch: "agend/task".into(),
                        nonce: "native-git-fixture".into(),
                    },
                    pull_number: None,
                    pushed_head: None,
                    push_intent: None,
                    create_attempted: false,
                    merge_head: None,
                    cleanup: Default::default(),
                };
                store.save_github_change(None, &change).await.unwrap();
                let git = Git {
                    executable: plain.executable.clone(),
                    runner: Faults {
                        runner: plain.runner.clone(),
                        git: plain.executable.clone(),
                        race: Mutex::new(None),
                        pushes: Mutex::new(0),
                    },
                };
                let record = store.github_change("t-1").await.unwrap().unwrap();
                let record = push_to(&git, &store, record, &first, bare.to_str().unwrap())
                    .await
                    .unwrap();
                assert_eq!(record.change.pushed_head.as_deref(), Some(first.as_str()));
                assert_eq!(*git.runner.pushes.lock().unwrap(), 1);
                plain
                    .run(&local, &["commit", "--allow-empty", "-m", "second"])
                    .await
                    .unwrap();
                let second = plain.run(&local, &["rev-parse", "HEAD"]).await.unwrap();
                plain
                    .run(
                        &local,
                        &[
                            "push",
                            bare.to_str().unwrap(),
                            &format!("{second}:refs/heads/foreign"),
                        ],
                    )
                    .await
                    .unwrap();
                plain
                    .run(&local, &["commit", "--allow-empty", "-m", "third"])
                    .await
                    .unwrap();
                let third = plain.run(&local, &["rev-parse", "HEAD"]).await.unwrap();
                *git.runner.race.lock().unwrap() = Some((bare.clone(), second.clone()));
                assert!(
                    push_to(&git, &store, record, &third, bare.to_str().unwrap())
                        .await
                        .is_err()
                );
                assert_eq!(
                    remote_head(&plain, &local, bare.to_str().unwrap(), "agend/task")
                        .await
                        .unwrap(),
                    Some(second)
                );
                assert_eq!(*git.runner.pushes.lock().unwrap(), 2);
                drop(store);
                let store = SqliteStore::open(&dir.path().join("home"), 1).unwrap();
                let record = store.github_change("t-1").await.unwrap().unwrap();
                assert_eq!(record.change.push_intent.as_deref(), Some(third.as_str()));
                assert!(
                    push_to(&git, &store, record, &third, bare.to_str().unwrap())
                        .await
                        .is_err()
                );
                assert_eq!(
                    *git.runner.pushes.lock().unwrap(),
                    2,
                    "unknown push was replayed"
                );
            });
    }
}
