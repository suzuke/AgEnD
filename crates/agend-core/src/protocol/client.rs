//! Client protocol v1: what TUI, CLI and future GUIs exchange with the daemon
//! over its unix socket (`$AGEND_HOME/run/daemon.sock`, [`DAEMON_SOCKET`]).
//!
//! 1.1 (gate 8) only adds: the optional `caller` in `hello`, `get_fleet` and
//! its `fleet` reply (the fleet view), `resolve_attention` and the
//! `attention_resolved` event, and optional fields on `attention_required`,
//! `instance_changed` and `task_changed`. A 1.0 peer decodes every 1.1
//! message (new variants become `unknown`, new fields are ignored).
//!
//! 1.2 (gate 9) only adds: the `operator` request ([`OperatorCommand`]:
//! `instance_add`, `instance_remove`, `daemon_restart`, `task_cancel`) and
//! its `instance_added` / `restarting` results; optional `level` and
//! `message_id` on `send`; optional `identity` on `status`; optional
//! `daemon_version`, `daemon_pid` and `boot_id` in the `hello` reply;
//! optional `working_directory` on each instance of the fleet view. A 1.1
//! peer decodes every 1.2 message the same way.
//!
//! Event cursor rules (gate 8 P4): event ids start from a base (the daemon's
//! boot time in unix ms × 1000); the first event is base + 1. The daemon
//! keeps the last [`RETAINED_EVENTS`] events in memory. `subscribe_events`
//! with `after_event_id`:
//! - `None`: replay every retained event, then live ones (the 1.0 meaning);
//! - from "oldest retained − 1" up to the newest id (the base itself while
//!   there is no event yet): the events after it, then live ones;
//! - anything else (older, or newer than the newest id): error
//!   [`error_code::EVENT_GAP`]; the client fetches the fleet view again.
//!
//! A 1.1 client never reuses a cursor after it reconnects: it fetches the
//! fleet view (`get_fleet`) and subscribes after its `as_of_event_id`.
//!
//! Must NOT: contain transport code (sockets live in `agend-client` and the
//! daemon's `server` module).

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

use super::ask::{AnswerSource, AskReply, AskThread, ContextRecap};
use super::{ProtocolVersion, VersionMismatch, negotiate};
use crate::policy::attention::AttentionItem;

mod terminal;
pub use terminal::*;

pub const V1: ProtocolVersion = ProtocolVersion::new(1, 0);
/// Gate 8: fleet view, needs-you actions, caller identity.
pub const V1_1: ProtocolVersion = ProtocolVersion::new(1, 1);
/// Gate 9: operator requests, send level and message id, daemon identity.
/// `daemon_restart` exists since 1.2 and never changes shape (gate 9 P6).
pub const V1_2: ProtocolVersion = ProtocolVersion::new(1, 2);
pub const V1_3: ProtocolVersion = ProtocolVersion::new(1, 3);
/// Full terminal API capability. General clients still require only 1.3.
pub const V1_4: ProtocolVersion = ProtocolVersion::new(1, 4);
/// Client-side offers; the daemon advertises 1.4 only when its path is ready.
pub const OFFERED_VERSIONS: [ProtocolVersion; 2] = [V1_4, V1_3];
/// Legacy fixture baseline. The real server and parser-backed fake fixtures
/// advertise 1.4 independently once a full-terminal producer is available.
pub const SUPPORTED_VERSIONS: [ProtocolVersion; 1] = [V1_3];

/// The daemon's socket, relative to the AgEnD home.
pub const DAEMON_SOCKET: &str = "run/daemon.sock";

/// Longest `send` body in bytes (1 MiB, gate 9 L17). Larger is
/// `invalid_request` before anything is stored: the whole body goes into
/// one backend call (codex: one WebSocket frame, at most 16 MiB).
pub const MAX_MESSAGE_BYTES: usize = 1 << 20;
/// Longest protocol line in bytes (8 MiB, gate 9 L17): room for a
/// [`MAX_MESSAGE_BYTES`] body whose every byte JSON escapes to `\u00XX`.
/// A longer line gets `invalid_request` and the connection is closed,
/// before the line is read into memory.
pub const MAX_LINE_BYTES: usize = 8 << 20;

