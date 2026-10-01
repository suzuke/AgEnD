//! Archive round trips must preserve artifact bytes beyond the diagnostic output cap.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;

fn binary(mut state: u64) -> Vec<u8> {
    (0..6 * 1024 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

#[test]
fn cancellation_archives_large_committed_tracked_and_untracked_binaries_losslessly() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    std::fs::write(lab.repo().join("tracked.bin"), binary(17)).unwrap();
    common::git(&lab.repo(), &["add", "tracked.bin"]).unwrap();
    common::git(&lab.repo(), &["commit", "-m", "Initial tracked binary"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10h", "demo", "large WIP").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    let committed = binary(0xdeadbeef12345678);
    let tracked = binary(0x123456789abcdef);
    let untracked = binary(0x987654321abcdef);
    std::fs::write(wt.join("committed.bin"), &committed).unwrap();
    common::git(&wt, &["add", "committed.bin"]).unwrap();
    common::git(&wt, &["commit", "-m", "Unmerged binary"]).unwrap();
    std::fs::write(wt.join("tracked.bin"), &tracked).unwrap();
    std::fs::write(wt.join("untracked.bin"), &untracked).unwrap();
    lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: Some("archive probe".into()),
    })
    .unwrap();
    lab.wait_stage(&task, "cancelled").unwrap();
    assert!(!wt.exists(), "cleanup did not finish");
    let patches = std::fs::read_dir(lab.home.join("archive"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(patches.len(), 1, "temporary artifact was not cleaned");
    assert!(std::fs::metadata(&patches[0]).unwrap().len() > 5 * 1024 * 1024);
    common::git(&lab.repo(), &["apply", patches[0].to_str().unwrap()]).unwrap();
    for (name, bytes) in [
        ("committed.bin", committed),
        ("tracked.bin", tracked),
        ("untracked.bin", untracked),
    ] {
        let restored = std::fs::read(lab.repo().join(name)).unwrap();
        assert!(
            restored == bytes,
            "archive changed {name}: restored length {}, expected length {}, first mismatch {:?}",
            restored.len(),
            bytes.len(),
            restored.iter().zip(&bytes).position(|(a, b)| a != b)
        );
    }
}

#[test]
fn archive_io_failure_preserves_original_wip_until_a_later_wake_can_finish() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab
        .create("g10h", "demo", "preserve on archive failure")
        .unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    let bytes = binary(0xdeadbeef12345678);
    std::fs::write(wt.join("untracked.bin"), &bytes).unwrap();
    let archive = lab.home.join("archive");
    if archive.exists() {
        std::fs::remove_dir(&archive).unwrap();
    }
    std::fs::write(&archive, "blocked archive directory").unwrap();
    assert!(
        lab.operator(OperatorCommand::TaskCancel {
            task_id: task.clone(),
            reason: Some("archive IO probe".into())
        })
        .is_err()
    );
    // A cleanup execution error is reported as Failed while preserving the binding.
    let fleet = lab.wait_stage(&task, "failed").unwrap();
    assert!(
        fleet
            .tasks
            .iter()
            .any(|t| t.task_id == task && t.assignee.as_deref() == Some("g10-hold"))
    );
    assert_eq!(std::fs::read(wt.join("untracked.bin")).unwrap(), bytes);
    std::fs::remove_file(&archive).unwrap();
    let next = lab.create("g10h", "demo", "wake cleanup").unwrap();
    common::wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .tasks
            .iter()
            .any(|t| t.task_id == next && t.assignee.as_deref() == Some("g10-hold")))
    })
    .unwrap();
    assert!(!wt.exists());
    let patch = std::fs::read_dir(&archive)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    common::git(&lab.repo(), &["apply", patch.to_str().unwrap()]).unwrap();
    assert_eq!(
        std::fs::read(lab.repo().join("untracked.bin")).unwrap(),
        bytes
    );
}

#[test]
fn boot_preserves_foreign_checks_worktrees_and_cleans_only_its_reserved_names() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    let checks = lab.home.join("checks");
    std::fs::create_dir_all(&checks).unwrap();
    let foreign = checks.join("human-scratch");
    common::git(
        &lab.repo(),
        &[
            "worktree",
            "add",
            "-b",
            "human-scratch",
            foreign.to_str().unwrap(),
            "main",
        ],
    )
    .unwrap();
    std::fs::write(foreign.join("human-wip"), "operator data").unwrap();
    let foreign_tmp = checks.join("t-personal.tmp");
    std::fs::create_dir(&foreign_tmp).unwrap();
    std::fs::write(foreign_tmp.join("notes"), "operator cache").unwrap();
    let owned = checks.join("t-999-checks-1-orphan");
    common::git(
        &lab.repo(),
        &[
            "worktree",
            "add",
            "--detach",
            owned.to_str().unwrap(),
            "main",
        ],
    )
    .unwrap();
    std::fs::write(owned.join("orphan-wip"), "daemon orphan data").unwrap();
    let owned_tmp = checks.join("t-999-checks-1-orphan.tmp");
    std::fs::create_dir(&owned_tmp).unwrap();
    lab.boot(None).unwrap();
    assert_eq!(
        std::fs::read_to_string(foreign.join("human-wip")).unwrap(),
        "operator data"
    );
    assert_eq!(
        std::fs::read_to_string(foreign_tmp.join("notes")).unwrap(),
        "operator cache"
    );
    assert!(!owned.exists());
    assert!(!owned_tmp.exists());
    let patch = std::fs::read_dir(lab.home.join("archive"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    common::git(&lab.repo(), &["apply", patch.to_str().unwrap()]).unwrap();
    assert_eq!(
        std::fs::read_to_string(lab.repo().join("orphan-wip")).unwrap(),
        "daemon orphan data"
    );
}
