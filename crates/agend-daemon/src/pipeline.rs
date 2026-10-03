//! One serialized pipeline writer. Results return through the same queue.
mod attention;
mod commands;
mod context;
mod lifecycle;
mod reconcile;
mod terminal;
pub(crate) mod transition;

use crate::log;
use agend_core::pipeline::ports::*;
use agend_core::pipeline::state::{
    PipelineAction, PipelineEvent, PipelineSnapshot, PipelineState, PipelineStatus,
    outstanding_actions,
};
use agend_core::pipeline::task::{Task, TaskStatus};
use agend_core::pipeline::workflow::{Approver, Stage, TimeoutAction, ValidatedWorkflow, Workflow};
use agend_core::protocol::client::*;
use agend_core::runtime_records::*;
use agend_core::traits::{CasResult, StoredEvent, TaskProgress};
use agend_core::traits::{Clock, Driver};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Semaphore, mpsc, oneshot};

pub type Refusal = (String, String);
pub type Reply = Result<CommandResult, Refusal>;
fn invalid(message: impl Into<String>) -> Refusal {
    (error_code::INVALID_REQUEST.into(), message.into())
}
fn db(error: impl std::fmt::Display) -> Refusal {
    invalid(error.to_string())
}

#[derive(Clone)]
pub struct Handle {
    tx: mpsc::UnboundedSender<Input>,
}
pub(crate) enum Input {
    Agent(Option<String>, AgentCommand, oneshot::Sender<Reply>),
    Operator(OperatorCommand, oneshot::Sender<Reply>),
    Resolve(ResolveAttentionData, oneshot::Sender<Reply>),
    Answer(AnswerAskData, oneshot::Sender<Reply>),
    Event(String, PipelineEvent),
    Check {
        task: String,
        stage: String,
        attempt: u32,
        head: Option<String>,
        result: Result<agend_core::traits::CommandOutput, ExecutionError>,
    },
    Wake,
    Daily,
}
impl Handle {
    async fn request(&self, make: impl FnOnce(oneshot::Sender<Reply>) -> Input) -> Reply {
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(make(reply))
            .map_err(|_| invalid("pipeline stopped"))?;
        answer.await.map_err(|_| invalid("pipeline stopped"))?
    }
    pub async fn agent(&self, caller: Option<String>, command: AgentCommand) -> Reply {
        self.request(|r| Input::Agent(caller, command, r)).await
    }
    pub async fn operator(&self, command: OperatorCommand) -> Reply {
        self.request(|r| Input::Operator(command, r)).await
    }
    pub async fn resolve(&self, data: ResolveAttentionData) -> Reply {
        self.request(|r| Input::Resolve(data, r)).await
    }
    pub async fn answer(&self, data: AnswerAskData) -> Reply {
        self.request(|r| Input::Answer(data, r)).await
    }
    pub fn wake(&self) {
        let _ = self.tx.send(Input::Wake);
    }
    pub fn daily(&self) {
        let _ = self.tx.send(Input::Daily);
    }
}
struct Engine<S, D, E, C, V> {
    home: PathBuf,
    store: Arc<S>,
    fleet: Arc<V>,
    codex: D,
    git: Option<E>,
    executor: E,
    clock: C,
    tx: mpsc::UnboundedSender<Input>,
    running: BTreeSet<String>,
    timers: BTreeSet<String>,
    checks: Arc<Semaphore>,
    last_day: u64,
}
struct Loaded {
    version: u64,
    task: Task,
    state: PipelineState,
    progress: Progress,
}