/// What a `send` whose body is `len` bytes gets (the daemon, the fake and
/// the CLI say the same).
pub fn message_too_long(len: usize) -> String {
    alloc::format!(
        "the message is {len} bytes; a message is limited to {MAX_MESSAGE_BYTES} bytes (1 MiB)"
    )
}

/// How many events the daemon keeps for `subscribe_events`; a subscriber
/// that falls further behind is disconnected (gate 8 P8).
pub const RETAINED_EVENTS: usize = 1024;

/// `ErrorData::code` values: the only list (gate 8 P3). The CLI picks its
/// message and exit code by these.
pub mod error_code {
    /// The first message was not `hello`; the connection is closed.
    pub const HELLO_REQUIRED: &str = "hello_required";
    /// No shared major version; the connection is closed.
    pub const VERSION_MISMATCH: &str = "version_mismatch";
    /// A line that is not a valid request (bad JSON, bad fields).
    pub const INVALID_REQUEST: &str = "invalid_request";
    /// A request type or agent command this peer does not know.
    pub const UNKNOWN_REQUEST: &str = "unknown_request";
    pub const UNKNOWN_ASK: &str = "unknown_ask";
    /// See [`super::STALE_RESULT`].
    pub const STALE_RESULT: &str = "stale_result";
    /// The cursor cannot be continued, or the subscriber fell behind; fetch
    /// the fleet view again.
    pub const EVENT_GAP: &str = "event_gap";
    /// A known request this daemon does not serve yet; nothing changed.
    pub const NOT_SUPPORTED: &str = "not_supported";
    /// The caller may not do this (an agent sending an operator request).
    pub const FORBIDDEN: &str = "forbidden";
    /// The instance has no terminal to show.
    pub const NO_TERMINAL: &str = "no_terminal";
    /// 1.4: the client view or holder connection/generation is no longer current.
    pub const STALE_TERMINAL: &str = "stale_terminal";
    /// 1.4: another view now owns the PTY.
    pub const CONTROL_LOST: &str = "control_lost";
    /// 1.4: legacy input has no attach while a full-terminal view owns the PTY.
    pub const CONTROL_REQUIRED: &str = "control_required";
    pub const INVALID_SIZE: &str = "invalid_size";
    pub const FRAME_TOO_LARGE: &str = "frame_too_large";
    pub const PTY_BUSY: &str = "pty_busy";
    /// No such needs-you item, or the action is not one of its `actions`.
    pub const UNKNOWN_ATTENTION: &str = "unknown_attention";
    /// 1.2: no instance with this name.
    pub const UNKNOWN_INSTANCE: &str = "unknown_instance";
    /// 1.2: the name is taken (a row, or a holder that still runs).
    pub const INSTANCE_EXISTS: &str = "instance_exists";
    /// 1.2: the new binary failed the restart preflight; nothing changed.
    pub const PREFLIGHT_FAILED: &str = "preflight_failed";
    /// 1.2: `inbox` after a message id the caller has no message with.
    pub const UNKNOWN_MESSAGE: &str = "unknown_message";
}

/// The `hello` of a client. `caller` is the instance id when the CLI runs
/// inside an agent (`AGEND_INSTANCE`); none means the operator. Any caller
/// is treated as an agent: it can only lose permissions by naming itself
/// (gate 8 P2). Not the holder protocol's `Hello`, so `caller` never
/// reaches that protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHello {
    pub supported: Vec<ProtocolVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
}

