//! Cancellation must preserve staged-only bytes and retain unresolved index state.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;

#[test]
fn staged_only_large_binary_can_be_restored_to_the_index_after_cancel() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10h", "demo", "staged-only archive").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    let mut random = 0xdeadbeef12345678u64;
    let bytes = (0..6 * 1024 * 1024)
        .map(|_| {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            random as u8
        })
        .collect::<Vec<_>>();
    std::fs::write(wt.join("staged-only.bin"), &bytes).unwrap();
    let added = std::process::Command::new(lab.home.join("bin/git"))
        .current_dir(&wt)
        .args(["add", "staged-only.bin"])
        .env("AGEND_HOME", &lab.home)
        .env("AGEND_INSTANCE", "g10-hold")
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    std::fs::remove_file(wt.join("staged-only.bin")).unwrap();
    assert!(
        common::git(&wt, &["status", "--porcelain"])
            .unwrap()
            .contains("AD staged-only.bin")
    );
    lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: Some("preserve staged-only data".into()),
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
    let text = std::fs::read_to_string(patch).unwrap();
    let (_, staged) = text.split_once("# agend-wip: index\n").unwrap();
    let (index, working) = staged.split_once("# agend-wip: worktree\n").unwrap();
    let index_path = lab.home.join("recover-index.patch");
    let work_path = lab.home.join("recover-worktree.patch");
    std::fs::write(&index_path, index).unwrap();
    std::fs::write(&work_path, working).unwrap();
    common::git(
        &lab.repo(),
        &["apply", "--index", index_path.to_str().unwrap()],
    )
    .unwrap();
    common::git(&lab.repo(), &["apply", work_path.to_str().unwrap()]).unwrap();
    assert!(!lab.repo().join("staged-only.bin").exists());
    assert!(
        common::git(&lab.repo(), &["status", "--porcelain"])
            .unwrap()
            .contains("AD staged-only.bin")
    );
    common::git(&lab.repo(), &["checkout-index", "staged-only.bin"]).unwrap();
    assert_eq!(
        std::fs::read(lab.repo().join("staged-only.bin")).unwrap(),
        bytes
    );
}

#[test]
fn an_unresolved_index_keeps_all_stages_and_the_original_worktree() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10h", "demo", "unresolved index").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    std::fs::write(wt.join("conflict.txt"), "left\n").unwrap();
    common::git(&wt, &["add", "conflict.txt"]).unwrap();
    common::git(&wt, &["commit", "-m", "left"]).unwrap();
    common::git(&lab.repo(), &["switch", "-c", "operator-side", "main"]).unwrap();
    std::fs::write(lab.repo().join("conflict.txt"), "right\n").unwrap();
    common::git(&lab.repo(), &["add", "conflict.txt"]).unwrap();
    common::git(&lab.repo(), &["commit", "-m", "right"]).unwrap();
    common::git(&lab.repo(), &["switch", "main"]).unwrap();
    assert!(common::git(&wt, &["merge", "--no-ff", "operator-side"]).is_err());
    let index = common::git(&wt, &["ls-files", "--unmerged"]).unwrap();
    let working = std::fs::read(wt.join("conflict.txt")).unwrap();
    assert!(!index.is_empty());
    assert!(
        lab.operator(OperatorCommand::TaskCancel {
            task_id: task.clone(),
            reason: Some("preserve conflicting stages".into())
        })
        .is_err()
    );
    lab.wait_stage(&task, "failed").unwrap();
    assert_eq!(
        common::git(&wt, &["ls-files", "--unmerged"]).unwrap(),
        index
    );
    assert_eq!(std::fs::read(wt.join("conflict.txt")).unwrap(), working);
    assert!(
        std::fs::read_dir(lab.home.join("archive"))
            .unwrap()
            .all(|e| e
                .unwrap()
                .path()
                .extension()
                .is_none_or(|ext| ext != "patch")),
        "incomplete archive was published"
    );
}
