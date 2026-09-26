//! Unit tests for `binding`.

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
