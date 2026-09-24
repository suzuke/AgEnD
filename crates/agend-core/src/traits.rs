//! Boundary traits between the daemon's domain logic and its adapters
//! (D9, D11). The pipeline state machine stays pure and never calls a trait.
//!
//! Implementations and contract fakes live outside this crate. Async methods
//! return no runtime-specific types; `Clock` is synchronous and deterministic.
//!
//! Must NOT: contain implementations or refer to an async runtime.

use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;

use crate::model::Backend;
use crate::pipeline::task::Task;
use crate::pipeline::workflow::Workflow;
use crate::policy::busy::BusyLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessage {
    pub id: String,
    pub from: String,
    pub task_id: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryReceipt {
    pub backend_message_id: Option<String>,
    pub state: crate::model::DeliveryState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverEvent {
    pub cursor: String,
    pub kind: DriverEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverEventKind {
    BusyChanged { busy: bool },
    MessageConfirmed { message_id: String },
    UsageLimit { reset_at_unix_ms: Option<u64> },
    TurnCompleted { summary: Option<String> },
}

pub trait Driver: Sync {
    type Error: Send;

    fn deliver<'a>(
        &'a self,
        instance_id: &'a str,
        message: &'a AgentMessage,
        mode: BusyLevel,
    ) -> impl Future<Output = Result<DeliveryReceipt, Self::Error>> + Send + 'a;

    fn events<'a>(
        &'a self,
        instance_id: &'a str,
        after_cursor: Option<&'a str>,
    ) -> impl Future<Output = Result<Vec<DriverEvent>, Self::Error>> + Send + 'a;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    pub task_id: String,
    pub branch: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedChange {
    pub id: String,
    pub url: Option<String>,
    pub head: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRequest {
    pub branch: String,
    pub expected_head: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeResult {
    Merged { merge_commit: String },
    HeadChanged { actual_head: String },
}

pub trait Forge: Sync {
    type Error: Send;

    fn submit<'a>(
        &'a self,
        change: &'a Submission,
    ) -> impl Future<Output = Result<SubmittedChange, Self::Error>> + Send + 'a;

    fn head<'a>(
        &'a self,
        branch: &'a str,
    ) -> impl Future<Output = Result<String, Self::Error>> + Send + 'a;

    fn merge_if_head_is<'a>(
        &'a self,
        request: &'a MergeRequest,
    ) -> impl Future<Output = Result<MergeResult, Self::Error>> + Send + 'a;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedTask {
    pub version: u64,
    pub task: Task,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CasResult {
    Written { new_version: u64 },
    Conflict { current_version: Option<u64> },
}

pub trait Store: Sync {
    type Error: Send;

    fn load_task<'a>(
        &'a self,
        task_id: &'a str,
    ) -> impl Future<Output = Result<Option<VersionedTask>, Self::Error>> + Send + 'a;

    fn create_task<'a>(
        &'a self,
        task: &'a Task,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;

    fn compare_and_swap_task<'a>(
        &'a self,
        task: &'a Task,
        expected_version: u64,
    ) -> impl Future<Output = Result<CasResult, Self::Error>> + Send + 'a;

    fn load_workflow<'a>(
        &'a self,
        workflow_id: &'a str,
        version: u64,
    ) -> impl Future<Output = Result<Option<Workflow>, Self::Error>> + Send + 'a;

    fn append_event<'a>(
        &'a self,
        task_id: &'a str,
        event: &'a StoredEvent,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub id: String,
    pub occurred_at_unix_ms: u64,
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderLaunch {
    pub instance_id: String,
    pub backend: Backend,
    pub executable: String,
    pub args: Vec<String>,
    pub working_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderHandle {
    pub instance_id: String,
    pub process_id: Option<u32>,
    pub socket_path: String,
}

pub trait Runtime: Sync {
    type Error: Send;

    fn start_holder<'a>(
        &'a self,
        launch: &'a HolderLaunch,
    ) -> impl Future<Output = Result<HolderHandle, Self::Error>> + Send + 'a;

    fn stop_holder<'a>(
        &'a self,
        instance_id: &'a str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;

    fn recover_holders(
        &self,
    ) -> impl Future<Output = Result<Vec<HolderHandle>, Self::Error>> + Send + '_;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub severity: NotificationSeverity,
    pub title: String,
    pub body: String,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationSeverity {
    Info,
    Attention,
    Error,
}

pub trait Notifier: Sync {
    type Error: Send;

    fn notify<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
}

pub trait Clock {
    fn now_unix_ms(&self) -> u64;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

pub trait Runner: Sync {
    type Error: Send;

    fn run<'a>(
        &'a self,
        command: &'a str,
        working_directory: &'a str,
        timeout_ms: u64,
    ) -> impl Future<Output = Result<CommandOutput, Self::Error>> + Send + 'a;
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use core::convert::Infallible;

    fn assert_send<T: Send>(_: T) {}

    struct FakeDriver;

    impl Driver for FakeDriver {
        type Error = Infallible;

        async fn deliver(
            &self,
            _instance_id: &str,
            _message: &AgentMessage,
            _mode: BusyLevel,
        ) -> Result<DeliveryReceipt, Self::Error> {
            Ok(DeliveryReceipt {
                backend_message_id: None,
                state: crate::model::DeliveryState::Sent,
            })
        }

        async fn events(
            &self,
            _instance_id: &str,
            _after_cursor: Option<&str>,
        ) -> Result<Vec<DriverEvent>, Self::Error> {
            Ok(vec![])
        }
    }

    #[test]
    fn async_boundary_futures_are_send_for_multithreaded_runtimes() {
        let driver = FakeDriver;
        let message = AgentMessage {
            id: "m-1".into(),
            from: "operator".into(),
            task_id: None,
            body: "hello".into(),
        };
        assert_send(driver.deliver("instance", &message, BusyLevel::Queue));
        assert_send(driver.events("instance", None));
    }
}
