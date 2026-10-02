//! Ports used by the serialized pipeline domain and its IO adapters.
use crate::pipeline::{task::Task, workflow::Workflow};
use crate::protocol::client::*;
use crate::runtime_records::*;
use crate::traits::{CommandOutput, Forge, Store, StoredEvent};
use alloc::{string::String, vec::Vec};
use core::future::Future;

pub trait PipelineStore: Store {
    /// One CAS transaction clears the attention reason (preserving acknowledgement)
    /// and confirms the dispatch whose result was accepted. Errors/conflicts change neither.
    fn advance_pipeline<'a>(
        &'a self,
        task: &'a Task,
        version: u64,
        progress: &'a crate::traits::TaskProgress,
        event: &'a StoredEvent,
        confirmation: Option<&'a str>,
    ) -> impl Future<Output = Result<crate::traits::CasResult, Self::Error>> + Send + 'a;

    fn advance_message<'a>(
        &'a self,
        id: &'a str,
        next: crate::model::DeliveryState,
        turn_id: Option<String>,
        now: u64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn teams<'a>(&'a self) -> impl Future<Output = Result<Vec<Team>, Self::Error>> + Send + 'a;
    fn add_team<'a>(
        &'a self,
        team: &'a Team,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn members<'a>(&'a self) -> impl Future<Output = Result<Vec<Member>, Self::Error>> + Send + 'a;
    fn join_team<'a>(
        &'a self,
        team: &'a str,
        instance: &'a str,
        role: &'a str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn set_team_workflow<'a>(
        &'a self,
        team: &'a str,
        workflow: &'a str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn progress<'a>(
        &'a self,
        task: &'a str,
    ) -> impl Future<Output = Result<Option<Progress>, Self::Error>> + Send + 'a;
    fn task_note<'a>(
        &'a self,
        task: &'a str,
        block: Option<String>,
        attention: Option<String>,
        ack: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn bindings<'a>(
        &'a self,
    ) -> impl Future<Output = Result<Vec<BindingRow>, Self::Error>> + Send + 'a;
    fn put_binding<'a>(
        &'a self,
        b: &'a BindingRow,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn delete_binding<'a>(
        &'a self,
        instance: &'a str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn latest_workflow<'a>(
        &'a self,
        id: &'a str,
    ) -> impl Future<Output = Result<Option<crate::pipeline::workflow::Workflow>, Self::Error>> + Send + 'a;
    fn workflow_ids<'a>(
        &'a self,
    ) -> impl Future<Output = Result<Vec<String>, Self::Error>> + Send + 'a;
    fn held_task<'a>(
        &'a self,
        instance: &'a str,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send + 'a;
    fn asks<'a>(&'a self) -> impl Future<Output = Result<Vec<AskRow>, Self::Error>> + Send + 'a;
    fn save_ask<'a>(
        &'a self,
        row: &'a AskRow,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn pending_answers<'a>(
        &'a self,
    ) -> impl Future<Output = Result<Vec<PendingAnswer>, Self::Error>> + Send + 'a;
    fn mark_answer_sent<'a>(
        &'a self,
        seq: u64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn reminders<'a>(
        &'a self,
    ) -> impl Future<Output = Result<Vec<(u64, String, u64)>, Self::Error>> + Send + 'a;
    fn add_reminder<'a>(
        &'a self,
        task: &'a str,
        due: u64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn delete_reminder<'a>(
        &'a self,
        seq: u64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn create_pipeline_task<'a>(
        &'a self,
        task: &'a crate::pipeline::task::Task,
        pipeline: &'a str,
        now: u64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn tasks<'a>(&'a self) -> impl Future<Output = Result<Vec<Task>, Self::Error>> + Send + 'a;
    fn instance<'a>(
        &'a self,
        id: &'a str,
    ) -> impl Future<Output = Result<Option<Instance>, Self::Error>> + Send + 'a;
    fn instances<'a>(
        &'a self,
    ) -> impl Future<Output = Result<Vec<Instance>, Self::Error>> + Send + 'a;
    fn message<'a>(
        &'a self,
        id: &'a str,
    ) -> impl Future<Output = Result<Option<Message>, Self::Error>> + Send + 'a;
    fn claim_message<'a>(
        &'a self,
        message: &'a NewMessage,
        now_unix_ms: u64,
    ) -> impl Future<Output = Result<Claim, Self::Error>> + Send + 'a;
    fn load_events<'a>(
        &'a self,
        task_id: &'a str,
    ) -> impl Future<Output = Result<Vec<StoredEvent>, Self::Error>> + Send + 'a;
    fn save_workflow<'a>(
        &'a self,
        workflow: &'a Workflow,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}

