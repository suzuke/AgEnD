//! Repository display diff settings must not replace restorable archive data.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;

fn probe(textconv: bool) {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    if textconv {
        std::fs::write(lab.repo().join(".gitattributes"), "* diff=archive\n").unwrap();
        common::git(&lab.repo(), &["add", ".gitattributes"]).unwrap();
        common::git(&lab.repo(), &["commit", "-m", "Set display attributes"]).unwrap();
    }
    lab.boot(None).unwrap();
    let task = lab
        .create("g10h", "demo", "restorable diff archive")
        .unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    let setting = if textconv {
        "diff.archive.textconv"
    } else {
        "diff.external"
    };
    let configured = std::process::Command::new(lab.home.join("bin/git"))
        .current_dir(&wt)
        .args(["config", "--worktree", setting, "/usr/bin/true"])
        .env("AGEND_HOME", &lab.home)
        .env("AGEND_INSTANCE", "g10-hold")
        .output()
        .unwrap();
    assert!(
        configured.status.success(),
        "{}",
        String::from_utf8_lossy(&configured.stderr)
    );
    std::fs::write(wt.join("README.md"), "STAGED_ARCHIVE_BYTES\n").unwrap();
    common::git(&wt, &["add", "README.md"]).unwrap();
    let working = b"UNSTAGED_ARCHIVE_BYTES\n";
    let untracked = b"UNTRACKED_ARCHIVE_BYTES\n";
    std::fs::write(wt.join("README.md"), working).unwrap();
    std::fs::write(wt.join("untracked.txt"), untracked).unwrap();
    assert!(common::git(&wt, &["diff", "--binary"]).unwrap().is_empty());
    lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: Some("restore display-filtered WIP".into()),
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
    assert!(
        patch.contains("STAGED_ARCHIVE_BYTES"),
        "staged-only bytes missing"
    );
    common::git(&lab.repo(), &["apply", patches[0].to_str().unwrap()]).unwrap();
    assert_eq!(
        std::fs::read(lab.repo().join("README.md")).unwrap(),
        working
    );
    assert_eq!(
        std::fs::read(lab.repo().join("untracked.txt")).unwrap(),
        untracked
    );
}
#[test]
fn external_diff_cannot_suppress_staged_unstaged_or_untracked_archive_bytes() {
    probe(false);
}
#[test]
fn textconv_cannot_suppress_staged_unstaged_or_untracked_archive_bytes() {
    probe(true);
}

#[test]
fn ignored_untracked_data_is_archived_before_worktree_removal() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    std::fs::write(lab.repo().join(".gitignore"), "*.local\n").unwrap();
    common::git(&lab.repo(), &["add", ".gitignore"]).unwrap();
    common::git(&lab.repo(), &["commit", "-m", "Ignore local notes"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10h", "demo", "ignored local notes").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    let bytes = b"UNIQUE_IGNORED_LOCAL_NOTES\n";
    std::fs::write(wt.join("notes.local"), bytes).unwrap();
    assert!(
        common::git(&wt, &["status", "--porcelain"])
            .unwrap()
            .is_empty()
    );
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
    assert_eq!(
        std::fs::read(lab.repo().join("notes.local")).unwrap(),
        bytes
    );
}

#[test]
fn a_content_filter_retains_physical_wip_instead_of_publishing_normalized_data() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    std::fs::write(
        lab.repo().join(".gitattributes"),
        "README.md filter=normalize\n",
    )
    .unwrap();
    common::git(&lab.repo(), &["add", ".gitattributes"]).unwrap();
    common::git(&lab.repo(), &["commit", "-m", "Content filter attributes"]).unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10h", "demo", "content-filtered WIP").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    let out = std::process::Command::new(lab.home.join("bin/git"))
        .current_dir(&wt)
        .args([
            "config",
            "--worktree",
            "filter.normalize.clean",
            "/usr/bin/true",
        ])
        .env("AGEND_HOME", &lab.home)
        .env("AGEND_INSTANCE", "g10-hold")
        .output()
        .unwrap();
    assert!(out.status.success());
    let bytes = b"UNIQUE_RAW_FILTERED_WIP\n";
    std::fs::write(wt.join("README.md"), bytes).unwrap();
    let error = lab
        .operator(OperatorCommand::TaskCancel {
            task_id: task.clone(),
            reason: None,
        })
        .unwrap_err();
    assert!(error.contains("content conversion"), "{error}");
    lab.wait_stage(&task, "failed").unwrap();
    assert!(wt.exists());
    assert_eq!(std::fs::read(wt.join("README.md")).unwrap(), bytes);
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
fn display_diff_configuration_cannot_change_patch_identity() {
    let lab = common::Lab::new(&["--hold"]).unwrap();
    std::fs::write(lab.repo().join(".gitattributes"), "* diff=display\n").unwrap();
    common::git(&lab.repo(), &["add", ".gitattributes"]).unwrap();
    common::git(&lab.repo(), &["commit", "-m", "Diff display attributes"]).unwrap();
    common::git(&lab.repo(), &["switch", "-c", "identity-test"]).unwrap();
    std::fs::write(lab.repo().join("README.md"), "identity contribution\n").unwrap();
    common::git(&lab.repo(), &["add", "README.md"]).unwrap();
    common::git(&lab.repo(), &["commit", "-m", "Identity contribution"]).unwrap();
    let git = agend_daemon::git::Git::discover(&lab.home).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let before = runtime.block_on(git.patch_id(&lab.repo(), "HEAD")).unwrap();
    assert_ne!(before, "empty");
    common::git(&lab.repo(), &["config", "diff.external", "/usr/bin/true"]).unwrap();
    common::git(
        &lab.repo(),
        &["config", "diff.display.textconv", "/usr/bin/true"],
    )
    .unwrap();
    for (key, value) in [
        ("color.ui", "always"),
        ("color.diff", "always"),
        ("diff.noprefix", "true"),
    ] {
        common::git(&lab.repo(), &["config", key, value]).unwrap();
    }
    let after = runtime.block_on(git.patch_id(&lab.repo(), "HEAD")).unwrap();
    assert_eq!(after, before);
}

#[test]
fn a_content_filter_cannot_prove_a_worktree_safe_to_overwrite() {
    let lab = common::Lab::new(&["--hold"]).unwrap();
    std::fs::write(
        lab.repo().join(".gitattributes"),
        "README.md filter=normalize\n",
    )
    .unwrap();
    common::git(
        &lab.repo(),
        &[
            "config",
            "filter.normalize.clean",
            "/usr/bin/printf normalized",
        ],
    )
    .unwrap();
    common::git(&lab.repo(), &["add", ".gitattributes", "README.md"]).unwrap();
    common::git(&lab.repo(), &["commit", "-m", "Store normalized content"]).unwrap();
    std::fs::write(
        lab.repo().join("README.md"),
        "UNIQUE_CONCEALED_PHYSICAL_BYTES\n",
    )
    .unwrap();
    let stored = common::git(&lab.repo(), &["show", "HEAD:README.md"]).unwrap();
    assert_eq!(
        stored, "normalized",
        "content conversion producer was not active"
    );
    assert!(
        common::git(&lab.repo(), &["status", "--porcelain"])
            .unwrap()
            .contains("README.md")
    );
    let git = agend_daemon::git::Git::discover(&lab.home).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(!runtime.block_on(git.clean_worktree(&lab.repo())).unwrap());
}