/// Requests sent as individual JSON Lines. All non-hello variants are only
/// valid after the peer has selected a compatible version. The nested `data`
/// structs preserve v1's wire shape while the internal tag lets older peers
/// ignore an unknown future variant, including its payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientRequest {
    Hello {
        data: ClientHello,
    },
    Command {
        data: ClientCommandData,
    },
    SubscribeEvents {
        data: SubscribeEventsData,
    },
    SubscribeTerminal {
        data: InstanceData,
    },
    /// Raw PTY input is operator-originated input for the interactive attach
    /// view. Agent messages continue to use the structured delivery API.
    TerminalInput {
        data: TerminalInputData,
    },
    /// 1.4: one connection-scoped, structured terminal view.
    SubscribeTerminalFrames {
        data: TerminalSubscribeData,
    },
    /// 1.4: select this view's history without changing the live parser grid.
    SetTerminalViewport {
        data: TerminalViewportData,
    },
    /// 1.4: operator control; never replayed on another daemon connection.
    TerminalControl {
        data: ClientTerminalControlData,
    },
    /// Operator answer to a needs-you ask, from the TUI, Telegram or CLI (D35).
    AnswerAsk {
        data: AnswerAskData,
    },
    /// 1.1: the fleet view, answered with [`ClientResponse::Fleet`].
    GetFleet {
        data: RequestIdData,
    },
    /// 1.1: the operator acts on a needs-you item that is not an ask;
    /// answered with `command_result` `accepted`, then `attention_resolved`.
    ResolveAttention {
        data: ResolveAttentionData,
    },
    /// 1.2: an operator command (only the operator; agents get
    /// `forbidden`). Answered with `command_result`.
    Operator {
        data: OperatorData,
    },
    #[serde(other)]
    Unknown,
}

impl ClientRequest {
    /// `hello` of the operator.
    pub fn hello() -> Self {
        Self::hello_as(None)
    }

