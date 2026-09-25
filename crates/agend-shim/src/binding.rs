//! Reads the per-agent binding snapshot written by the daemon (D6): which
//! instance this is, the team's source repo, extra protected refs, and the
//! agent's single active binding (work or review), if any. No HMAC.
//!
//! The file is `$AGEND_HOME/bindings/<instance>.json`. A missing, unreadable
//! or malformed file is an error the caller must treat as "refuse mutations";
//! this module never guesses a binding.
//!
//! Must NOT: write the snapshot or read the DB.

use agend_core::model::{BRANCH_NAMESPACE, task_id_of_branch};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

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
    pub source_repo: Option<PathBuf>,
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
        worktree: PathBuf,
    },
    /// Review: a detached review worktree at the reviewed head.
    Review {
        task_id: String,
        head: String,
        worktree: PathBuf,
    },
}

impl Binding {
    pub fn task_id(&self) -> &str {
        match self {
            Binding::Work { task_id, .. } | Binding::Review { task_id, .. } => task_id,
        }
    }

    pub fn worktree(&self) -> &Path {
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

/// Why the snapshot cannot be used. Every variant means "refuse mutations".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    /// `AGEND_HOME` or `AGEND_INSTANCE` is not set (not running as an agent).
    NotAnAgent,
    /// The instance id is empty or not a plain file name.
    BadInstance(String),
    Missing(PathBuf),
    Unreadable(PathBuf, String),
    Malformed(PathBuf, String),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::NotAnAgent => {
                write!(f, "AGEND_HOME and AGEND_INSTANCE are not both set")
            }
            SnapshotError::BadInstance(id) => write!(f, "invalid AGEND_INSTANCE {id:?}"),
            SnapshotError::Missing(p) => write!(f, "binding snapshot {} is missing", p.display()),
            SnapshotError::Unreadable(p, e) => {
                write!(f, "binding snapshot {} is unreadable: {e}", p.display())
            }
            SnapshotError::Malformed(p, e) => {
                write!(f, "binding snapshot {} is malformed: {e}", p.display())
            }
        }
    }
}

/// `$AGEND_HOME/bindings/<instance>.json`.
pub fn snapshot_path(home: &Path, instance: &str) -> PathBuf {
    home.join("bindings").join(format!("{instance}.json"))
}

/// Instance ids become file names; reject anything that could escape the
/// bindings directory.
pub fn valid_instance(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.contains(['/', '\\', '\0'])
        && !id.starts_with('.')
}

/// Loads and validates the snapshot for `instance` under `home`.
pub fn load(home: Option<&Path>, instance: Option<&str>) -> Result<Snapshot, SnapshotError> {
    let (Some(home), Some(instance)) = (home, instance) else {
        return Err(SnapshotError::NotAnAgent);
    };
    if !valid_instance(instance) {
        return Err(SnapshotError::BadInstance(instance.to_string()));
    }
    let path = snapshot_path(home, instance);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SnapshotError::Missing(path));
        }
        Err(e) => return Err(SnapshotError::Unreadable(path, e.to_string())),
    };
    parse(&text, instance).map_err(|e| SnapshotError::Malformed(path, e))
}

/// Parses and validates snapshot text for `instance`.
pub fn parse(text: &str, instance: &str) -> Result<Snapshot, String> {
    let snap: Snapshot = serde_json::from_str(text).map_err(|e| e.to_string())?;
    validate(&snap, instance)?;
    Ok(snap)
}

fn validate(snap: &Snapshot, instance: &str) -> Result<(), String> {
    if snap.version != SNAPSHOT_VERSION {
        return Err(format!(
            "version {} is not supported (this build reads version {SNAPSHOT_VERSION})",
            snap.version
        ));
    }
    if snap.instance != instance {
        return Err(format!(
            "written for instance {:?}, not {instance:?}",
            snap.instance
        ));
    }
    if let Some(repo) = &snap.source_repo
        && !repo.is_absolute()
    {
        return Err("source_repo must be an absolute path".into());
    }
    if snap.protected_refs.iter().any(|r| r.trim().is_empty()) {
        return Err("protected_refs contains an empty entry".into());
    }
    let Some(binding) = &snap.binding else {
        return Ok(());
    };
    if snap.source_repo.is_none() {
        return Err("a binding requires source_repo".into());
    }
    if binding.task_id().is_empty() {
        return Err("binding task_id is empty".into());
    }
    if !binding.worktree().is_absolute() {
        return Err("binding worktree must be an absolute path".into());
    }
    match binding {
        Binding::Work {
            task_id, branch, ..
        } => {
            if task_id_of_branch(branch) != Some(task_id.as_str()) {
                return Err(format!(
                    "work branch {branch:?} is not agend/{task_id}/<slug>"
                ));
            }
        }
        Binding::Review { head, .. } => {
            if head.is_empty() {
                return Err("review head is empty".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
