//! Claude bridge RPCs (client 1.5), distinct from MCP stdio framing.
use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

/// Bound the complete batch, never truncate a message. ACK and written
/// receipts use the original tuple even after the live session changes.
pub const CLAUDE_BATCH_BYTES: usize = 1 << 20;
pub const CLAUDE_BATCH_COUNT: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeReceipt {
    pub message_id: String,
    pub delivery_id: String,
    pub session_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudePush {
    pub receipt: ClaudeReceipt,
    pub content: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ClaudeOperation {
    Attach,
    Poll {
        session_id: String,
    },
    Written {
        receipts: Vec<ClaudeReceipt>,
    },
    Ack {
        receipts: Vec<ClaudeReceipt>,
    },
    Hook {
        event_id: String,
        session_id: String,
        event: String,
        payload: String,
        occurred_at_unix_ms: u64,
        replayed: bool,
    },
    #[serde(other)]
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeRequestData {
    pub request_id: String,
    pub instance_id: String,
    pub operation: ClaudeOperation,
}
/// A struct avoids an internally-tagged result's unbounded conversion tail
/// when the one-shot client decodes full message content under its deadline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeReplyData {
    pub request_id: String,
    pub session_id: Option<String>,
    pub messages: Vec<ClaudePush>,
    pub committed: bool,
}

/// Versioned disk envelope shared by helpers and the daemon ingester.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudePendingRecord {
    pub version: u32,
    pub request: ClaudeRequestData,
}
