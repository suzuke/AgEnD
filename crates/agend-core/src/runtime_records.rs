//! Runtime records crossing domain/adapters; no IO or runtime dependencies.
pub mod claude;
pub use claude::*;
pub mod claude_startup;
pub use claude_startup::*;

use crate::model::{Backend, DeliveryState};
use crate::policy::busy::BusyLevel;
use alloc::{string::String, vec::Vec};
/// Where an instance stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceStatus {
    /// Never started: the first start is fresh.
    New,
    /// The daemon keeps it running; every start after the first resumes.
    Running,
    /// The daemon gave up (gate 6 P6); a human decides.
    Failed,
}

impl InstanceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Running => "running",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::New, Self::Running, Self::Failed]
            .into_iter()
            .find(|v| v.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub id: String,
    pub backend: Backend,
    pub program: String,
    /// The agent's base arguments; session arguments are added per start.
    pub args: Vec<String>,
    pub working_directory: String,
    /// The backend session to resume: claude's session id, codex's thread
    /// id (created by the daemon, gate 7 P3); `None` when there is none yet
    /// (opencode until gate 12).
    pub session_id: Option<String>,
    pub status: InstanceStatus,
    /// The backend session was created (the first `Spawn` was
    /// acknowledged); set with `running` and never cleared (migration 0003).
    pub session_started: bool,
    /// The agent's pid (its own process group) from the last `Spawned`;
    /// cleared by the codex sweep after its holder died (gate 7 P2).
    pub agent_pid: Option<u32>,
    /// A codex instance migration 0004 found without a thread id that may
    /// hold a conversation: never started again, a human decides (gate 7 P3).
    pub legacy_no_thread: bool,
    pub delivery: String,
}

/// A message to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub id: String,
    pub from_instance: String,
    pub to_instance: String,
    pub task_id: Option<String>,
    pub body: String,
    pub level: BusyLevel,
}

/// A stored message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub seq: i64,
    pub id: String,
    pub from_instance: String,
    pub to_instance: String,
    pub task_id: Option<String>,
    pub body: String,
    pub level: BusyLevel,
    pub state: DeliveryState,
    pub turn_id: Option<String>,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    /// Set just before the first RPC that sends it: a `queued` row with it
    /// may have reached codex (the reply was lost, or the daemon stopped).
    pub attempted_at_unix_ms: Option<u64>,
}

impl Message {
    /// Same id, sender, target, task, body and level as `new`.
    pub fn same_as(&self, new: &NewMessage) -> bool {
        self.id == new.id
            && self.from_instance == new.from_instance
            && self.to_instance == new.to_instance
            && self.task_id == new.task_id
            && self.body == new.body
            && self.level == new.level
    }
}

/// What [`claim`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// A new id: inserted as `queued`.
    Inserted(Message),
    /// The id exists with the same content: nothing changed.
    Existing(Message),
    /// The id exists with other content: nothing changed (`invalid_request`).
    Different(Message),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    pub id: String,
    pub repo: Option<String>,
    pub default_workflow: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: String,
    pub team: String,
    pub role: String,
    pub delivery: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRow {
    pub instance: String,
    pub task: String,
    pub kind: String,
    pub worktree: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub ticket: String,
    pub status: String,
}
#[derive(Debug, Clone)]
pub struct Progress {
    pub data: TaskProgress,
    pub block_reason: Option<String>,
    pub attention_reason: Option<String>,
    pub acknowledged: bool,
}

#[derive(Debug, Clone)]
pub struct AskRow {
    pub instance: String,
    pub created: u64,
    pub thread: crate::protocol::ask::AskThread,
}

use crate::traits::TaskProgress;

use alloc::format;
const MAX_ID: usize = 24;
pub fn validate_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid instance id {id:?}: use 1-{MAX_ID} characters from a-z 0-9 -"
        ))
    }
}

/// One durable answer outbox item: sequence, identity, recipient, task, JSON body.
pub type PendingAnswer = (u64, String, String, Option<String>, String);
