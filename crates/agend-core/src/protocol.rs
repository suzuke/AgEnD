//! Wire protocol definitions shared by every process (D1, D11).
//!
//! There is exactly one public, versioned client protocol (event stream plus
//! terminal stream) and one versioned holder protocol. Both must stay backward
//! compatible because old holders and old clients keep running across a daemon
//! upgrade. External GUIs consume a JSON schema generated from these types
//! (`xtask`, not implemented yet).
//!
//! Must NOT: perform I/O. Encoding/decoding of in-memory values only.

use alloc::vec::Vec;
use core::fmt;
use serde::{Deserialize, Serialize};

pub mod client;
pub mod holder;

/// Version advertised in a protocol `hello` message. Minor versions only add
/// optional fields; unknown fields are ignored by serde's default behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

/// First message sent on either socket. The receiver chooses a mutually
/// supported major and the newest minor supported by both peers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub supported: Vec<ProtocolVersion>,
}

impl Hello {
    pub fn new(supported: &[ProtocolVersion]) -> Self {
        Self {
            supported: supported.to_vec(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionMismatch {
    pub protocol: &'static str,
    pub local: Vec<ProtocolVersion>,
    pub remote: Vec<ProtocolVersion>,
}

impl VersionMismatch {
    /// Human-readable English message suitable for a client error frame.
    pub fn message(&self) -> alloc::string::String {
        use alloc::format;
        format!(
            "{protocol} protocol version mismatch: local supports {local}, remote supports {remote}",
            protocol = self.protocol,
            local = format_versions(&self.local),
            remote = format_versions(&self.remote),
        )
    }
}

impl fmt::Display for VersionMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl core::error::Error for VersionMismatch {}

/// Finds the highest shared major and minor. Advertising an older holder
/// major lets a new daemon keep communicating with holders that outlive it.
pub fn negotiate(
    protocol: &'static str,
    local: &[ProtocolVersion],
    remote: &[ProtocolVersion],
) -> Result<ProtocolVersion, VersionMismatch> {
    let selected = local
        .iter()
        .flat_map(|ours| {
            remote.iter().filter_map(move |theirs| {
                (ours.major == theirs.major).then_some(ProtocolVersion {
                    major: ours.major,
                    minor: ours.minor.min(theirs.minor),
                })
            })
        })
        .max();
    selected.ok_or_else(|| VersionMismatch {
        protocol,
        local: local.to_vec(),
        remote: remote.to_vec(),
    })
}

fn format_versions(versions: &[ProtocolVersion]) -> alloc::string::String {
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;

    if versions.is_empty() {
        return "none".into();
    }
    let mut values: Vec<String> = versions
        .iter()
        .map(|v| format!("{}.{}", v.major, v.minor))
        .collect();
    values.sort();
    values.dedup();
    values.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_latest_common_minor_and_keeps_old_holder_major() {
        let daemon = [ProtocolVersion::new(1, 3), ProtocolVersion::new(2, 1)];
        let old_holder = [ProtocolVersion::new(1, 2)];
        assert_eq!(
            negotiate("holder", &daemon, &old_holder),
            Ok(ProtocolVersion::new(1, 2))
        );
    }

    #[test]
    fn incompatible_major_has_a_clear_error() {
        let error = negotiate(
            "client",
            &[ProtocolVersion::new(2, 0)],
            &[ProtocolVersion::new(1, 4)],
        )
        .unwrap_err();
        assert_eq!(
            error.message(),
            "client protocol version mismatch: local supports 2.0, remote supports 1.4"
        );
    }
}