#[derive(Debug)]
pub enum ExecutionError {
    Blocked(String),
    Sandbox(String),
    Failed(String),
}
impl core::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Blocked(s) | Self::Sandbox(s) | Self::Failed(s) => f.write_str(s),
        }
    }
}

/// How a writer relinquishes a binding without losing a task's branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingRelease {
    Abandoned,
    Merged,
    Handoff,
}

/// All filesystem/process effects, separate from assignment and state decisions.
pub trait PipelineExecutor: Clone + Send + Sync + 'static {
    type Forge: Forge<Error = ExecutionError> + Send;
    fn git_available(&self) -> bool;
    fn new_id(&self) -> Result<String, String>;
    fn canonical_repo(&self, repo: &str) -> Result<String, String>;
    fn forge(&self, repo: &str, expected_main: Option<String>) -> Self::Forge;
    fn run<'a>(
        &'a self,
        repo: &'a str,
        args: &'a [&'a str],
    ) -> impl Future<Output = Result<String, String>> + Send + 'a;
    /// Prove a worktree has no modifications or index flags concealing them.
    fn clean_worktree<'a>(
        &'a self,
        repo: &'a str,
    ) -> impl Future<Output = Result<bool, String>> + Send + 'a;
    fn ancestor<'a>(
        &'a self,
        repo: &'a str,
        a: &'a str,
        b: &'a str,
    ) -> impl Future<Output = Result<bool, String>> + Send + 'a;
    fn patch_id<'a>(
        &'a self,
        repo: &'a str,
        head: &'a str,
    ) -> impl Future<Output = Result<String, String>> + Send + 'a;
    fn find_merge<'a>(
        &'a self,
        repo: &'a str,
        task: &'a str,
        head: &'a str,
    ) -> impl Future<Output = Result<Option<(String, bool)>, String>> + Send + 'a;
    fn readiness(&self) -> impl Future<Output = Result<(), String>> + Send + '_;
    fn ensure<'a>(
        &'a self,
        repo: &'a str,
        binding: &'a BindingRow,
    ) -> impl Future<Output = Result<(), String>> + Send + 'a;
    fn release<'a>(
        &'a self,
        repo: &'a str,
        binding: &'a BindingRow,
        mode: BindingRelease,
    ) -> impl Future<Output = Result<Option<String>, String>> + Send + 'a;
    fn projections<'a>(
        &'a self,
        members: &'a [Member],
        teams: &'a [Team],
        bindings: &'a [BindingRow],
    ) -> impl Future<Output = Result<(), String>> + Send + 'a;
    fn orphans<'a>(
        &'a self,
        tasks: &'a [Task],
        teams: &'a [Team],
        bindings: &'a [BindingRow],
        running: &'a [String],
    ) -> impl Future<Output = Result<(), String>> + Send + 'a;
    fn check<'a>(
        &'a self,
        repo: &'a str,
        ticket: &'a str,
        head: &'a str,
        command: &'a str,
        timeout: u64,
    ) -> impl Future<Output = Result<CommandOutput, ExecutionError>> + Send + 'a;
}

pub trait PipelineView: Send + Sync + 'static {
    fn view(&self) -> FleetView;
    fn set_teams(&self, teams: Vec<TeamView>);
    fn set_instance(&self, instance: InstanceView, summary: String);
    fn sync_tasks(&self, tasks: Vec<TaskView>);
    fn dismiss(&self, id: &str);
    fn raise(&self, item: AttentionRequiredData);
    fn upsert_attention(&self, item: AttentionRequiredData);
    fn attention(&self, id: &str) -> Option<AttentionRequiredData>;
    fn resolve(&self, id: &str, action: AttentionAction) -> Option<AttentionRequiredData>;
    fn publish(&self, event: DaemonEvent) -> u64;
}
