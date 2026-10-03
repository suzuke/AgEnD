//! Reconstructing the inspection index must not lose Git's racy-file protection.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;

#[test]
fn matching_stat_data_cannot_hide_modified_bytes_from_the_archive() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10h", "demo", "racy index archive").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    common::git(&wt, &["config", "--worktree", "core.checkStat", "minimal"]).unwrap();
    common::git(&wt, &["config", "--worktree", "core.trustctime", "false"]).unwrap();
    let path = wt.join("racy.txt");
    let stamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    std::fs::write(&path, b"ORIGINAL_RACY_BYTES\n").unwrap();
    std::fs::File::open(&path)
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    common::git(&wt, &["add", "racy.txt"]).unwrap();
    common::git(&wt, &["commit", "-m", "Record cached stat data"]).unwrap();
    let bytes = b"MODIFIED_RACY_BYTES\n";
    assert_eq!(bytes.len(), b"ORIGINAL_RACY_BYTES\n".len());
    std::fs::write(&path, bytes).unwrap();
    std::fs::File::open(&path)
        .unwrap()
        .set_modified(stamp)
        .unwrap();
    let index = common::git(
        &wt,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )
    .unwrap();
    // The original index proves this entry racy. A byte-copy with a fresh mtime
    // incorrectly makes its old cached stat appear safely unchanged.
    std::fs::File::open(&index)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .unwrap();
    let copied = lab.home.join("fresh-copy.index");
    std::fs::copy(&index, &copied).unwrap();
    // Emulate the fresh destination mtime of a Linux byte-copy, even on macOS.
    std::fs::File::open(&copied)
        .unwrap()
        .set_modified(std::time::SystemTime::now())
        .unwrap();
    let hidden = std::process::Command::new("git")
        .current_dir(&wt)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "diff",
            "--binary",
        ])
        .env("GIT_INDEX_FILE", &copied)
        .output()
        .unwrap();
    assert!(hidden.status.success());
    assert!(
        hidden.stdout.is_empty(),
        "fixture did not conceal the changed bytes with a fresh copied index"
    );
    assert!(
        common::git(&wt, &["diff-files", "--binary"])
            .unwrap()
            .contains("MODIFIED_RACY_BYTES")
    );
    lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: Some("preserve racy bytes".into()),
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
    assert!(
        std::fs::read(lab.repo().join("racy.txt")).unwrap() == bytes,
        "archive restored cached bytes instead of physical modified bytes"
    );
}
