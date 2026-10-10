//! Additive client 1.4 terminal messages (D39). A view is scoped to one
//! daemon connection; each successful Acquire receives a fresh attach ID.

use crate::protocol::terminal::{TerminalFrame, TerminalSize, TerminalViewport};
use alloc::string::String;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSubscribeData {
    pub request_id: String,
    pub instance_id: String,
    pub viewport: TerminalViewport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalViewportData {
    /// Resize only when no view controls the PTY; never grants input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit_size: Option<TerminalSize>,
    pub request_id: String,
    pub instance_id: String,
    pub view_id: String,
    pub generation: String,
    pub viewport: TerminalViewport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientTerminalControlData {
    pub request_id: String,
    pub instance_id: String,
    pub view_id: String,
    pub generation: String,
    pub operation: ClientTerminalOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ClientTerminalOperation {
    /// The daemon generates the attach ID; callers cannot reuse an old grant.
    Acquire {
        size: TerminalSize,
    },
    Resize {
        attach_id: String,
        size: TerminalSize,
    },
    Input {
        attach_id: String,
        bytes_base64: String,
    },
    Release {
        attach_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TerminalControlState {
    ReadOnly,
    Controlled { attach_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientTerminalFrameData {
    /// The latest viewport selection's request ID, including unsolicited updates.
    pub request_id: String,
    pub instance_id: String,
    pub view_id: String,
    pub frame: TerminalFrame,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientTerminalControlAck {
    pub request_id: String,
    pub instance_id: String,
    pub view_id: String,
    pub generation: String,
    pub control: TerminalControlState,
    /// Acquire/Resize acknowledge the real PTY size with a complete live frame.
    pub frame: Option<TerminalFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalControlChangedData {
    pub instance_id: String,
    pub view_id: String,
    pub generation: String,
    pub control: TerminalControlState,
    pub reason: String,
}
