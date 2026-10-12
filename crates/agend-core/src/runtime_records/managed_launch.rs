//! Durable identity reserved before a managed holder spawn; not a liveness claim.
use crate::setup::backend::ImportedBackend;
use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedLaunchIntent {
    pub binding: String,
    pub instance_id: String,
    pub artifact: ImportedBackend,
    pub configured_program: String,
    pub configured_args: Vec<String>,
    pub session_id: Option<String>,
    pub delivery: String,
    pub executable: String,
    pub args: Vec<String>,
    pub working_directory: String,
}
