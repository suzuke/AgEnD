//! Artifact patches must remain applicable under repository display preferences.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;

fn probe(setting: &str, value: &str, history: bool) {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    if history {
        common::git(&lab.repo(), &["config", setting, value]).unwrap();
    }
    lab.boot(None).unwrap();
    let task = lab.create("g10h", "demo", "machine readable WIP").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    if !history {
        let configured = std::process::Command::new(lab.home.join("bin/git"))
            .current_dir(&wt)
            .args(["config", "--worktree", setting, value])
            .env("AGEND_HOME", &lab.home)
            .env("AGEND_INSTANCE", "g10-hold")
            .output()
            .unwrap();
        assert!(configured.status.success());
    }
    if history {
        std::fs::write(wt.join("committed.txt"), b"UNMERGED_DISPLAY_BYTES\n").unwrap();
        common::git(&wt, &["add", "committed.txt"]).unwrap();
        common::git(&wt, &["commit", "-m", "Unmerged display fixture"]).unwrap();
    }
    std::fs::write(wt.join("README.md"), b"STAGED_DISPLAY_BYTES\n").unwrap();
    common::git(&wt, &["add", "README.md"]).unwrap();
    let bytes = b"RAW_DISPLAY_BYTES\x1b[32mkeep this original escape\n";
    std::fs::write(wt.join("README.md"), bytes).unwrap();
    std::fs::write(wt.join("untracked.txt"), b"UNTRACKED_DISPLAY_BYTES\n").unwrap();
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
    let applied = common::git(&lab.repo(), &["apply", patch.to_str().unwrap()]);
    eprintln!("setting={setting}={value}; original worktree removed; git apply={applied:?}");
    applied.unwrap();
    assert_eq!(std::fs::read(lab.repo().join("README.md")).unwrap(), bytes);
    assert_eq!(
        std::fs::read(lab.repo().join("untracked.txt")).unwrap(),
        b"UNTRACKED_DISPLAY_BYTES\n"
    );
    if history {
        assert_eq!(
            std::fs::read(lab.repo().join("committed.txt")).unwrap(),
            b"UNMERGED_DISPLAY_BYTES\n"
        );
    }
}
#[test]
fn color_ui_cannot_make_the_archive_unusable_or_remove_original_escapes() {
    probe("color.ui", "always", false);
}
#[test]
fn color_diff_cannot_make_the_archive_unusable() {
    probe("color.diff", "always", false);
}
#[test]
fn no_prefix_preferences_cannot_break_commit_index_or_worktree_archives() {
    probe("diff.noprefix", "true", true);
}
