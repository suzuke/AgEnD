//! Preserve nested Git repositories rather than accepting incomplete patches.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;

fn probe(staged_gitlink: bool) {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10h", "demo", "nested local WIP").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    let nested = wt.join("nested");
    std::fs::create_dir(&nested).unwrap();
    common::git(&nested, &["init", "-q"]).unwrap();
    common::git(&nested, &["config", "user.name", "Gate 10 Fixture"]).unwrap();
    common::git(&nested, &["config", "user.email", "gate10@example.invalid"]).unwrap();
    let bytes = b"UNIQUE_NESTED_RAW_WIP\n";
    std::fs::write(nested.join("notes"), bytes).unwrap();
    if staged_gitlink {
        common::git(&nested, &["add", "notes"]).unwrap();
        common::git(&nested, &["commit", "-m", "Inner repository content"]).unwrap();
        common::git(&wt, &["add", "nested"]).unwrap();
        assert!(
            common::git(&wt, &["ls-files", "--stage", "nested"])
                .unwrap()
                .starts_with("160000 ")
        );
    } else {
        assert_eq!(
            common::git(&wt, &["ls-files", "--others", "-z"]).unwrap(),
            "nested/\0"
        );
    }
    let index = common::git(
        &wt,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )
    .unwrap();
    let original_index = std::fs::read(&index).unwrap();
    let nested_head = std::fs::read(nested.join(".git/HEAD")).unwrap();
    let error = lab
        .operator(OperatorCommand::TaskCancel {
            task_id: task.clone(),
            reason: None,
        })
        .unwrap_err();
    assert!(error.contains("nested repository"), "{error}");
    lab.wait_stage(&task, "failed").unwrap();
    assert_eq!(std::fs::read(nested.join("notes")).unwrap(), bytes);
    assert_eq!(
        std::fs::read(nested.join(".git/HEAD")).unwrap(),
        nested_head
    );
    assert_eq!(std::fs::read(index).unwrap(), original_index);
    assert!(
        std::fs::read_dir(lab.home.join("archive"))
            .unwrap()
            .all(|e| e
                .unwrap()
                .path()
                .extension()
                .is_none_or(|ext| ext != "patch"))
    );
}
#[test]
fn an_untracked_nested_repository_retains_its_original_data() {
    probe(false);
}
#[test]
fn a_staged_gitlink_retains_its_repository_and_original_index() {
    probe(true);
}

#[test]
fn regular_untracked_directories_empty_files_and_symlinks_round_trip() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab
        .create("g10h", "demo", "ordinary untracked paths")
        .unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    std::fs::create_dir(wt.join("notes")).unwrap();
    let bytes = b"UNIQUE_ORDINARY_UNTRACKED_FILE\n";
    std::fs::write(wt.join("notes/file"), bytes).unwrap();
    std::fs::write(wt.join("empty"), b"").unwrap();
    std::os::unix::fs::symlink("missing-target", wt.join("link")).unwrap();
    lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: None,
    })
    .unwrap();
    lab.wait_stage(&task, "cancelled").unwrap();
    assert!(!wt.exists());
    let patch = std::fs::read_dir(lab.home.join("archive"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    common::git(&lab.repo(), &["apply", patch.to_str().unwrap()]).unwrap();
    assert_eq!(std::fs::read(lab.repo().join("notes/file")).unwrap(), bytes);
    assert!(std::fs::read(lab.repo().join("empty")).unwrap().is_empty());
    assert_eq!(
        std::fs::read_link(lab.repo().join("link")).unwrap(),
        std::path::Path::new("missing-target")
    );
}
