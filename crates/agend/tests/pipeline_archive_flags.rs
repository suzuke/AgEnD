//! Real shim concealment flags must not hide physical WIP from cancellation archives.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;

fn probe(flag: &str, split: bool, fail_archive: bool) {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10h", "demo", "concealed WIP").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    if split {
        common::git(&wt, &["update-index", "--split-index"]).unwrap();
    }
    let flagged = std::process::Command::new(lab.home.join("bin/git"))
        .current_dir(&wt)
        .args(["update-index", flag, "README.md"])
        .env("AGEND_HOME", &lab.home)
        .env("AGEND_INSTANCE", "g10-hold")
        .output()
        .unwrap();
    assert!(
        flagged.status.success(),
        "{}",
        String::from_utf8_lossy(&flagged.stderr)
    );
    let bytes = b"UNIQUE_UNCOMMITTED_CONCEALED_DATA\n";
    std::fs::write(wt.join("README.md"), bytes).unwrap();
    assert!(common::git(&wt, &["diff", "--binary"]).unwrap().is_empty());
    let index = common::git(
        &wt,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )
    .unwrap();
    let before = std::fs::read(&index).unwrap();
    let archive = lab.home.join("archive");
    if fail_archive {
        // Fail after private index inspection while producing untracked patches.
        std::fs::create_dir_all(&archive).unwrap();
        // An unreadable untracked file prevents complete patch publication.
        use std::os::unix::fs::PermissionsExt;
        let unreadable = wt.join("unreadable-wip");
        std::fs::write(&unreadable, b"retain me").unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o0)).unwrap();
    }
    let result = lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: Some("archive concealed bytes".into()),
    });
    if fail_archive {
        assert!(result.is_err());
        lab.wait_stage(&task, "failed").unwrap();
        assert!(wt.exists());
        assert_eq!(std::fs::read(wt.join("README.md")).unwrap(), bytes);
        assert_eq!(
            std::fs::read(index).unwrap(),
            before,
            "original index changed on failed archival"
        );
        assert!(
            std::fs::read_dir(archive).unwrap().all(|e| e
                .unwrap()
                .path()
                .extension()
                .is_none_or(|ext| ext != "patch")),
            "incomplete archive was published"
        );
    } else {
        result.unwrap();
        lab.wait_stage(&task, "cancelled").unwrap();
        assert!(!wt.exists());
        let patches = std::fs::read_dir(archive)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(patches.len(), 1);
        common::git(&lab.repo(), &["apply", patches[0].to_str().unwrap()]).unwrap();
        assert_eq!(std::fs::read(lab.repo().join("README.md")).unwrap(), bytes);
    }
}
#[test]
fn skip_worktree_does_not_discard_physical_wip() {
    probe("--skip-worktree", false, false);
}
#[test]
fn assume_unchanged_with_split_index_does_not_discard_physical_wip() {
    probe("--assume-unchanged", true, false);
}
#[test]
fn inspection_failure_preserves_concealed_bytes_and_original_index() {
    probe("--skip-worktree", false, true);
}

#[test]
fn concealed_canonical_changes_block_merge_before_main_moves() {
    let mut lab = common::Lab::new(&[]).unwrap();
    std::fs::write(lab.repo().join("hello.txt"), "initial\n").unwrap();
    common::git(&lab.repo(), &["add", "hello.txt"]).unwrap();
    common::git(
        &lab.repo(),
        &["commit", "-m", "Track the future merge path"],
    )
    .unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10", "demo", "concealed main WIP").unwrap();
    lab.wait_stage(&task, "approve").unwrap();
    common::git(
        &lab.repo(),
        &["update-index", "--skip-worktree", "hello.txt"],
    )
    .unwrap();
    let bytes = b"UNIQUE_OPERATOR_CONCEALED_DATA\n";
    std::fs::write(lab.repo().join("hello.txt"), bytes).unwrap();
    assert!(
        common::git(&lab.repo(), &["status", "--porcelain"])
            .unwrap()
            .is_empty()
    );
    let before = common::git(&lab.repo(), &["rev-parse", "main"]).unwrap();
    lab.approve(&task).unwrap();
    let attention = format!("merge-blocked:{task}");
    common::wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(&attention)))
    })
    .unwrap();
    assert_eq!(
        common::git(&lab.repo(), &["rev-parse", "main"]).unwrap(),
        before
    );
    assert_eq!(std::fs::read(lab.repo().join("hello.txt")).unwrap(), bytes);
}

#[test]
fn concealed_author_changes_are_retained_when_main_requires_rebase() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    lab.boot(None).unwrap();
    lab.operator(OperatorCommand::TeamJoin {
        team_id: "g10h".into(),
        instance_id: "g10-rev".into(),
        role: "reviewer".into(),
    })
    .unwrap();
    let task = lab.create("g10h", "demo", "concealed rebase WIP").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    std::fs::write(wt.join("hello.txt"), "author contribution\n").unwrap();
    common::git(&wt, &["add", "hello.txt"]).unwrap();
    common::git(&wt, &["commit", "-m", "Author contribution"]).unwrap();
    let head = common::git(&wt, &["rev-parse", "HEAD"]).unwrap();
    common::git(&wt, &["update-index", "--assume-unchanged", "README.md"]).unwrap();
    let bytes = b"UNIQUE_AUTHOR_CONCEALED_DATA\n";
    std::fs::write(wt.join("README.md"), bytes).unwrap();
    std::fs::write(lab.repo().join("README.md"), "advanced main content\n").unwrap();
    common::git(&lab.repo(), &["add", "README.md"]).unwrap();
    common::git(
        &lab.repo(),
        &["commit", "-m", "Advance main over concealed path"],
    )
    .unwrap();
    lab.agent(
        "g10-hold",
        AgentCommand::Done {
            task_id: task.clone(),
            identity: Some(ResultIdentity {
                stage_id: "work".into(),
                attempt: 1,
            }),
        },
    )
    .unwrap();
    lab.approve(&task).unwrap();
    lab.wait_stage(&task, "work").unwrap();
    common::wait_until(&lab, || Ok(lab.logs().contains("back to work"))).unwrap();
    assert_eq!(common::git(&wt, &["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(std::fs::read(wt.join("README.md")).unwrap(), bytes);
}
