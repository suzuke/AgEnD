//! IO composition for the pipeline; the queue itself uses only core ports.
use super::pipeline::{self, Handle};
use crate::{driver::codex::CodexDriver, fleet::Fleet, git::Git, store::SqliteStore};
use agend_core::pipeline::ports::*;
use agend_core::pipeline::task::{Task, TaskStatus};
use agend_core::runtime_records::*;
use agend_core::traits::CommandOutput;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) struct LocalExecutor {
    home: PathBuf,
    exe: PathBuf,
    git: Option<Git>,
    store: Arc<SqliteStore>,
}
pub async fn start(
    home: &Path,
    exe: &Path,
    store: Arc<SqliteStore>,
    fleet: Arc<Fleet>,
    codex: CodexDriver,
) -> Result<(Handle, tokio::task::JoinHandle<()>), String> {
    let executor = LocalExecutor {
        home: home.canonicalize().map_err(|e| e.to_string())?,
        exe: exe.into(),
        store: store.clone(),
        git: Git::checked(home)
            .await
            .map_err(|e| crate::log::line(&format!("repo execution unavailable: {e}")))
            .ok(),
    };
    pipeline::start_with(
        &executor.home.clone(),
        store,
        fleet,
        codex,
        executor,
        pipeline::transition::WallClock,
    )
    .await
}
impl PipelineExecutor for LocalExecutor {
    type Forge = crate::forge::local::LocalForge;
    fn git_available(&self) -> bool {
        self.git.is_some()
    }
    fn new_id(&self) -> Result<String, String> {
        crate::store::instances::new_session_id().map_err(|e| e.to_string())
    }
    fn canonical_repo(&self, repo: &str) -> Result<String, String> {
        Path::new(repo)
            .canonicalize()
            .map(|p| p.display().to_string())
            .map_err(|e| e.to_string())
    }
    fn forge(&self, repo: &str, expected_main: Option<String>) -> Self::Forge {
        crate::forge::local::LocalForge {
            repo: repo.into(),
            git: self.git.clone().expect("git availability checked"),
            store: self.store.clone(),
            expected_main,
        }
    }
    async fn run(&self, repo: &str, args: &[&str]) -> Result<String, String> {
        self.git
            .as_ref()
            .ok_or("git unavailable")?
            .run(Path::new(repo), args)
            .await
    }
    async fn ancestor(&self, repo: &str, a: &str, b: &str) -> Result<bool, String> {
        self.git
            .as_ref()
            .ok_or("git unavailable")?
            .ancestor(Path::new(repo), a, b)
            .await
    }
    async fn patch_id(&self, repo: &str, head: &str) -> Result<String, String> {
        self.git
            .as_ref()
            .ok_or("git unavailable")?
            .patch_id(Path::new(repo), head)
            .await
    }
    async fn find_merge(
        &self,
        repo: &str,
        task: &str,
        head: &str,
    ) -> Result<Option<(String, bool)>, String> {
        self.forge(repo, None).find_merge(task, head).await
    }
    async fn readiness(&self) -> Result<(), String> {
        crate::checks::readiness(&self.home).await
    }
    async fn ensure(&self, repo: &str, b: &BindingRow) -> Result<(), String> {
        crate::bindings::ensure(
            self.store.as_ref(),
            self.git.as_ref().ok_or("git unavailable")?,
            &self.home,
            &self.exe,
            Path::new(repo),
            b,
        )
        .await
    }
    async fn release(
        &self,
        repo: &str,
        b: &BindingRow,
        merged: bool,
    ) -> Result<Option<String>, String> {
        crate::bindings::release(
            self.store.as_ref(),
            self.git.as_ref().ok_or("git unavailable")?,
            &self.home,
            &self.exe,
            Path::new(repo),
            b,
            merged,
        )
        .await
    }
    async fn check(
        &self,
        repo: &str,
        ticket: &str,
        head: &str,
        command: &str,
        timeout: u64,
    ) -> Result<CommandOutput, ExecutionError> {
        crate::checks::run(
            self.git
                .as_ref()
                .ok_or_else(|| ExecutionError::Failed("git unavailable".into()))?,
            &self.home,
            Path::new(repo),
            ticket,
            head,
            command,
            timeout,
        )
        .await
    }
    async fn projections(
        &self,
        members: &[Member],
        teams: &[Team],
        bindings: &[BindingRow],
    ) -> Result<(), String> {
        for member in members {
            if bindings.iter().any(|b| b.instance == member.id) {
                continue;
            }
            let repo = teams
                .iter()
                .find(|t| t.id == member.team)
                .and_then(|t| t.repo.clone());
            if let Err(reason) = crate::bindings::snapshot(&self.home, &member.id, repo, None) {
                crate::log::line(&format!("{}: projection failed: {reason}", member.id));
            }
        }
        let dir = self.home.join("bindings");
        if let Ok(files) = std::fs::read_dir(&dir) {
            for file in files.flatten() {
                let path = file.path();
                if path.extension().is_some_and(|e| e == "json")
                    && path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| !members.iter().any(|m| m.id == s))
                {
                    std::fs::remove_file(path).map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    }
    async fn orphans(
        &self,
        tasks: &[Task],
        teams: &[Team],
        bindings: &[BindingRow],
        running: &[String],
    ) -> Result<(), String> {
        let Some(git) = self.git.clone() else {
            return Ok(());
        };
        let repos = teams
            .iter()
            .filter_map(|t| t.repo.clone())
            .collect::<BTreeSet<_>>();
        for repo in repos {
            if let Err(reason) = self
                .orphan_repo(&git, Path::new(&repo), tasks, bindings, running)
                .await
            {
                crate::log::line(&format!("repo {repo}: orphan cleanup deferred: {reason}"));
            }
        }
        let checks = self.home.join("checks");
        if let Ok(entries) = std::fs::read_dir(&checks) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name
                    .to_str()
                    .is_some_and(|n| n.starts_with("t-") && n.ends_with(".tmp"))
                    && !name.to_str().is_some_and(|n| {
                        running
                            .iter()
                            .any(|ticket| n.starts_with(&format!("{}-", ticket.replace('/', "-"))))
                    })
                {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
        Ok(())
    }
}

impl LocalExecutor {
    async fn orphan_repo(
        &self,
        git: &Git,
        repo: &Path,
        tasks: &[Task],
        bindings: &[BindingRow],
        running: &[String],
    ) -> Result<(), String> {
        let list = git
            .run(repo, &["worktree", "list", "--porcelain"])
            .await
            .map_err(|e| e.to_string())?;
        for block in list.split("\n\n") {
            let Some(path) = block.lines().find_map(|l| l.strip_prefix("worktree ")) else {
                continue;
            };
            let wt = Path::new(path);
            let checks = wt.parent() == Some(self.home.join("checks").as_path());
            let managed = wt.parent() == Some(self.home.join("worktrees").as_path())
                && wt.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    let id = n.strip_suffix("-review").unwrap_or(n);
                    tasks.iter().any(|t| t.id == id)
                        || id.strip_prefix("t-").is_some_and(|id| {
                            !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
                        })
                });
            let active = bindings.iter().any(|b| b.worktree == path);
            let running = wt.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                running
                    .iter()
                    .any(|ticket| n.starts_with(&format!("{}-", ticket.replace('/', "-"))))
            });
            if (checks && !running) || (managed && !active) {
                let branch = block
                    .lines()
                    .find_map(|l| l.strip_prefix("branch refs/heads/"));
                let task = branch
                    .and_then(agend_core::model::task_id_of_branch)
                    .or_else(|| {
                        wt.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.strip_suffix("-review").unwrap_or(n))
                    })
                    .unwrap_or("orphan");
                crate::bindings::archive(git, &self.home, repo, wt, task, branch, false)
                    .await
                    .map_err(|e| e.to_string())?;
                git.run(repo, &["worktree", "remove", "--force", path])
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        let branches = git
            .run(
                repo,
                &[
                    "for-each-ref",
                    "--format=%(refname:short)",
                    "refs/heads/agend/",
                ],
            )
            .await
            .map_err(|e| e.to_string())?;
        for branch in branches.lines() {
            let Some(id) = agend_core::model::task_id_of_branch(branch) else {
                continue;
            };
            let task = tasks.iter().find(|t| t.id == id);
            if task.is_none()
                || task.is_some_and(|t| {
                    matches!(
                        t.status,
                        TaskStatus::Done
                            | TaskStatus::Failed
                            | TaskStatus::Cancelled
                            | TaskStatus::Superseded
                    )
                })
            {
                crate::bindings::archive(
                    git,
                    &self.home,
                    repo,
                    Path::new("/nonexistent-agend-worktree"),
                    id,
                    Some(branch),
                    task.is_some_and(|t| t.status == TaskStatus::Done),
                )
                .await
                .map_err(|e| e.to_string())?;
                git.run(repo, &["branch", "-D", branch])
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}
