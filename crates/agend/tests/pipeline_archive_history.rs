//! Merge-only resolution and later WIP must survive cancellation with a restorable series.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;

fn writer_git(lab: &common::Lab, wt: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(lab.home.join("bin/git"))
        .current_dir(wt)
        .args(args)
        .env("AGEND_HOME", &lab.home)
        .env("AGEND_INSTANCE", "g10-hold")
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().into())
    } else {
        Err(format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

#[test]
fn cancellation_restores_a_merge_only_resolution_and_following_uncommitted_edits() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab
        .create("g10h", "demo", "archive a merge resolution")
        .unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    let base = common::git(&lab.repo(), &["rev-parse", "main"]).unwrap();
    std::fs::write(wt.join("conflict.txt"), "left parent\n").unwrap();
    writer_git(&lab, &wt, &["add", "conflict.txt"]).unwrap();
    writer_git(&lab, &wt, &["commit", "-m", "left parent"]).unwrap();
    common::git(&lab.repo(), &["checkout", "-b", "operator-side", &base]).unwrap();
    std::fs::write(lab.repo().join("conflict.txt"), "right parent\n").unwrap();
    common::git(&lab.repo(), &["add", "conflict.txt"]).unwrap();
    common::git(&lab.repo(), &["commit", "-m", "right parent"]).unwrap();
    common::git(&lab.repo(), &["checkout", "main"]).unwrap();
    let conflict = writer_git(&lab, &wt, &["merge", "--no-ff", "operator-side"]).unwrap_err();
    assert!(
        conflict.contains("CONFLICT"),
        "merge must reach a real conflict, not a shim refusal: {conflict}"
    );
    let resolution = "MERGE_RESOLUTION_ONLY_R5_742963\n";
    std::fs::write(wt.join("conflict.txt"), resolution).unwrap();
    writer_git(&lab, &wt, &["add", "conflict.txt"]).unwrap();
    writer_git(
        &lab,
        &wt,
        &["commit", "-m", "resolve conflict with unique bytes"],
    )
    .unwrap();
    let head = writer_git(&lab, &wt, &["rev-parse", "HEAD"]).unwrap();
    let parents = writer_git(&lab, &wt, &["show", "-s", "--format=%P", "HEAD"]).unwrap();
    assert_eq!(parents.split_whitespace().count(), 2);
    let final_bytes = format!("{resolution}uncommitted after merge\n");
    std::fs::write(wt.join("conflict.txt"), &final_bytes).unwrap();
    lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: Some("archive merge-only artifact".into()),
    })
    .unwrap();
    lab.wait_stage(&task, "cancelled").unwrap();
    assert!(!wt.exists());
    let patches = std::fs::read_dir(lab.home.join("archive"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(patches.len(), 1);
    let patch = std::fs::read_to_string(&patches[0]).unwrap();
    eprintln!("ARCHIVED HEAD {head}\nPATCH:\n{patch}");
    // The unique merge-resolution bytes have no representation in either parent.
    assert!(
        patch.contains(resolution.trim()),
        "archive omitted the cancelled branch's merge-only resolution before deleting branch and worktree"
    );
    common::git(&lab.repo(), &["apply", patches[0].to_str().unwrap()]).unwrap();
    assert_eq!(
        std::fs::read_to_string(lab.repo().join("conflict.txt")).unwrap(),
        final_bytes
    );
}
