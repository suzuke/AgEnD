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
mod tests {
    use super::*;
    use agend_core::model::work_branch;

    fn work_snapshot() -> Snapshot {
        Snapshot {
            version: SNAPSHOT_VERSION,
            instance: "dev-1".into(),
            source_repo: Some("/repo".into()),
            protected_refs: vec!["release/*".into()],
            binding: Some(Binding::Work {
                task_id: "t-1".into(),
                branch: work_branch("t-1", "fix"),
                worktree: "/home/worktrees/t-1".into(),
            }),
        }
    }

    #[test]
    fn written_snapshot_reads_back() {
        let snap = work_snapshot();
        let text = serde_json::to_string(&snap).unwrap();
        assert_eq!(parse(&text, "dev-1").unwrap(), snap);
    }

    #[test]
    fn unbound_snapshot_reads_back() {
        let snap = Snapshot {
            binding: None,
            source_repo: None,
            ..work_snapshot()
        };
        let text = serde_json::to_string(&snap).unwrap();
        assert_eq!(parse(&text, "dev-1").unwrap(), snap);
    }

    #[test]
    fn malformed_snapshots_are_rejected() {
        let ok = work_snapshot();
        let cases: Vec<(&str, Snapshot)> = vec![
            (
                "version",
                Snapshot {
                    version: 2,
                    ..ok.clone()
                },
            ),
            (
                "instance",
                Snapshot {
                    instance: "dev-2".into(),
                    ..ok.clone()
                },
            ),
            (
                "requires source_repo",
                Snapshot {
                    source_repo: None,
                    ..ok.clone()
                },
            ),
            (
                "absolute",
                Snapshot {
                    source_repo: Some("repo".into()),
                    ..ok.clone()
                },
            ),
            (
                "not agend/",
                Snapshot {
                    binding: Some(Binding::Work {
                        task_id: "t-1".into(),
                        branch: "feat/x".into(),
                        worktree: "/w".into(),
                    }),
                    ..ok.clone()
                },
            ),
            (
                "not agend/",
                Snapshot {
                    binding: Some(Binding::Work {
                        task_id: "t-1".into(),
                        branch: work_branch("t-2", "x"),
                        worktree: "/w".into(),
                    }),
                    ..ok.clone()
                },
            ),
            (
                "absolute",
                Snapshot {
                    binding: Some(Binding::Review {
                        task_id: "t-1".into(),
                        head: "abc".into(),
                        worktree: "w".into(),
                    }),
                    ..ok.clone()
                },
            ),
            (
                "empty entry",
                Snapshot {
                    protected_refs: vec![" ".into()],
                    ..ok.clone()
                },
            ),
        ];
        for (want, snap) in cases {
            let text = serde_json::to_string(&snap).unwrap();
            let err = parse(&text, "dev-1").unwrap_err();
            assert!(err.contains(want), "{want:?} not in {err:?}");
        }
        for text in ["", "{", "[]", r#"{"version":1}"#] {
            assert!(parse(text, "dev-1").is_err(), "{text:?}");
        }
    }

    #[test]
    fn instance_ids_cannot_escape_the_bindings_dir() {
        for bad in ["", ".", "..", "../x", "a/b", ".hidden", "a\\b"] {
            assert!(!valid_instance(bad), "{bad:?}");
        }
        assert!(valid_instance("dev-1"));
    }

    #[test]
    fn missing_env_and_missing_file_are_errors() {
        assert_eq!(load(None, Some("dev-1")), Err(SnapshotError::NotAnAgent));
        let dir = agend_testkit::tempdir::TempDir::new("binding").unwrap();
        assert!(matches!(
            load(Some(dir.path()), Some("dev-1")),
            Err(SnapshotError::Missing(_))
        ));
        assert!(matches!(
            load(Some(dir.path()), Some("../x")),
            Err(SnapshotError::BadInstance(_))
        ));
    }
}
