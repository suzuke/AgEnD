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
    #[serde(default)]
    pub changed_ms: Option<u64>,
    pub revision: u64,
    pub acknowledged_revision: u64,
}

/// A bounded --version observation of an external configured executable.
/// This is not a claim about an already running holder or its loaded image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemBackendVersion {
    pub backend: String,
    pub configured_program: String,
    pub resolved_program: String,
    pub version_output: String,
    pub sha256: String,
}

/// One durable external-program observation per configured instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemVersionObservation {
    pub instance_id: String,
    /// Never reused after instance removal and recreation.
    pub generation: String,
    pub backend: String,
    pub program: String,
    pub working_directory: String,
    pub attempt: u64,
    pub started_ms: u64,
    pub completed_ms: Option<u64>,
    pub latest: Option<SystemBackendVersion>,
    pub error: Option<String>,
    pub changed_ms: Option<u64>,
    pub revision: u64,
    pub acknowledged_revision: u64,
}
