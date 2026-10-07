//! Production GithubForge with native Git/SQLite and an offline recorded-shape
//! API producer in separate processes. This does not certify live GitHub policy.
use agend_core::{
    github::GithubStore,
    pipeline::task::Task,
    traits::{Forge, MergeRequest, MergeResult, Store, Submission},
};
use agend_daemon::{
    forge::github::{GithubForge, api::Api},
    git::Git,
    runner::ProcessRunner,
    store::SqliteStore,
};
use agend_testkit::tempdir::TempDir;
use std::{
    collections::BTreeMap,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

struct Lab {
    dir: TempDir,
    local: PathBuf,
    real_git: PathBuf,
    runner: ProcessRunner,
}
impl Lab {
    fn git(&self, repo: &Path, args: &[&str]) -> String {
        let out = Command::new(&self.real_git)
            .args(["-c", "core.hooksPath=/dev/null", "-C"])
            .arg(repo)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().into()
    }
    fn new() -> Self {
        let dir = TempDir::new("github-forge-native").unwrap();
        let local = dir.path().join("repo");
        std::fs::create_dir(&local).unwrap();
        let real_git = Git::discover(dir.path()).unwrap().executable;
        let mut env = BTreeMap::from([
            ("PATH".into(), std::env::var("PATH").unwrap()),
            (
                "G12_GITHUB_FIXTURE".into(),
                dir.path().to_str().unwrap().into(),
            ),
            ("GIT_AUTHOR_NAME".into(), "Agend fixture".into()),
            ("GIT_AUTHOR_EMAIL".into(), "fixture@example.invalid".into()),
            ("GIT_COMMITTER_NAME".into(), "Agend fixture".into()),
            (
                "GIT_COMMITTER_EMAIL".into(),
                "fixture@example.invalid".into(),
            ),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ]);
        // Isolate global configuration from the developer's account.
        env.insert("GIT_CONFIG_GLOBAL".into(), "/dev/null".into());
        env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
        let lab = Self {
            dir,
            local,
            real_git,
            runner: ProcessRunner { env },
        };
        lab.git(&lab.local, &["init", "-b", "main"]);
        lab.git(&lab.local, &["config", "user.name", "Agend fixture"]);
        lab.git(
            &lab.local,
            &["config", "user.email", "fixture@example.invalid"],
        );
        lab.git(&lab.local, &["commit", "--allow-empty", "-m", "base"]);
        lab.git(
            lab.dir.path(),
            &["clone", "--bare", lab.local.to_str().unwrap(), "remote"],
        );
        lab.git(
            &lab.local,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/suzuke/AgEnD.git",
            ],
        );
        for (name, source) in [
            ("gh", include_str!("common/github/gh.py")),
            ("git", include_str!("common/github/git.py")),
        ] {
            let path = lab.dir.path().join(name);
            std::fs::write(&path, source).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let repo: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/github/repository.json")).unwrap();
        std::fs::write(
            lab.dir.path().join("config.json"),
            serde_json::to_vec(&serde_json::json!({"git":lab.real_git,"repository":repo})).unwrap(),
        )
        .unwrap();
        std::fs::write(
            lab.dir.path().join("protection-template.json"),
            include_bytes!("fixtures/github/branch-protection.json"),
        )
        .unwrap();
        let bytes = include_bytes!("fixtures/github/pull.http");
        let boundary = bytes.windows(4).position(|s| s == b"\r\n\r\n").unwrap() + 4;
        std::fs::write(lab.dir.path().join("headers.bin"), &bytes[..boundary]).unwrap();
        std::fs::write(
            lab.dir.path().join("pull-template.json"),
            &bytes[boundary..],
        )
        .unwrap();
        lab
    }
    fn forge(&self, store: Arc<SqliteStore>) -> GithubForge {
        GithubForge {
            repo: self.local.clone(),
            store,
            git: Git {
                executable: self.dir.path().join("git"),
                runner: self.runner.clone(),
            },
            api: Api {
                executable: self.dir.path().join("gh"),
                runner: self.runner.clone(),
                directory: self.local.clone(),
            },
        }
    }
    fn branch(&self) -> String {
        let branch = "agend/t-1/native";
        self.git(&self.local, &["checkout", "-b", branch]);
        std::fs::write(self.local.join("work.txt"), "native work\n").unwrap();
        self.git(&self.local, &["add", "work.txt"]);
        self.git(&self.local, &["commit", "-m", "work"]);
        self.git(&self.local, &["checkout", "main"]);
        branch.into()
    }
    fn writes(&self, method: &str) -> usize {
        std::fs::read_to_string(self.dir.path().join("calls.jsonl"))
            .unwrap()
            .lines()
            .filter(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["method"] == method
            })
            .count()
    }
}

