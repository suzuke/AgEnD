//! Durable operator-requested program transition; process quiescence is IO-owned.
use super::ManagedLaunchIntent;
use crate::setup::backend::ImportedBackend;
use alloc::string::String;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendSwitchPhase {
    Prepared,
    Cancelled,
    /// Target program selected, but native activation is not yet verified.
    Committed,
    Activated,
    /// Original program restored, but its new native launch is not yet verified.
    Restoring,
    RolledBack,
}

impl BackendSwitchPhase {
    pub fn pending(self) -> bool {
        matches!(self, Self::Prepared | Self::Committed | Self::Restoring)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendSwitch {
    pub id: String,
    pub instance_id: String,
    pub previous: ManagedLaunchIntent,
    pub target: ImportedBackend,
    pub target_program: String,
    /// Native session at preparation, including post-spawn discovery.
    pub session_id: Option<String>,
    pub phase: BackendSwitchPhase,
}