    /// `hello` naming the caller (`Some(instance id)` inside an agent).
    pub fn hello_as(caller: Option<String>) -> Self {
        Self::Hello {
            data: ClientHello {
                supported: OFFERED_VERSIONS.to_vec(),
                caller,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorData {
    pub request_id: String,
    pub command: OperatorCommand,
}

/// What the operator asks the daemon to do (gate 9 P6). Users see the
/// instance id as its `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum OperatorCommand {
    /// Adds an instance and starts it (answered `instance_added`).
    /// `working_directory` defaults to `$AGEND_HOME/workspace/<id>`,
    /// `program` to the backend's name; `args` are the agent's own.
    InstanceAdd {
        instance_id: String,
        /// `claude`, `codex` or `opencode`.
        backend: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        working_directory: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
    /// Stops the instance's holder and removes its row; the workspace stays.
    InstanceRemove {
        instance_id: String,
    },
    /// Preflight `binary` (default: the daemon's own), then `exec` it
    /// (answered `restarting`). Its shape never changes (gate 9 P6).
    DaemonRestart {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binary: Option<String>,
    },
    /// Cancels a task (its handler arrives in gate 10).
    TaskCancel {
        task_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    TaskCreate {
        title: String,
        role: String,
        team_id: String,
        workflow_id: Option<String>,
    },
    TeamAdd {
        team_id: String,
        repo: Option<String>,
        workflow_id: Option<String>,
    },
    TeamList,
    TeamJoin {
        team_id: String,
        instance_id: String,
        role: String,
    },
    TeamSetWorkflow {
        team_id: String,
        workflow_id: String,
    },
    WorkflowList,
    WorkflowShow {
        workflow_id: String,
    },
    WorkflowCheck {
        toml: String,
    },
    WorkflowApply {
        toml: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestIdData {
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveAttentionData {
    pub request_id: String,
    pub attention_id: String,
    pub action: AttentionAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What the operator can do with a needs-you item that is not an ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionAction {
    /// Start a `failed` instance again (gate 8 P5).
    Retry,
    Approve,
    RequestChanges,
    Acknowledge,
    #[serde(other)]
    Unknown,
}

impl AttentionAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retry => "retry",
            Self::Approve => "approve",
            Self::RequestChanges => "request_changes",
            Self::Acknowledge => "acknowledge",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientCommandData {
    pub request_id: String,
    pub command: AgentCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribeEventsData {
    pub after_event_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceData {
    pub instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerAskData {
    pub request_id: String,
    pub ask_id: String,
    pub source: AnswerSource,
    pub reply: AskReply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInputData {
    pub instance_id: String,
    pub bytes_base64: String,
}

/// The stage attempt a result answers, copied from the assignment the agent
/// received (`PipelineAction` `stage_id` and `attempt`). The field is optional
/// on the wire so v1 peers that predate it still decode, but a result without
/// it is stale by default: the daemon rejects it with [`STALE_RESULT`] and
/// changes nothing, exactly like a result for another attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultIdentity {
    pub stage_id: String,
    pub attempt: u32,
}

/// `ErrorData::code` for a result whose identity is missing or not the
/// current stage attempt.
pub const STALE_RESULT: &str = error_code::STALE_RESULT;

/// Agent-facing commands from D17. The daemon authenticates the caller and
/// binds approvals to the review head; agents submit only their intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum AgentCommand {
    Status,
    Done {
        task_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity: Option<ResultIdentity>,
    },
    Result {
        task_id: String,
        summary: String,
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity: Option<ResultIdentity>,
    },
    ReviewApprove {
        task_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity: Option<ResultIdentity>,
    },
    ReviewChanges {
        task_id: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity: Option<ResultIdentity>,
    },
    Send {
        to: String,
        message: String,
        /// 1.2: how a busy target gets it; `queue` when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<MessageLevel>,
        /// 1.2: a UUID v4 the caller chose, so a resend is the same
        /// message; the daemon makes one when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    Inbox {
        after_message_id: Option<String>,
    },
    /// Open a needs-you ask; `options` are optional choices, and the
    /// operator may always answer in free text (D35).
    Ask {
        question: String,
        #[serde(default)]
        options: Vec<String>,
    },
    /// Continue an open ask after an answer.
    AskFollowUp {
        ask_id: String,
        question: String,
        #[serde(default)]
        options: Vec<String>,
    },
    /// Close an ask with what was decided.
    AskResolve {
        ask_id: String,
        summary: String,
    },
    Block {
        task_id: String,
        reason: String,
    },
    Unblock {
        task_id: String,
    },
    TaskCreate {
        title: String,
        role: String,
        team_id: Option<String>,
        workflow_id: Option<String>,
    },
    Remind {
        task_id: String,
        delay_seconds: u64,
    },
    #[serde(other)]
    Unknown,
}

/// The busy level a message is sent with (`agend send --level`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageLevel {
    Queue,
    Steer,
    Interrupt,
    #[serde(other)]
    Unknown,
}

impl MessageLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Steer => "steer",
            Self::Interrupt => "interrupt",
            Self::Unknown => "unknown",
        }
    }
}

/// Whether `id` is a UUID v4 in its lowercase text form (the only message
/// ids a client may choose, gate 9 P5).
pub fn is_uuid_v4(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, &b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
        && bytes[14] == b'4'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
}

/// A UUID v4 in text form from 16 random bytes.
pub fn uuid_v4(mut b: [u8; 16]) -> String {
    use core::fmt::Write;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let mut out = String::with_capacity(36);
    for (i, x) in b.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        let _ = write!(out, "{x:02x}");
    }
    out
}

/// A ticket `<task_id>/<stage_id>/<attempt>` (gate 9 P2): what a result
/// command answers, as printed in the dispatch message and `agend status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub task_id: String,
    pub identity: ResultIdentity,
}

impl Ticket {
    /// Parses `t-42/review/2`; `None` unless it has three non-empty parts
    /// and the attempt is a number.
    pub fn parse(text: &str) -> Option<Ticket> {
        let mut parts = text.split('/');
        let (task, stage, attempt) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || task.is_empty() || stage.is_empty() {
            return None;
        }
        Some(Ticket {
            task_id: task.into(),
            identity: ResultIdentity {
                stage_id: stage.into(),
                attempt: attempt.parse().ok()?,
            },
        })
    }
}

impl core::fmt::Display for Ticket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}/{}/{}",
            self.task_id, self.identity.stage_id, self.identity.attempt
        )
    }
}

/// Responses are decoded and handled one line at a time, never stored in
/// bulk, so the size of the event variant does not matter; boxing it would
/// only change every constructor.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientResponse {
    Hello {
        data: SelectedVersionData,
    },
    CommandResult {
        data: ClientCommandResultData,
    },
    Error {
        data: ErrorData,
    },
    Event {
        data: EventData,
    },
    TerminalSnapshot {
        data: TerminalSnapshotData,
    },
    /// PTY output is base64 text in JSON Lines; the adapter owns encoding.
    TerminalBytes {
        data: TerminalBytesData,
    },
    /// 1.4: only the requested rows, with holder generation and revision.
    TerminalFrame {
        data: ClientTerminalFrameData,
    },
    /// 1.4: completed operation, with actual resized frame for Acquire/Resize.
    TerminalControlAck {
        data: ClientTerminalControlAck,
    },
    /// 1.4: a control change invalidates a former owner's queued input.
    TerminalControlChanged {
        data: TerminalControlChangedData,
    },
    /// 1.1: the answer to `get_fleet`.
    Fleet {
        data: FleetData,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetData {
    pub request_id: String,
    pub fleet: FleetView,
}

/// The fleet view (全貌): everything a client shows, as of one event id.
/// Subscribing after `as_of_event_id` gives every later change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetView {
    pub as_of_event_id: u64,
    pub teams: Vec<TeamView>,
    pub tasks: Vec<TaskView>,
    pub instances: Vec<InstanceView>,
    /// The needs-you list, in D36 order ([`order_attention`]).
    pub attention: Vec<AttentionRequiredData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamView {
    pub team_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskView {
    pub task_id: String,
    pub title: String,
    pub team_id: String,
    /// `open`, `running`, `blocked`, `done` or `superseded`.
    pub status: String,
    #[serde(default)]
    pub assignee: Option<String>,
    /// The workflow's stage ids, in order (empty until gate 10).
    #[serde(default)]
    pub stages: Vec<String>,
    #[serde(default)]
    pub current_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<TaskPipelineView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPipelineView {
    pub repo: Option<String>,
    pub stage_kinds: Vec<String>,
    pub stage_agents: Vec<Option<String>>,
    pub block_reason: Option<String>,
    pub archive_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceView {
    pub instance_id: String,
    pub team_id: String,
    /// `claude`, `codex` or `opencode`.
    pub backend: String,
    pub state: AgentState,
    /// 1.2: where the agent runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

/// What an agent is doing. "Needs you" is not a state: clients derive it
/// from the fleet view's `attention` list, the one source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Starting,
    Working,
    Idle,
    Stuck,
    Failed,
    /// Running, but the daemon cannot tell working from idle (no driver yet),
    /// or a state this client does not know.
    #[serde(other)]
    Unknown,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Working => "working",
            Self::Idle => "idle",
            Self::Stuck => "stuck",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedVersionData {
    pub selected: ProtocolVersion,
    /// 1.2: `agend <version>` of the daemon binary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_pid: Option<u32>,
    /// 1.2: this boot's event id base (boot time in unix ms × 1000); new at
    /// every boot, including an `exec` restart with the same pid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_id: Option<u64>,
}

impl SelectedVersionData {
    /// Only the selected version (a 1.1 reply).
    pub fn new(selected: ProtocolVersion) -> Self {
        Self {
            selected,
            daemon_version: None,
            daemon_pid: None,
            boot_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientCommandResultData {
    pub request_id: String,
    pub result: CommandResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorData {
    pub request_id: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventData {
    pub event_id: u64,
    pub event: DaemonEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSnapshotData {
    pub instance_id: String,
    pub screen: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalBytesData {
    pub instance_id: String,
    pub bytes_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum CommandResult {
    Text {
        text: String,
    },
    Accepted,
    Status {
        data: StatusData,
    },
    Messages {
        data: MessagesData,
    },
    TaskCreated {
        data: TaskCreatedData,
    },
    AskCreated {
        data: AskCreatedData,
    },
    /// 1.2: `instance_add` wrote the row; the instance is starting.
    InstanceAdded {
        data: InstanceAddedData,
    },
    /// 1.2: the preflight passed; the daemon stops and `exec`s the binary.
    Restarting {
        data: RestartingData,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceAddedData {
    pub instance_id: String,
    /// claude's session id; none for codex (its thread comes later) and
    /// opencode.
    pub session_id: Option<String>,
    pub working_directory: String,
}

/// What the preflight printed, one step per line; the first names the new
/// binary's version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestartingData {
    pub preflight: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskCreatedData {
    pub ask_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusData {
    pub task_id: Option<String>,
    pub instance_id: Option<String>,
    pub summary: String,
    /// 1.2: the stage attempt the caller works on, so `agend status` prints
    /// the ticket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<ResultIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagesData {
    pub messages: Vec<InboxMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCreatedData {
    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxMessage {
    pub message_id: String,
    pub from: String,
    pub body: String,
    pub created_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DaemonEvent {
    AttentionRequired {
        data: AttentionRequiredData,
    },
    TaskChanged {
        data: TaskChangedData,
    },
    MessageReceived {
        data: MessageReceivedData,
    },
    InstanceChanged {
        data: InstanceChangedData,
    },
    /// A needs-you thread gained an entry (answer, follow-up, resolution).
    AskUpdated {
        data: AskThread,
    },
    /// 1.1: a needs-you item left the list because the daemon acted on it.
    AttentionResolved {
        data: AttentionResolvedData,
    },
    #[serde(other)]
    Unknown,
}

/// A needs-you item. The fields after `recap` are 1.1 additions, optional
/// on the wire (omitted when absent) so 1.0 peers read the same shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionRequiredData {
    pub reason: String,
    pub task_id: Option<String>,
    /// The ask thread when the item is a needs-you ask (D35).
    #[serde(default)]
    pub ask: Option<AskThread>,
    /// Where the operator is when switching to this item (D37).
    #[serde(default)]
    pub recap: Option<ContextRecap>,
    /// The item's id: the ask id for an ask, otherwise a fixed string such
    /// as `instance-failed:<instance id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention_id: Option<String>,
    /// How much work continues once it is resolved (D36).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unblocks: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_since_unix_ms: Option<u64>,
    /// What happens if nobody acts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_ignored: Option<String>,
    /// What `resolve_attention` accepts for it (asks use `answer_ask`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<AttentionAction>,
    /// The agent the item belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
}

impl AttentionRequiredData {
    /// The item as D36 orders it: `None` without an `attention_id`; a
    /// missing `unblocks` counts 0 and a missing wait start as the newest.
    pub fn order_item(&self) -> Option<AttentionItem> {
        Some(AttentionItem {
            id: self.attention_id.clone()?,
            unblocks: self.unblocks.unwrap_or(0),
            waiting_since_unix_ms: self.waiting_since_unix_ms.unwrap_or(u64::MAX),
        })
    }
}

/// Sorts needs-you items in D36 order (`policy::attention::order`); items
/// without an `attention_id` go last, in their current order.
pub fn order_attention(items: &mut Vec<AttentionRequiredData>) {
    let (mut keyed, rest): (Vec<_>, Vec<_>) = core::mem::take(items)
        .into_iter()
        .partition(|item| item.attention_id.is_some());
    let mut order: Vec<AttentionItem> = keyed.iter().filter_map(|i| i.order_item()).collect();
    crate::policy::attention::order(&mut order);
    keyed.sort_by_key(|item| {
        order
            .iter()
            .position(|o| Some(&o.id) == item.attention_id.as_ref())
    });
    items.extend(keyed);
    items.extend(rest);
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionResolvedData {
    pub attention_id: String,
    pub action: AttentionAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskChangedData {
    pub task_id: String,
    pub summary: String,
    /// 1.1: the task as the fleet view shows it now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReceivedData {
    pub message_id: String,
    pub from: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceChangedData {
    pub instance_id: String,
    pub summary: String,
    /// 1.1: the instance as the fleet view shows it now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<InstanceView>,
}

/// Negotiate protocol v1 before handling any other client request.
pub fn negotiate_version(remote: &ClientHello) -> Result<ProtocolVersion, VersionMismatch> {
    negotiate("client", &SUPPORTED_VERSIONS, &remote.supported)
}

#[cfg(test)]
mod tests {
    use super::*;

    use alloc::vec;

    fn hello(supported: &[ProtocolVersion]) -> ClientHello {
        ClientHello {
            supported: supported.to_vec(),
            caller: None,
        }
    }

    #[test]
    fn client_hello_offers_v1_4_but_current_server_selects_v1_3() {
        let ClientRequest::Hello { data: hello } = ClientRequest::hello() else {
            unreachable!();
        };
        assert_eq!(hello.supported, OFFERED_VERSIONS);
        assert_eq!(negotiate_version(&hello), Ok(V1_3));
        assert_eq!(hello.caller, None);
    }

    #[test]
    fn a_1_1_peer_negotiates_1_1() {
        assert_eq!(negotiate_version(&hello(&[V1_1])), Ok(V1_1));
    }

    #[test]
    fn uuid_v4_text_round_trips_and_other_ids_are_not_uuid_v4() {
        let id = uuid_v4([0xff; 16]);
        assert_eq!(id, "ffffffff-ffff-4fff-bfff-ffffffffffff");
        assert!(is_uuid_v4(&id));
        assert!(is_uuid_v4(&uuid_v4([0; 16])));
        for bad in [
            "",
            "dispatch:t-1/work/1",
            "FFFFFFFF-FFFF-4FFF-BFFF-FFFFFFFFFFFF",
            "ffffffff-ffff-3fff-bfff-ffffffffffff",
            "ffffffff-ffff-4fff-cfff-ffffffffffff",
            "ffffffffffff-4fff-bfff-ffffffffffff-",
        ] {
            assert!(!is_uuid_v4(bad), "{bad}");
        }
    }

    #[test]
    fn tickets_parse_and_print() {
        let ticket = Ticket::parse("t-42/review/2").unwrap();
        assert_eq!(ticket.task_id, "t-42");
        assert_eq!(
            ticket.identity,
            ResultIdentity {
                stage_id: "review".into(),
                attempt: 2
            }
        );
        assert_eq!(alloc::format!("{ticket}"), "t-42/review/2");
        for bad in [
            "t-42",
            "t-42/review",
            "t-42/review/x",
            "/work/1",
            "t/w/1/2",
            "t//1",
        ] {
            assert!(Ticket::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_1_0_peer_negotiates_1_0() {
        assert_eq!(negotiate_version(&hello(&[V1])), Ok(V1));
    }

    #[test]
    fn client_rejects_an_unknown_major() {
        let error = negotiate_version(&hello(&[ProtocolVersion::new(2, 0)])).unwrap_err();
        assert!(error.message().contains("client protocol version mismatch"));
    }

    fn item(id: Option<&str>, unblocks: u32, since: u64) -> AttentionRequiredData {
        AttentionRequiredData {
            reason: "r".into(),
            task_id: None,
            ask: None,
            recap: None,
            attention_id: id.map(Into::into),
            unblocks: Some(unblocks),
            waiting_since_unix_ms: Some(since),
            if_ignored: None,
            actions: vec![],
            instance_id: None,
        }
    }

    #[test]
    fn attention_is_ordered_by_d36_and_unkeyed_items_go_last() {
        let mut items = vec![
            item(None, 9, 1),
            item(Some("old"), 0, 100),
            item(Some("wide"), 2, 900),
            item(Some("new"), 0, 500),
        ];
        order_attention(&mut items);
        let ids: Vec<Option<&str>> = items.iter().map(|i| i.attention_id.as_deref()).collect();
        assert_eq!(ids, [Some("wide"), Some("old"), Some("new"), None]);
    }
}
