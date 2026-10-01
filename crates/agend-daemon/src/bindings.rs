//! Durable binding creation, shim projections and WIP-preserving release.
use crate::git::Git;
use crate::runner::{ProcessRunner, quote};
use agend_core::binding::{Binding, SNAPSHOT_VERSION, Snapshot};
use agend_core::pipeline::ports::{BindingRelease, PipelineStore};
use agend_core::runtime_records::BindingRow;
use agend_core::traits::Runner;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
mod archive;
pub use archive::archive;

pub fn snapshot(
    home: &Path,
    instance: &str,
    repo: Option<String>,
    binding: Option<Binding>,
) -> Result<(), String> {
    crate::store::instances::validate_id(instance)?;
    let dir = home.join("bindings");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{instance}.json"));
    let tmp = dir.join(format!(".{instance}.tmp"));
    let snap = Snapshot {
        version: SNAPSHOT_VERSION,
        instance: instance.into(),
        source_repo: repo,
        protected_refs: Vec::new(),
        binding,
    };
    let bytes = serde_json::to_vec(&snap).map_err(|e| e.to_string())?;
    // unlink any interrupted projection; never follow an agent-supplied symlink.
    let _ = std::fs::remove_file(&tmp);
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o444))
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

async fn hooks(
    git: &Git,
    home: &Path,
    exe: &Path,
    operation: &str,
    wt: &Path,
) -> Result<(), String> {
    let mut runner = ProcessRunner {
        env: git.runner.env.clone(),
    };
    runner
        .env
        .insert("AGEND_HOME".into(), home.display().to_string());
    let command = format!(
        "{} hooks {} {}",
        quote(&exe.to_string_lossy()),
        operation,
        quote(&wt.to_string_lossy())
    );
    let out = runner
        .run(&command, &home.to_string_lossy(), 60_000)
        .await
        .map_err(|e| e.to_string())?;
    if out.exit_code == Some(0) {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into())
    }
}

pub async fn ensure<S: PipelineStore>(
    store: &S,
    git: &Git,
    home: &Path,
    exe: &Path,
    repo: &Path,
    b: &BindingRow,
) -> Result<(), String>
where
    S::Error: std::fmt::Display,
{
    let wt = PathBuf::from(&b.worktree);
    if !wt.starts_with(home.join("worktrees")) {
        return Err("binding worktree escapes managed directory".into());
    }
    std::fs::create_dir_all(home.join("worktrees")).map_err(|e| e.to_string())?;
    if !wt.exists() {
        if b.kind == "work" {
            let branch = b.branch.as_deref().ok_or("work binding has no branch")?;
            if agend_core::model::task_id_of_branch(branch) != Some(b.task.as_str()) {
                return Err("invalid managed branch".into());
            }
            let exists = git
                .run(
                    repo,
                    &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
                )
                .await
                .is_ok();
            if b.status == "ready" && !exists {
                return Err(format!("bound branch {branch} is missing"));
            }
            if exists {
                git.run(repo, &["worktree", "prune"]).await?;
                git.run(repo, &["worktree", "add", &b.worktree, branch])
                    .await?;
            } else {
                let main = git.run(repo, &["rev-parse", "main"]).await?;
                git.run(repo, &["worktree", "add", "-b", branch, &b.worktree, &main])
                    .await?;
            }
        } else {
            let head = b.head.as_deref().ok_or("review binding has no head")?;
            git.run(repo, &["worktree", "prune"]).await?;
            git.run(repo, &["worktree", "add", "--detach", &b.worktree, head])
                .await?;
        }
    }
    if b.kind == "work" {
        git.run(&wt, &["rev-parse", "--verify", "HEAD"]).await?;
        let actual = git.run(&wt, &["symbolic-ref", "--short", "HEAD"]).await?;
        if Some(actual.as_str()) != b.branch.as_deref() {
            return Err("existing worktree is on a different branch".into());
        }
    } else {
        let actual = git.run(&wt, &["rev-parse", "HEAD"]).await?;
        if Some(actual.as_str()) != b.head.as_deref()
            || git.run(&wt, &["symbolic-ref", "HEAD"]).await.is_ok()
        {
            return Err("review worktree is not detached at the requested head".into());
        }
    }
    hooks(git, home, exe, "install", &wt).await?;
    let binding = if b.kind == "work" {
        Binding::Work {
            task_id: b.task.clone(),
            branch: b.branch.clone().ok_or("missing branch")?,
            worktree: b.worktree.clone(),
        }
    } else {
        Binding::Review {
            task_id: b.task.clone(),
            head: b.head.clone().ok_or("missing head")?,
            worktree: b.worktree.clone(),
        }
    };
    snapshot(
        home,
        &b.instance,
        Some(repo.display().to_string()),
        Some(binding),
    )?;
    let mut ready = b.clone();
    ready.status = "ready".into();
    store.put_binding(&ready).await.map_err(|e| e.to_string())
}

pub async fn release<S: PipelineStore>(
    store: &S,
    git: &Git,
    home: &Path,
    exe: &Path,
    repo: &Path,
    b: &BindingRow,
    mode: BindingRelease,
) -> Result<Option<String>, String>
where
    S::Error: std::fmt::Display,
{
    snapshot(home, &b.instance, Some(repo.display().to_string()), None)?;
    let wt = Path::new(&b.worktree);
    let patch = archive(
        git,
        home,
        repo,
        wt,
        &b.task,
        if b.kind == "work" {
            b.branch.as_deref()
        } else {
            None
        },
        mode == BindingRelease::Merged,
    )
    .await?;
    if wt.exists() {
        hooks(git, home, exe, "uninstall", wt).await?;
        git.run(repo, &["worktree", "remove", "--force", &b.worktree])
            .await?;
    }
    if mode != BindingRelease::Handoff
        && let Some(branch) = b.branch.as_deref()
        && agend_core::model::task_id_of_branch(branch) == Some(b.task.as_str())
        && git
            .run(
                repo,
                &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
            )
            .await
            .is_ok()
    {
        git.run(repo, &["branch", "-D", branch]).await?;
    }
    store
        .delete_binding(&b.instance)
        .await
        .map_err(|e| e.to_string())?;
    Ok(patch)
}