#[tokio::test]
async fn production_forge_recovers_lost_create_and_merge_replies_without_duplicate_mutations() {
    let lab = Lab::new();
    let branch = lab.branch();
    let store = Arc::new(SqliteStore::open(&lab.dir.path().join("home"), 0).unwrap());
    store
        .create_task(&Task::new("t-1", "native", "team", "code", 1))
        .await
        .unwrap();
    let forge = lab.forge(store.clone());
    std::fs::write(lab.dir.path().join("lose-create-reply"), "").unwrap();
    let submitted = forge
        .submit(&Submission {
            task_id: "t-1".into(),
            branch: branch.clone(),
            title: "native".into(),
            body: "body".into(),
        })
        .await
        .unwrap();
    assert_eq!(submitted.id.as_deref(), Some("1"));
    assert_eq!(forge.head(&branch).await.unwrap(), submitted.head);
    assert_eq!(lab.writes("POST"), 1);
    drop(forge);
    drop(store);
    let store = Arc::new(SqliteStore::open(&lab.dir.path().join("home"), 1).unwrap());
    assert_eq!(
        store
            .github_change("t-1")
            .await
            .unwrap()
            .unwrap()
            .change
            .pull_number,
        Some(1)
    );
    let forge = lab.forge(store.clone());
    std::fs::write(lab.dir.path().join("lose-merge-reply"), "").unwrap();
    let request = MergeRequest {
        branch: branch.clone(),
        expected_head: submitted.head.clone(),
    };
    let result = forge.merge_if_head_is(&request).await.unwrap();
    let MergeResult::Merged { merge_commit } = result else {
        panic!("not merged")
    };
    assert_eq!(lab.git(&lab.local, &["rev-parse", "main"]), merge_commit);
    assert_eq!(
        lab.git(&lab.local, &["rev-parse", "main^2"]),
        submitted.head
    );
    assert_eq!(lab.writes("PUT"), 1);
    drop(forge);
    drop(store);
    let store = Arc::new(SqliteStore::open(&lab.dir.path().join("home"), 2).unwrap());
    let forge = lab.forge(store);
    assert_eq!(
        forge
            .find_merge("t-1", &request.expected_head)
            .await
            .unwrap(),
        Some((merge_commit.clone(), false))
    );
    assert_eq!(
        forge.merge_if_head_is(&request).await.unwrap(),
        MergeResult::Merged { merge_commit }
    );
    assert_eq!(lab.writes("PUT"), 1);
    assert_eq!(lab.writes("POST"), 1);
    forge.cleanup("t-1", true).await.unwrap();
    assert!(
        forge
            .store
            .github_change("t-1")
            .await
            .unwrap()
            .unwrap()
            .change
            .cleanup
            .complete
    );
    assert!(
        lab.git(
            &lab.dir.path().join("remote"),
            &["for-each-ref", &format!("refs/heads/{}", request.branch)]
        )
        .is_empty()
    );
    assert_eq!(lab.writes("PATCH"), 0);
}

