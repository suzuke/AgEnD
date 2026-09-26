//! Holder protocol: daemon <-> per-instance holder. Carries spawn, PTY
//! resize/signal, single control keys, screen snapshots, output stream and
//! exit status. Needs real version negotiation (v1's `framing.rs` had a single
//! version byte and no negotiation).
//!
//! Must NOT: carry agent message content. That uses the backend's structured
//! API. PTY input is limited to enumerated control keys or a separately tagged
//! operator attach stream.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};

use super::{Hello, ProtocolVersion, VersionMismatch, negotiate};

pub const V1: ProtocolVersion = ProtocolVersion::new(1, 0);
pub const SUPPORTED_VERSIONS: [ProtocolVersion; 1] = [V1];

/// A single control key the daemon may ask a holder to write to an agent PTY.
/// Typing text into the PTY is not a supported delivery path in v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKey {
    /// Interrupts the current turn (claude driver, D16).
    Esc,
    Enter,
    Up,
    Down,
    Left,
    Right,
    Digit1,
    Digit2,
    Digit3,
    Y,
    N,
    Unknown,
}

impl<'de> Deserialize<'de> for ControlKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ControlKeyVisitor;

        impl Visitor<'_> for ControlKeyVisitor {
            type Value = ControlKey;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a control key name")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(match value {
                    "esc" => ControlKey::Esc,
                    "enter" => ControlKey::Enter,
                    "up" => ControlKey::Up,
                    "down" => ControlKey::Down,
                    "left" => ControlKey::Left,
                    "right" => ControlKey::Right,
                    "digit1" => ControlKey::Digit1,
                    "digit2" => ControlKey::Digit2,
                    "digit3" => ControlKey::Digit3,
                    "y" => ControlKey::Y,
                    "n" => ControlKey::N,
                    _ => ControlKey::Unknown,
                })
            }
        }

        deserializer.deserialize_identifier(ControlKeyVisitor)
    }
}

/// Requests sent as JSON Lines. Nested `data` structs preserve v1's adjacent
/// tag and payload shape; the internal tag allows old holders to ignore an
/// unknown future request and its payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HolderRequest {
    Hello {
        data: Hello,
    },
    /// Starts the agent. A holder runs at most one agent in its life: a second
    /// `Spawn` (for example re-sent by a daemon that restarted between holder
    /// start and `Spawn`) gets `Error{code: "already_spawned"}` and changes
    /// nothing.
    Spawn {
        data: SpawnData,
    },
    Resize {
        data: ResizeData,
    },
    SendControlKey {
        data: SendControlKeyData,
    },
    /// Input from a human using the interactive attach view, encoded as base64
    /// PTY bytes and kept distinct from agent message delivery.
    OperatorTerminalInput {
        data: OperatorTerminalInputData,
    },
    Snapshot,
    Shutdown,
    #[serde(other)]
    Unknown,
}

impl HolderRequest {
    pub fn hello() -> Self {
        Self::Hello {
            data: Hello::new(&SUPPORTED_VERSIONS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnData {
    pub instance_id: String,
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub working_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResizeData {
    pub rows: u16,
    pub columns: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendControlKeyData {
    pub key: ControlKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorTerminalInputData {
    pub bytes_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HolderResponse {
    Hello {
        data: SelectedVersionData,
    },
    Spawned {
        data: SpawnedData,
    },
    ScreenSnapshot {
        data: ScreenSnapshotData,
    },
    /// PTY bytes are base64 text in JSON Lines; the adapter owns encoding.
    PtyBytes {
        data: PtyBytesData,
    },
    Exited {
        data: ExitedData,
    },
    Error {
        data: ErrorData,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedVersionData {
    pub selected: ProtocolVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnedData {
    pub instance_id: String,
    pub process_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenSnapshotData {
    pub screen: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyBytesData {
    pub bytes_base64: String,
}

/// How the agent ended. `code` is `None` when a signal killed it; then
/// `signal` names it (`"SIGKILL"`, or `"SIG<n>"` for an unnamed number).
/// `signal` was added within v1 as an optional field: it is omitted on the
/// wire when absent, and a missing field reads as `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitedData {
    pub code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorData {
    pub code: String,
    pub message: String,
}

pub fn negotiate_version(remote: &Hello) -> Result<ProtocolVersion, VersionMismatch> {
    negotiate("holder", &SUPPORTED_VERSIONS, &remote.supported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holder_hello_advertises_v1() {
        let HolderRequest::Hello { data: hello } = HolderRequest::hello() else {
            unreachable!();
        };
        assert_eq!(negotiate_version(&hello), Ok(V1));
    }
}
