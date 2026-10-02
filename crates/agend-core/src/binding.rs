//! Shared binding snapshot data. Paths are UTF-8 strings; adapters resolve them.
use crate::model::BRANCH_NAMESPACE;
use alloc::{format, string::String, vec::Vec};
use serde::{Deserialize, Serialize};

/// Snapshot format version this build reads and writes.
pub const SNAPSHOT_VERSION: u32 = 1;

/// The read-only file the daemon writes for one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    /// Instance id; must match the agent's `AGEND_INSTANCE`.
    pub instance: String,
    /// Canonical checkout of the team's repo (D15: 0 or 1 repo). Required
    /// whenever `binding` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repo: Option<String>,
    /// Protected refs on top of the built-in `main` and `master`: short branch
    /// names (`release`), full refs (`refs/tags/v1`) or a trailing `*` glob.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected_refs: Vec<String>,
    /// The agent's single active binding (D33); `None` = unbound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<Binding>,
}

/// One active binding (docs/architecture/pipeline.md#binding).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Binding {
    /// Work: (instance, task, branch, worktree). `branch` is
    /// `agend/<task_id>/<slug>`.
    Work {
        task_id: String,
        branch: String,
        worktree: String,
    },
    /// Review: a detached review worktree at the reviewed head.
    Review {
        task_id: String,
        head: String,
        worktree: String,
    },
}

impl Binding {
    pub fn task_id(&self) -> &str {
        match self {
            Binding::Work { task_id, .. } | Binding::Review { task_id, .. } => task_id,
        }
    }

    pub fn worktree(&self) -> &str {
        match self {
            Binding::Work { worktree, .. } | Binding::Review { worktree, .. } => worktree,
        }
    }

    /// The bound branch; review bindings are detached and have none.
    pub fn branch(&self) -> Option<&str> {
        match self {
            Binding::Work { branch, .. } => Some(branch),
            Binding::Review { .. } => None,
        }
    }

    /// The branch namespace the agent may write (`agend/<task_id>/`); none
    /// for a review.
    pub fn namespace(&self) -> Option<String> {
        match self {
            Binding::Work { task_id, .. } => Some(format!("{BRANCH_NAMESPACE}{task_id}/")),
            Binding::Review { .. } => None,
        }
    }
}
