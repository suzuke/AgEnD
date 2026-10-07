//! Real daemon/holder/worker processes and native Git; GitHub HTTP is offline.
#[path = "common/pipeline_process.rs"]
mod pipeline;

use agend_core::{github::GithubStore, pipeline::workflow::Stage};
use agend_daemon::{git::Git, store::SqliteStore};
use agend_testkit::block_on;
use std::{os::unix::fs::PermissionsExt, path::Path, process::Command};

fn fixture(lab: &mut pipeline::Lab) {
    let root = lab.home.join("github-fixture");
    std::fs::create_dir(&root).unwrap();
    let git = Git::discover(&lab.home).unwrap().executable;
    let remote = root.join("remote");
    assert!(
        Command::new(&git)
            .args(["clone", "--bare"])
            .arg(lab.repo())
            .arg(&remote)
            .output()
            .unwrap()
            .status
            .success()
    );
    pipeline::git(
        &lab.repo(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/suzuke/AgEnD.git",
        ],
    )
    .unwrap();
    let repository: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/github/repository.json")).unwrap();
    std::fs::write(
        root.join("config.json"),
        serde_json::to_vec(&serde_json::json!({"git":git,"repository":repository})).unwrap(),
    )
    .unwrap();
    let bytes = include_bytes!("fixtures/github/pull.http");
    let boundary = bytes.windows(4).position(|s| s == b"\r\n\r\n").unwrap() + 4;
    std::fs::write(root.join("headers.bin"), &bytes[..boundary]).unwrap();
    std::fs::write(root.join("pull-template.json"), &bytes[boundary..]).unwrap();
    for (name, source) in [
        ("gh", include_str!("common/github/gh.py")),
        ("git", include_str!("common/github/git.py")),
    ] {
        // Fixture location is pinned in the executable, not a production env exception.
        let source = source.replace("root = Path(os.environ['G12_GITHUB_FIXTURE'])", "root = Path(__file__).resolve().parent\nos.environ.update(GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1', GIT_AUTHOR_NAME='Fixture', GIT_AUTHOR_EMAIL='fixture@example.invalid', GIT_COMMITTER_NAME='Fixture', GIT_COMMITTER_EMAIL='fixture@example.invalid')");
        let path = root.join(name);
        std::fs::write(&path, source).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    // Child-only PATH/HOME isolate credentials and select the independent producer.
    lab.environment.push((
        "PATH".into(),
        format!(
            "{}:{}:{}",
            root.display(),
            lab.agend.parent().unwrap().display(),
            std::env::var("PATH").unwrap()
        ),
    ));
    lab.environment
        .push(("HOME".into(), lab.home.to_string_lossy().into()));
    let store = SqliteStore::open(&lab.home, 0).unwrap();
    let mut workflow = pipeline::workflow("github", "test -f hello.txt");
    workflow.stages[1].stage = Stage::Submit {
        forge: "github".into(),
    };
    block_on(store.save_workflow(&workflow)).unwrap();
}

fn mutations(lab: &pipeline::Lab, method: &str) -> usize {
    std::fs::read_to_string(lab.home.join("github-fixture/calls.jsonl"))
        .unwrap()
        .lines()
        .filter(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["method"] == method)
        .count()
}

fn wait_cleanup(lab: &pipeline::Lab, task: &str) {
    pipeline::wait_until(lab, || {
        let fleet = lab.fleet()?;
        let task = fleet.tasks.iter().find(|t| t.task_id == task).unwrap();
        Ok(task.assignee.is_none() && !lab.home.join("worktrees").join(&task.task_id).exists())
    })
    .unwrap();
}

#[test]
fn real_daemon_github_pipeline_recovers_restart_and_cleans_owned_remote_and_wip() {
    let mut lab = pipeline::Lab::new(&["--leave-wip"]).unwrap();
    fixture(&mut lab);
    for name in ["lose-create-reply", "lose-merge-reply", "lose-delete-reply"] {
        std::fs::write(lab.home.join("github-fixture").join(name), "").unwrap();
    }
    lab.boot(None).unwrap();
    let task = lab.create("g10", "github", "native-github").unwrap();
    lab.wait_stage(&task, "approve").unwrap();
    lab.stop(true);
    lab.boot(None).unwrap();
    lab.approve(&task).unwrap();
    lab.wait_stage(&task, "done").unwrap();
    wait_cleanup(&lab, &task);
    assert_eq!(mutations(&lab, "POST"), 1);
    assert_eq!(mutations(&lab, "PUT"), 1);
    assert_eq!(mutations(&lab, "PATCH"), 0);
    lab.stop(true);
    let store = SqliteStore::open(&lab.home, 0).unwrap();
    let change = block_on(store.github_change(&task))
        .unwrap()
        .unwrap()
        .change;
    assert!(change.cleanup.complete);
    let refs = pipeline::git(
        &lab.home.join("github-fixture/remote"),
        &["for-each-ref", "--format=%(refname)", "refs/heads"],
    )
    .unwrap();
    assert_eq!(refs.trim(), "refs/heads/main");
    assert_eq!(
        pipeline::git(&lab.repo(), &["rev-parse", "main^2"]).unwrap(),
        change.pushed_head.unwrap()
    );
    assert!(
        std::fs::read_dir(lab.home.join("archive"))
            .unwrap()
            .any(|e| {
                let e = e.unwrap();
                e.file_name().to_string_lossy().starts_with(&task)
                    && std::fs::read_to_string(e.path())
                        .unwrap()
                        .contains("uncommitted work")
            })
    );
    drop(store);
    lab.stop(true);
    lab.boot(None).unwrap();
    assert_eq!(mutations(&lab, "PUT"), 1);
    assert_eq!(
        std::fs::read_to_string(lab.home.join("github-fixture/delete-calls.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let home = lab.home.clone();
    let repo = lab.repo();
    drop(lab);
    assert!(!Path::new(&home).exists());
    assert!(!repo.exists());
}

#[test]
fn real_daemon_github_reruns_checks_when_main_advances_and_keeps_approvals() {
    let mut lab = pipeline::Lab::new(&[]).unwrap();
    fixture(&mut lab);
    lab.boot(None).unwrap();
    let task = lab.create("g10", "github", "main-advanced").unwrap();
    lab.wait_stage(&task, "approve").unwrap();
    let branch = "agend/t-1/main-advanced";
    let approved = pipeline::git(&lab.repo(), &["rev-parse", branch]).unwrap();
    let remote = lab.home.join("github-fixture/remote");
    pipeline::git(&remote, &["config", "user.name", "Fixture"]).unwrap();
    pipeline::git(
        &remote,
        &["config", "user.email", "fixture@example.invalid"],
    )
    .unwrap();
    let old_base = pipeline::git(&remote, &["rev-parse", "main"]).unwrap();
    let tree = pipeline::git(&remote, &["rev-parse", "main^{tree}"]).unwrap();
    let advanced = pipeline::git(
        &remote,
        &["commit-tree", &tree, "-p", &old_base, "-m", "main advanced"],
    )
    .unwrap();
    pipeline::git(
        &remote,
        &["update-ref", "refs/heads/main", &advanced, &old_base],
    )
    .unwrap();
    lab.approve(&task).unwrap();
    lab.wait_stage(&task, "done").unwrap();
    wait_cleanup(&lab, &task);
    assert_eq!(
        pipeline::git(&lab.repo(), &["rev-parse", "main^1"]).unwrap(),
        advanced
    );
    assert_ne!(
        pipeline::git(&lab.repo(), &["rev-parse", "main^2"]).unwrap(),
        approved
    );
    assert!(
        lab.logs().contains("checks t-1/checks/2 passed"),
        "{}",
        lab.logs()
    );
    assert_eq!(mutations(&lab, "POST"), 1);
    assert_eq!(mutations(&lab, "PUT"), 1);
}

#[test]
fn real_daemon_cancellation_closes_owned_pr_and_archives_wip() {
    use agend_core::protocol::client::OperatorCommand;
    let mut lab = pipeline::Lab::new(&["--leave-wip"]).unwrap();
    fixture(&mut lab);
    std::fs::write(lab.home.join("github-fixture/lose-close-reply"), "").unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10", "github", "cancel-github").unwrap();
    lab.wait_stage(&task, "approve").unwrap();
    lab.operator(OperatorCommand::TaskCancel {
        task_id: task.clone(),
        reason: None,
    })
    .unwrap();
    lab.wait_stage(&task, "cancelled").unwrap();
    wait_cleanup(&lab, &task);
    assert_eq!(mutations(&lab, "PATCH"), 1);
    assert_eq!(mutations(&lab, "PUT"), 0);
    let pulls: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(lab.home.join("github-fixture/pull-state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(pulls[0]["state"], "closed");
    assert_eq!(pulls[0]["merged"], false);
    assert_eq!(
        pipeline::git(
            &lab.home.join("github-fixture/remote"),
            &["for-each-ref", "--format=%(refname)", "refs/heads"]
        )
        .unwrap(),
        "refs/heads/main"
    );
    assert!(
        std::fs::read_dir(lab.home.join("archive"))
            .unwrap()
            .any(|e| std::fs::read_to_string(e.unwrap().path())
                .unwrap()
                .contains("uncommitted work"))
    );
}
