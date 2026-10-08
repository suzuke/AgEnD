//! Durable public-registry observations. No installation or admission authority.
use alloc::string::String;
use serde::{Deserialize, Serialize};

pub const REGISTRY_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryObservation {
    pub backend: String,
    pub attempt: u64,
    pub started_ms: u64,
    pub completed_ms: Option<u64>,
    /// Last successful observation, retained when the next check fails.
    pub latest: Option<super::PublishedBackend>,
    pub error: Option<String>,
    pub revision: u64,
    pub acknowledged_revision: u64,
}
