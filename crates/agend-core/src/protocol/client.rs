//! Client protocol v1: what TUI, CLI and future GUIs exchange with the daemon
//! over its unix socket.
//!
//! Must NOT: contain transport code (sockets live in `agend-client` and the
//! daemon's `server` module).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use serde::{Deserialize, Serialize};

use super::{Hello, ProtocolVersion, VersionMismatch, negotiate};

pub const V1: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const SUPPORTED_VERSIONS: [ProtocolVersion; 1] = [V1];

/// Requests sent as individual JSON Lines. All non-hello variants are only
/// valid after the peer has selected a compatible version. The nested `data`
/// structs preserve v1's wire shape while the internal tag lets older peers
/// ignore an unknown future variant, including its payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientRequest {
    Hello {
        data: Hello,
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
    #[serde(other)]
    Unknown,
}

impl ClientRequest {
    pub fn hello() -> Self {
        Self::Hello {
            data: Hello::new(&SUPPORTED_VERSIONS),
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
pub struct TerminalInputData {
    pub instance_id: String,
    pub bytes_base64: String,
}

/// Agent-facing commands from D17. The daemon authenticates the caller and
/// binds approvals to the review head; agents submit only their intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum AgentCommand {
    Status,
    Done {
        task_id: String,
    },
    Result {
        task_id: String,
        summary: String,
        output: Option<String>,
    },
    ReviewApprove {
        task_id: String,
    },
    ReviewChanges {
        task_id: String,
        summary: String,
    },
    Send {
        to: String,
        message: String,
    },
    Inbox {
        after_message_id: Option<String>,
    },
    Ask {
        question: String,
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
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedVersionData {
    pub selected: ProtocolVersion,
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
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusData {
    pub task_id: Option<String>,
    pub instance_id: Option<String>,
    pub summary: String,
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
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionRequiredData {
    pub reason: String,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskChangedData {
    pub task_id: String,
    pub summary: String,
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
}

/// Negotiate protocol v1 before handling any other client request.
pub fn negotiate_version(remote: &Hello) -> Result<ProtocolVersion, VersionMismatch> {
    negotiate("client", &SUPPORTED_VERSIONS, &remote.supported)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientProtocolError(pub VersionMismatch);

impl fmt::Display for ClientProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl core::error::Error for ClientProtocolError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_hello_advertises_v1() {
        let ClientRequest::Hello { data: hello } = ClientRequest::hello() else {
            unreachable!();
        };
        assert_eq!(negotiate_version(&hello), Ok(V1));
    }

    #[test]
    fn client_rejects_an_unknown_major() {
        let error = negotiate_version(&Hello::new(&[ProtocolVersion::new(2, 0)])).unwrap_err();
        assert!(error.message().contains("client protocol version mismatch"));
    }
}
