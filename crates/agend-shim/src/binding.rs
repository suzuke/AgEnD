//! Reads the per-agent binding snapshot written by the daemon (D6): which
//! instance this is, the team's source repo, extra protected refs, and the
//! agent's single active binding (work or review), if any. No HMAC.
//!
//! The file is `$AGEND_HOME/bindings/<instance>.json`. A missing, unreadable
//! or malformed file is an error the caller must treat as "refuse mutations";
//! this module never guesses a binding.
//!
//! Must NOT: write the snapshot or read the DB.

use agend_core::model::task_id_of_branch;
use std::fmt;
use std::path::{Path, PathBuf};

pub use agend_core::binding::{Binding, SNAPSHOT_VERSION, Snapshot};

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
        && !Path::new(repo).is_absolute()
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
    if !Path::new(binding.worktree()).is_absolute() {
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