struct ContractFixture {
    forge: GithubForge,
    lab: Lab,
    seq: std::sync::atomic::AtomicUsize,
}
impl ContractFixture {
    fn new() -> Self {
        let lab = Lab::new();
        let store = Arc::new(SqliteStore::open(&lab.dir.path().join("home"), 0).unwrap());
        let forge = lab.forge(store);
        Self {
            lab,
            forge,
            seq: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}
impl agend_testkit::contract::forge::ForgeFixture for ContractFixture {
    type Forge = GithubForge;
    type Error = agend_core::pipeline::ports::ExecutionError;
    fn requires_up_to_date_base(&self) -> bool {
        true
    }
    fn is_stale_base_refusal(&self, error: &Self::Error) -> bool {
        matches!(error, agend_core::pipeline::ports::ExecutionError::Blocked(reason) if reason.contains("HTTP 405"))
    }
    fn forge(&self) -> &GithubForge {
        &self.forge
    }
    fn commit_to(&self, branch: &str) -> String {
        use agend_testkit::block_on;
        let task = agend_core::model::task_id_of_branch(branch).unwrap();
        if block_on(self.forge.store.load_task(task))
            .unwrap()
            .is_none()
        {
            block_on(
                self.forge
                    .store
                    .create_task(&Task::new(task, "contract", "team", "code", 1)),
            )
            .unwrap();
        }
        let exists = Command::new(&self.lab.real_git)
            .arg("-C")
            .arg(&self.lab.local)
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .status()
            .unwrap()
            .success();
        if exists {
            self.lab.git(&self.lab.local, &["checkout", branch]);
        } else {
            self.lab
                .git(&self.lab.local, &["checkout", "-b", branch, "main"]);
        }
        let name = format!(
            "file-{}",
            self.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        );
        std::fs::write(self.lab.local.join(&name), "contract\n").unwrap();
        self.lab.git(&self.lab.local, &["add", &name]);
        self.lab.git(&self.lab.local, &["commit", "-m", &name]);
        let head = self.lab.git(&self.lab.local, &["rev-parse", "HEAD"]);
        self.lab.git(&self.lab.local, &["checkout", "main"]);
        // Before submit the branch is local. Thereafter commits are made
        // visible on the already-owned remote branch, as an external writer.
        if block_on(self.forge.store.github_change(task))
            .unwrap()
            .is_some()
        {
            self.lab.git(
                &self.lab.local,
                &[
                    "push",
                    self.lab.dir.path().join("remote").to_str().unwrap(),
                    &format!("{head}:refs/heads/{branch}"),
                ],
            );
        }
        head
    }
    fn base_head(&self) -> String {
        self.lab
            .git(&self.lab.dir.path().join("remote"), &["rev-parse", "main"])
    }
    fn base_contains(&self, commit: &str) -> bool {
        Command::new(&self.lab.real_git)
            .arg("-C")
            .arg(self.lab.dir.path().join("remote"))
            .args(["merge-base", "--is-ancestor", commit, "main"])
            .output()
            .unwrap()
            .status
            .success()
    }
}

#[test]
fn production_github_forge_satisfies_all_frg_contracts_with_native_git() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let _entered = runtime.enter();
    let report =
        agend_testkit::contract::forge::run("GithubForge/native-offline", ContractFixture::new);
    assert!(report.all_passed(), "{report:#?}");
}

#[tokio::test]
async fn dirty_local_main_does_not_erase_work_or_replay_a_completed_remote_merge() {
    let lab = Lab::new();
    let branch = lab.branch();
    let store = Arc::new(SqliteStore::open(&lab.dir.path().join("home"), 0).unwrap());
    store
        .create_task(&Task::new("t-1", "native", "team", "code", 1))
        .await
        .unwrap();
    let forge = lab.forge(store);
    let submitted = forge
        .submit(&Submission {
            task_id: "t-1".into(),
            branch: branch.clone(),
            title: "native".into(),
            body: String::new(),
        })
        .await
        .unwrap();
    let before = lab.git(&lab.local, &["rev-parse", "main"]);
    std::fs::write(lab.local.join("keep-wip.txt"), "operator work\n").unwrap();
    let request = MergeRequest {
        branch,
        expected_head: submitted.head,
    };
    assert!(forge.merge_if_head_is(&request).await.is_err());
    assert_eq!(lab.git(&lab.local, &["rev-parse", "main"]), before);
    assert_eq!(
        std::fs::read_to_string(lab.local.join("keep-wip.txt")).unwrap(),
        "operator work\n"
    );
    assert_eq!(lab.writes("PUT"), 1);
    // Only the fixture-created WIP is removed to model operator reconciliation.
    std::fs::remove_file(lab.local.join("keep-wip.txt")).unwrap();
    assert!(matches!(
        forge.merge_if_head_is(&request).await.unwrap(),
        MergeResult::Merged { .. }
    ));
    assert_eq!(lab.writes("PUT"), 1);
}

#[tokio::test]
async fn cancelled_pr_cleanup_survives_lost_close_reply_and_does_not_delete_a_recreated_branch() {
    let lab = Lab::new();
    let branch = lab.branch();
    let home = lab.dir.path().join("home");
    let store = Arc::new(SqliteStore::open(&home, 0).unwrap());
    store
        .create_task(&Task::new("t-1", "native", "team", "code", 1))
        .await
        .unwrap();
    let forge = lab.forge(store.clone());
    let submitted = forge
        .submit(&Submission {
            task_id: "t-1".into(),
            branch: branch.clone(),
            title: "native".into(),
            body: String::new(),
        })
        .await
        .unwrap();
    std::fs::write(lab.dir.path().join("lose-close-reply"), "").unwrap();
    std::fs::write(lab.dir.path().join("lose-delete-reply"), "").unwrap();
    forge.cleanup("t-1", false).await.unwrap();
    assert!(
        store
            .github_change("t-1")
            .await
            .unwrap()
            .unwrap()
            .change
            .cleanup
            .complete
    );
    assert_eq!(lab.writes("PATCH"), 1);
    let bare = lab.dir.path().join("remote");
    assert!(
        lab.git(&bare, &["for-each-ref", &format!("refs/heads/{branch}")])
            .is_empty()
    );
    drop(forge);
    drop(store);
    // Simulate somebody creating the same name after our confirmed cleanup.
    lab.git(
        &bare,
        &[
            "update-ref",
            &format!("refs/heads/{branch}"),
            &submitted.head,
        ],
    );
    let store = Arc::new(SqliteStore::open(&home, 1).unwrap());
    lab.forge(store).cleanup("t-1", false).await.unwrap();
    assert_eq!(
        lab.git(&bare, &["rev-parse", &format!("refs/heads/{branch}")]),
        submitted.head
    );
    assert_eq!(lab.writes("PATCH"), 1);
}

#[tokio::test]
async fn cleanup_preserves_a_foreign_branch_update_before_closing_the_pr() {
    let lab = Lab::new();
    let branch = lab.branch();
    let store = Arc::new(SqliteStore::open(&lab.dir.path().join("home"), 0).unwrap());
    store
        .create_task(&Task::new("t-1", "native", "team", "code", 1))
        .await
        .unwrap();
    let forge = lab.forge(store.clone());
    forge
        .submit(&Submission {
            task_id: "t-1".into(),
            branch: branch.clone(),
            title: "native".into(),
            body: String::new(),
        })
        .await
        .unwrap();
    lab.git(&lab.local, &["checkout", &branch]);
    lab.git(
        &lab.local,
        &["commit", "--allow-empty", "-m", "foreign work"],
    );
    let foreign = lab.git(&lab.local, &["rev-parse", "HEAD"]);
    let bare = lab.dir.path().join("remote");
    lab.git(
        &lab.local,
        &[
            "push",
            bare.to_str().unwrap(),
            &format!("{foreign}:refs/heads/{branch}"),
        ],
    );
    lab.git(&lab.local, &["checkout", "main"]);
    assert!(forge.cleanup("t-1", false).await.is_err());
    assert_eq!(lab.writes("PATCH"), 0);
    assert_eq!(
        lab.git(&bare, &["rev-parse", &format!("refs/heads/{branch}")]),
        foreign
    );
    assert!(
        !store
            .github_change("t-1")
            .await
            .unwrap()
            .unwrap()
            .change
            .cleanup
            .complete
    );
}

#[tokio::test]
async fn unknown_delete_does_not_retry_against_a_recreated_identical_head_after_restart() {
    let lab = Lab::new();
    let branch = lab.branch();
    let home = lab.dir.path().join("home");
    let store = Arc::new(SqliteStore::open(&home, 0).unwrap());
    store
        .create_task(&Task::new("t-1", "native", "team", "code", 1))
        .await
        .unwrap();
    let forge = lab.forge(store.clone());
    let submitted = forge
        .submit(&Submission {
            task_id: "t-1".into(),
            branch: branch.clone(),
            title: "native".into(),
            body: String::new(),
        })
        .await
        .unwrap();
    std::fs::write(lab.dir.path().join("recreate-after-delete"), "").unwrap();
    assert!(forge.cleanup("t-1", false).await.is_err());
    assert!(
        store
            .github_change("t-1")
            .await
            .unwrap()
            .unwrap()
            .change
            .cleanup
            .delete_attempted
    );
    drop(forge);
    drop(store);
    let store = Arc::new(SqliteStore::open(&home, 1).unwrap());
    assert!(lab.forge(store).cleanup("t-1", false).await.is_err());
    assert_eq!(
        lab.git(
            &lab.dir.path().join("remote"),
            &["rev-parse", &format!("refs/heads/{branch}")]
        ),
        submitted.head
    );
    assert_eq!(
        std::fs::read_to_string(lab.dir.path().join("delete-calls.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[tokio::test]
async fn weak_protection_blocks_before_intent_and_a_corrected_policy_can_retry() {
    let lab = Lab::new();
    let branch = lab.branch();
    let store = Arc::new(SqliteStore::open(&lab.dir.path().join("home"), 0).unwrap());
    store
        .create_task(&Task::new("t-1", "policy", "team", "code", 1))
        .await
        .unwrap();
    let forge = lab.forge(store.clone());
    let submitted = forge
        .submit(&Submission {
            task_id: "t-1".into(),
            branch: branch.clone(),
            title: "policy".into(),
            body: "body".into(),
        })
        .await
        .unwrap();
    let request = MergeRequest {
        branch,
        expected_head: submitted.head,
    };
    let marker = lab.dir.path().join("weak-protection");
    std::fs::write(&marker, "").unwrap();
    assert!(
        forge
            .merge_if_head_is(&request)
            .await
            .unwrap_err()
            .to_string()
            .contains("strict")
    );
    assert_eq!(lab.writes("PUT"), 0);
    assert!(
        store
            .github_change("t-1")
            .await
            .unwrap()
            .unwrap()
            .change
            .merge_head
            .is_none()
    );
    std::fs::remove_file(marker).unwrap();
    assert!(matches!(
        forge.merge_if_head_is(&request).await.unwrap(),
        MergeResult::Merged { .. }
    ));
    assert_eq!(lab.writes("PUT"), 1);
    // A later policy change cannot prevent read-only receipt recovery.
    std::fs::write(lab.dir.path().join("weak-protection"), "").unwrap();
    assert!(matches!(
        forge.merge_if_head_is(&request).await.unwrap(),
        MergeResult::Merged { .. }
    ));
    assert_eq!(lab.writes("PUT"), 1);
}

#[tokio::test]
async fn strict_server_rejects_a_base_move_after_the_last_policy_read() {
    let lab = Lab::new();
    let branch = lab.branch();
    let store = Arc::new(SqliteStore::open(&lab.dir.path().join("home"), 0).unwrap());
    store
        .create_task(&Task::new("t-1", "race", "team", "code", 1))
        .await
        .unwrap();
    let forge = lab.forge(store.clone());
    let submitted = forge
        .submit(&Submission {
            task_id: "t-1".into(),
            branch: branch.clone(),
            title: "race".into(),
            body: "body".into(),
        })
        .await
        .unwrap();
    let request = MergeRequest {
        branch,
        expected_head: submitted.head,
    };
    std::fs::write(lab.dir.path().join("advance-base-before-put"), "").unwrap();
    assert!(forge.merge_if_head_is(&request).await.is_err());
    assert_eq!(lab.writes("PUT"), 1);
    assert_eq!(
        store
            .github_change("t-1")
            .await
            .unwrap()
            .unwrap()
            .change
            .merge_head
            .as_deref(),
        Some(request.expected_head.as_str())
    );
    assert!(forge.merge_if_head_is(&request).await.is_err());
    assert_eq!(lab.writes("PUT"), 1);
    let pulls: serde_json::Value =
        serde_json::from_slice(&std::fs::read(lab.dir.path().join("pull-state.json")).unwrap())
            .unwrap();
    assert_eq!(pulls[0]["merged"], false);
}
