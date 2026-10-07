//! No-IO executor of the same pipeline ports, backed by contract fakes.
use super::{FakeForge, FakeRunner, FakeStore, lock};
use agend_core::{
    pipeline::{ports::*, task::Task},
    runtime_records::*,
    traits::*,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone)]
pub struct FakePipelineExecutor {
    pub store: Arc<FakeStore>,
    pub forge: Arc<FakeForge>,
    pub runner: Arc<FakeRunner>,
    effects: Arc<Mutex<Vec<String>>>,
    projections: Arc<Mutex<BTreeMap<String, Option<BindingRow>>>>,
    ids: Arc<AtomicU64>,
    remote_cleanup_failure: Arc<Mutex<Option<String>>>,
}
impl FakePipelineExecutor {
    pub fn new(store: Arc<FakeStore>) -> Self {
        Self {
            store,
            forge: Arc::new(FakeForge::new()),
            runner: Arc::new(FakeRunner::new()),
            effects: Arc::default(),
            projections: Arc::default(),
            ids: Arc::default(),
            remote_cleanup_failure: Arc::default(),
        }
    }
    pub fn set_remote_cleanup_failure(&self, reason: Option<String>) {
        *lock(&self.remote_cleanup_failure) = reason;
    }
    pub fn effects(&self) -> Vec<String> {
        lock(&self.effects).clone()
    }
    pub fn projection(&self, id: &str) -> Option<BindingRow> {
        lock(&self.projections).get(id).cloned().flatten()
    }
}
pub struct PipelineFakeForge(Arc<FakeForge>);
impl Forge for PipelineFakeForge {
    type Error = ExecutionError;
    async fn submit(&self, c: &Submission) -> Result<SubmittedChange, ExecutionError> {
        self.0
            .submit(c)
            .await
            .map_err(|e| ExecutionError::Failed(e.to_string()))
    }
    async fn head(&self, b: &str) -> Result<String, ExecutionError> {
        self.0
            .head(b)
            .await
            .map_err(|e| ExecutionError::Failed(e.to_string()))
    }
    async fn merge_if_head_is(&self, r: &MergeRequest) -> Result<MergeResult, ExecutionError> {
        self.0
            .merge_if_head_is(r)
            .await
            .map_err(|e| ExecutionError::Failed(e.to_string()))
    }
}
impl PipelineExecutor for FakePipelineExecutor {
    type Forge = PipelineFakeForge;
    fn git_available(&self) -> bool {
        true
    }
    fn new_id(&self) -> Result<String, String> {
        Ok(format!(
            "fake-event-{}",
            self.ids.fetch_add(1, Ordering::SeqCst)
        ))
    }
    fn canonical_repo(&self, repo: &str) -> Result<String, String> {
        Ok(repo.into())
    }
    fn forge(&self, _repo: &str, kind: &str, _expected: Option<String>) -> Self::Forge {
        lock(&self.effects).push(format!("forge:{kind}"));
        PipelineFakeForge(self.forge.clone())
    }
    async fn prepare_main(&self, repo: &str, kind: &str) -> Result<String, String> {
        lock(&self.effects).push(format!("prepare-main:{kind}"));
        self.run(repo, &["rev-parse", "main"]).await
    }
    async fn run(&self, repo: &str, args: &[&str]) -> Result<String, String> {
        lock(&self.effects).push(format!("git:{repo}:{}", args.join(" ")));
        if args.first() == Some(&"rev-parse") {
            let name = args
                .last()
                .ok_or("missing ref")?
                .strip_prefix("refs/heads/")
                .unwrap_or(args.last().unwrap());
            return if name == "main" {
                Ok(self.forge.base_head())
            } else {
                self.forge.head(name).await.map_err(|e| e.to_string())
            };
        }
        let out = self
            .runner
            .run(&args.join(" "), repo, 60_000)
            .await
            .map_err(|e| e.to_string())?;
        if out.exit_code != Some(0) {
            return Err(String::from_utf8_lossy(&out.stderr).into());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().into())
    }
    async fn clean_worktree(&self, repo: &str) -> Result<bool, String> {
        let flags = self.run(repo, &["ls-files", "-v", "-z"]).await?;
        if !flags.is_empty() && !flags.ends_with('\0') {
            return Err("cannot inspect worktree index flags".into());
        }
        if flags.as_bytes().split(|b| *b == 0).any(|entry| {
            entry
                .first()
                .is_some_and(|tag| *tag == b'S' || tag.is_ascii_lowercase())
        }) {
            return Ok(false);
        }
        Ok(self.run(repo, &["status", "--porcelain"]).await?.is_empty())
    }
    async fn ancestor(&self, _repo: &str, a: &str, b: &str) -> Result<bool, String> {
        let base = self.forge.base_head();
        Ok(self.forge.is_ancestor(
            if a == "main" { &base } else { a },
            if b == "main" { &base } else { b },
        ))
    }
    async fn patch_id(&self, _repo: &str, head: &str) -> Result<String, String> {
        Ok(format!("patch:{head}"))
    }
    async fn find_merge(
        &self,
        _repo: &str,
        kind: &str,
        _task: &str,
        head: &str,
    ) -> Result<Option<(String, bool)>, String> {
        lock(&self.effects).push(format!("find-merge:{kind}"));
        Ok(self
            .forge
            .base_contains(head)
            .then(|| (self.forge.base_head(), true)))
    }
    async fn cleanup_remote(&self, _repo: &str, task: &str, merged: bool) -> Result<(), String> {
        lock(&self.effects).push(format!("cleanup-remote:{task}:{merged}"));
        match lock(&self.remote_cleanup_failure).clone() {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
    async fn readiness(&self) -> Result<(), String> {
        lock(&self.effects).push("readiness".into());
        Ok(())
    }
    async fn ensure(&self, _repo: &str, b: &BindingRow) -> Result<(), String> {
        lock(&self.effects).push(format!("ensure:{}:{}", b.task, b.instance));
        if let Some(branch) = &b.branch {
            self.forge.create_branch(branch);
        }
        let mut ready = b.clone();
        ready.status = "ready".into();
        self.store
            .put_binding(&ready)
            .await
            .map_err(|e| e.to_string())?;
        lock(&self.projections).insert(b.instance.clone(), Some(ready));
        Ok(())
    }
    async fn release(
        &self,
        _repo: &str,
        b: &BindingRow,
        _mode: agend_core::pipeline::ports::BindingRelease,
    ) -> Result<Option<String>, String> {
        lock(&self.effects).push(format!("release:{}:{}", b.task, b.instance));
        lock(&self.projections).insert(b.instance.clone(), None);
        self.store
            .delete_binding(&b.instance)
            .await
            .map_err(|e| e.to_string())?;
        Ok(None)
    }
    async fn projections(
        &self,
        members: &[Member],
        _teams: &[Team],
        bindings: &[BindingRow],
    ) -> Result<(), String> {
        let mut p = lock(&self.projections);
        p.retain(|id, _| members.iter().any(|m| m.id == *id));
        for m in members {
            p.insert(
                m.id.clone(),
                bindings.iter().find(|b| b.instance == m.id).cloned(),
            );
        }
        Ok(())
    }
    async fn orphans(
        &self,
        _tasks: &[Task],
        _teams: &[Team],
        bindings: &[BindingRow],
        _running: &[String],
    ) -> Result<(), String> {
        lock(&self.effects).push("orphans".into());
        lock(&self.projections).values_mut().for_each(|p| {
            if p.as_ref().is_some_and(|b| !bindings.contains(b)) {
                *p = None;
            }
        });
        Ok(())
    }
    async fn check(
        &self,
        repo: &str,
        ticket: &str,
        head: &str,
        command: &str,
        timeout: u64,
    ) -> Result<CommandOutput, ExecutionError> {
        lock(&self.effects).push(format!("check:{ticket}:{head}"));
        self.runner
            .run(command, repo, timeout)
            .await
            .map_err(|e| ExecutionError::Failed(e.to_string()))
    }
}