/// Boot reconciliation completes before the protocol socket is bound.
pub use crate::pipeline_runtime::start;
pub(crate) async fn start_with<S, D, E, C, V>(
    home: &Path,
    store: Arc<S>,
    fleet: Arc<V>,
    codex: D,
    executor: E,
    clock: C,
) -> Result<(Handle, tokio::task::JoinHandle<()>), String>
where
    S: PipelineStore + Send + 'static,
    S::Error: std::fmt::Display,
    D: Driver + Send + 'static,
    D::Error: std::fmt::Display,
    E: PipelineExecutor,
    C: Clock + Send + Sync + 'static,
    V: PipelineView,
{
    let (tx, mut queue) = mpsc::unbounded_channel();
    let mut engine = Engine {
        home: home.into(),
        store,
        fleet,
        codex,
        git: executor.git_available().then(|| executor.clone()),
        executor,
        last_day: clock.now_unix_ms() / 86_400_000,
        clock,
        tx: tx.clone(),
        running: BTreeSet::new(),
        timers: BTreeSet::new(),
        checks: Arc::new(Semaphore::new(1)),
    };
    engine.boot().await.map_err(|(_, m)| m)?;
    let handle = Handle { tx };
    let worker = tokio::spawn(async move {
        while let Some(input) = queue.recv().await {
            match input {
                Input::Agent(caller, command, reply) => {
                    let _ = reply.send(engine.agent(caller.as_deref(), command).await);
                }
                Input::Operator(command, reply) => {
                    let _ = reply.send(engine.operator(command).await);
                }
                Input::Resolve(data, reply) => {
                    let _ = reply.send(engine.resolve(data).await);
                }
                Input::Answer(data, reply) => {
                    let _ = reply.send(engine.answer(data).await);
                }
                Input::Event(task, event) => {
                    if let Err((code, message)) = engine.event(&task, event).await {
                        log::line(&format!("{task}: {code}: {message}"));
                    }
                }
                Input::Check {
                    task,
                    stage,
                    attempt,
                    head,
                    result,
                } => {
                    engine.running.remove(&format!("{task}/{stage}/{attempt}"));
                    if !engine.load(&task).await.is_ok_and(|l| {
                        l.task.status == TaskStatus::Running
                            && ticket(&l.state) == format!("{task}/{stage}/{attempt}")
                    }) {
                        log::line(&format!(
                            "{task}: discarded stale check result {stage}/{attempt}"
                        ));
                        continue;
                    }
                    match result {
                        Err(ExecutionError::Sandbox(reason)) => {
                            let _ = engine
                                .note(&task, Some(format!("sandbox-missing:{reason}")))
                                .await;
                        }
                        result => {
                            let (exit, detail) = match result {
                                Ok(o) => (
                                    o.exit_code,
                                    format!(
                                        "stdout:\n{}\nstderr:\n{}",
                                        tail(&o.stdout),
                                        tail(&o.stderr)
                                    ),
                                ),
                                Err(e) => (Some(1), format!("{e:?}")),
                            };
                            log::line(&format!(
                                "{task}: checks {task}/{stage}/{attempt} {}\n{detail}",
                                if exit == Some(0) { "passed" } else { "failed" }
                            ));
                            let _ = engine
                                .event_detail(
                                    &task,
                                    PipelineEvent::CommandFinished {
                                        stage_id: stage,
                                        attempt,
                                        head,
                                        exit_code: exit,
                                    },
                                    Some(detail),
                                )
                                .await;
                        }
                    }
                }
                Input::Wake => {
                    let _ = engine.wake().await;
                }
                Input::Daily => {
                    let day = engine.clock.now_unix_ms() / 86_400_000;
                    if day != engine.last_day {
                        engine.last_day = day;
                        if let Err((_, e)) = engine.reconcile_bindings().await {
                            log::line(&format!("reconcile: {e}"));
                        }
                    }
                }
            }
            if let Err((_, e)) = engine.refresh().await {
                log::line(&format!("pipeline view: {e}"));
            }
            if let Err((_, e)) = engine.wake().await {
                log::line(&format!("pipeline wake: {e}"));
            }
            let _ = engine.refresh().await;
        }
    });
    Ok((handle, worker))
}
fn tail(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .rev()
        .take(20)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Validate supported execution independently from team availability: missing
/// roles queue as no-role attention rather than preventing task creation.
pub fn validate(workflow: Workflow) -> Result<ValidatedWorkflow, String> {
    for stage in &workflow.stages {
        if let Stage::Approval {
            by: Approver::Human,
            count,
            ..
        } = &stage.stage
            && *count != 1
        {
            return Err(
                "human approval count must be 1; this daemon has one operator identity".into(),
            );
        }
        if matches!(stage.stage, Stage::Fanout { .. })
            || stage.on_timeout == Some(TimeoutAction::Reassign)
        {
            return Err("fanout and timeout reassign are not supported yet".into());
        }
        if matches!(stage.stage, Stage::Command { .. }) && stage.on_timeout.is_some() {
            return Err(
                "command on_timeout is not supported; runner controls command timeouts".into(),
            );
        }
        if let Stage::Submit { forge } = &stage.stage
            && forge != "local"
        {
            return Err("only forge local is supported".into());
        }
    }
    let roles = workflow
        .stages
        .iter()
        .filter_map(|s| match &s.stage {
            Stage::Work { role, .. } => Some(role.clone()),
            Stage::Approval {
                by: Approver::Role(role),
                ..
            } => Some(role.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    workflow
        .validated(&roles)
        .map_err(|e| format!("invalid workflow: {e:?}"))
}
impl<S, D, E, C, V> Engine<S, D, E, C, V>
where
    S: PipelineStore + Send + 'static,
    S::Error: std::fmt::Display,
    D: Driver + Send + 'static,
    D::Error: std::fmt::Display,
    E: PipelineExecutor,
    C: Clock + Send + Sync + 'static,
    V: PipelineView,
{
    async fn load(&self, id: &str) -> Result<Loaded, Refusal> {
        let row = self
            .store
            .load_task(id)
            .await
            .map_err(db)?
            .ok_or_else(|| invalid(format!("unknown task {id}")))?;
        let workflow = self
            .store
            .load_workflow(&row.task.workflow_id, row.task.workflow_version)
            .await
            .map_err(db)?
            .ok_or_else(|| invalid("task workflow version missing"))?;
        if workflow.id != row.task.workflow_id
            || workflow.version != row.task.workflow_version
            || workflow.requires_repo() != row.task.requires_repo
        {
            return Err(invalid(
                "workflow differs from the task's pinned definition",
            ));
        }
        let progress = self
            .store
            .progress(id)
            .await
            .map_err(db)?
            .ok_or_else(|| invalid("pipeline snapshot missing"))?;
        let snapshot: PipelineSnapshot =
            serde_json::from_str(&progress.data.pipeline).map_err(db)?;
        let state = PipelineState::restore(snapshot, validate(workflow).map_err(invalid)?)
            .map_err(invalid)?;
        if state.task_id() != id {
            return Err(invalid("snapshot task id differs from row"));
        }
        if state
            .branch()
            .is_some_and(|branch| agend_core::model::task_id_of_branch(branch) != Some(id))
        {
            return Err(invalid(
                "snapshot branch belongs to another task or namespace",
            ));
        }
        Ok(Loaded {
            version: row.version,
            task: row.task,
            state,
            progress,
        })
    }
    async fn team(&self, id: &str) -> Result<Team, Refusal> {
        self.store
            .teams()
            .await
            .map_err(db)?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| invalid(format!("unknown team {id}")))
    }
    async fn note(&self, task: &str, reason: Option<String>) -> Result<(), Refusal> {
        let p = self
            .store
            .progress(task)
            .await
            .map_err(db)?
            .ok_or_else(|| invalid("pipeline missing"))?;
        self.store
            .task_note(task, p.block_reason, reason, p.acknowledged)
            .await
            .map_err(db)
    }
    async fn event(&mut self, task: &str, event: PipelineEvent) -> Reply {
        self.event_detail(task, event, None).await
    }
    async fn event_detail(
        &mut self,
        task: &str,
        event: PipelineEvent,
        detail: Option<String>,
    ) -> Reply {
        let mut loaded = self.load(task).await?;
        if loaded.task.status == TaskStatus::Blocked
            && !matches!(event, PipelineEvent::Cancel { .. })
        {
            return Err(invalid("task is blocked; unblock it first"));
        }
        if matches!(
            loaded.task.status,
            TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled | TaskStatus::Superseded
        ) {
            return Err((
                error_code::STALE_RESULT.into(),
                "task is closed; ticket is no longer current".into(),
            ));
        }
        if self.git.is_none()
            && self.team(&loaded.task.team_id).await?.repo.is_some()
            && !matches!(event, PipelineEvent::Cancel { .. })
        {
            return Err(invalid("repo execution unavailable; waiting for git"));
        }
        // All result sources observe the actual branch before using their result.
        if !matches!(
            event,
            PipelineEvent::Start
                | PipelineEvent::Cancel { .. }
                | PipelineEvent::CommitCreated { .. }
                | PipelineEvent::MainAdvanced { .. }
                | PipelineEvent::MergeCompleted { .. }
        ) && let Some(branch) = loaded.state.branch()
            && let Some(git) = self.git.clone()
            && let Some(repo) = self.team(&loaded.task.team_id).await?.repo
        {
            let repo = repo.as_str();
            let head = git
                .run(repo, &["rev-parse", &format!("refs/heads/{branch}")])
                .await
                .map_err(invalid)?;
            if loaded.state.current_head() != Some(&head) {
                let already_observed =
                    loaded
                        .state
                        .pending_head_changes()
                        .iter()
                        .any(|change| match change {
                            agend_core::pipeline::state::PendingHeadChange::CommitCreated {
                                head: pending,
                                ..
                            } => pending == &head,
                            agend_core::pipeline::state::PendingHeadChange::MainAdvanced {
                                rebased_head,
                                ..
                            } => rebased_head == &head,
                        });
                if !already_observed {
                    let patch_id = git.patch_id(repo, &head).await.map_err(invalid)?;
                    self.commit_transition(
                        &loaded,
                        PipelineEvent::CommitCreated { head, patch_id },
                        None,
                    )
                    .await?;
                    loaded = self.load(task).await?;
                }
            }
        }
        self.commit_transition(&loaded, event, detail).await?;
        Ok(CommandResult::Accepted)
    }
    async fn commit_transition(
        &mut self,
        loaded: &Loaded,
        event: PipelineEvent,
        detail: Option<String>,
    ) -> Result<(), Refusal> {
        let human_action = if matches!(
            loaded.state.current_stage().map(|s| &s.stage),
            Some(Stage::Approval {
                by: Approver::Human,
                ..
            })
        ) {
            match &event {
                PipelineEvent::ApprovalGranted { .. } => Some(AttentionAction::Approve),
                PipelineEvent::ChangesRequested { .. } => Some(AttentionAction::RequestChanges),
                _ => None,
            }
        } else {
            None
        };
        let (task, next, entered, actions) = transition::advance(
            self.store.as_ref(),
            &self.clock,
            loaded,
            event,
            detail,
            self.executor.new_id().map_err(db)?,
        )
        .await?;
        let human_resolution =
            human_action.map(|action| (format!("approval:{}", ticket(&loaded.state)), action));
        self.clear_task_attention(&task.id, &next, human_resolution);
        let result = async {
            // Drop obsolete review bindings before a new attempt is dispatched.
            for binding in self
                .store
                .bindings()
                .await
                .map_err(db)?
                .into_iter()
                .filter(|b| {
                    b.task == task.id
                        && b.kind == "review"
                        && (b.ticket != ticket(&next)
                            || next.approval_reviewers().contains(&b.instance))
                })
            {
                self.release(&binding, false).await?;
            }
            for action in actions {
                self.action(&task, &next, entered, action).await?;
            }
            Ok(())
        }
        .await;
        if let Err((_, reason)) = &result {
            self.fail_restore(&task.id, &format!("execution failed: {reason}"))
                .await?;
        }
        result
    }
}
fn ticket(state: &PipelineState) -> String {
    format!(
        "{}/{}/{}",
        state.task_id(),
        state.current_stage().map_or("done", |s| s.id.as_str()),
        state.attempt()
    )
}

#[cfg(test)]
mod tests;
