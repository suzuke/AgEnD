//! Native Claude channel/ACK helpers through a complete Git pipeline.
use super::*;
use agend_core::pipeline::workflow::{
    Approver, Stage, WorkOutput, Workflow, WorkflowRequirement, WorkflowStage,
};
use agend_daemon::store::pipeline::Team;

fn git(repo: &Path, args: &[&str]) -> String {
    git_as(repo, args, None)
}
fn git_as(repo: &Path, args: &[&str], home: Option<&Path>) -> String {
    let mut command = Command::new("git");
    command.current_dir(repo).args(args);
    for (key, _) in
        std::env::vars().filter(|(key, _)| key.starts_with("GIT_") || key.starts_with("AGEND_"))
    {
        command.env_remove(key);
    }
    if let Some(home) = home {
        command
            .env("AGEND_HOME", home)
            .env("AGEND_INSTANCE", "claude");
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().into()
}

fn fleet(home: &Path) -> FleetView {
    let response = operator_request(
        home,
        None,
        ClientRequest::GetFleet {
            data: RequestIdData { request_id: id() },
        },
    );
    let ClientResponse::Fleet { data } = response else {
        panic!("{response:?}")
    };
    data.fleet
}

fn wait_stage(home: &Path, task: &str, stage: &str) -> FleetView {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let view = fleet(home);
        if view.tasks.iter().any(|t| {
            t.task_id == task && (t.current_stage.as_deref() == Some(stage) || t.status == stage)
        }) {
            return view;
        }
        assert!(
            Instant::now() < until,
            "{task} did not reach {stage}: {view:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn accepted(home: &Path, caller: &str, args: &[&str]) {
    let result = Command::new(BIN)
        .args(args)
        .env("AGEND_HOME", home)
        .env("AGEND_INSTANCE", caller)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), "accepted");
}

fn notification(channel: &Channel, message: &str) -> Value {
    loop {
        let value = channel.recv();
        if value["method"] == "notifications/claude/channel" {
            assert_eq!(Channel::receipt(&value).message_id, message);
            return value;
        }
    }
}

#[test]
fn native_claude_work_review_ack_and_single_merge_survive_four_daemons() {
    let mut f = Fixture::new(0);
    let repo = f.home.join("repository");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.name", "Native Claude fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(repo.join("README.md"), "Native fixture\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "Initial fixture"]);
    let initial = git(&repo, &["rev-parse", "main"]);
    {
        let store = f.store();
        let mut reviewer = block_on(store.instance("claude")).unwrap().unwrap();
        reviewer.id = "reviewer".into();
        reviewer.session_id = Some(OTHER.into());
        let workspace = f.home.join("workspace/reviewer");
        fs::create_dir_all(&workspace).unwrap();
        reviewer.working_directory = workspace.display().to_string();
        block_on(store.add_instance(&reviewer)).unwrap();
        block_on(store.add_team(&Team {
            id: "native".into(),
            repo: Some(repo.display().to_string()),
            default_workflow: "native".into(),
        }))
        .unwrap();
        block_on(store.join_team("native", "claude", "dev")).unwrap();
        block_on(store.join_team("native", "reviewer", "reviewer")).unwrap();
        block_on(store.save_workflow(&Workflow {
            id: "native".into(),
            version: 1,
            requires: vec![WorkflowRequirement::Repo],
            allow_unreviewed: false,
            stages: vec![
                WorkflowStage::new(
                    "work",
                    Stage::Work {
                        role: "dev".into(),
                        instructions: "Commit hello.txt".into(),
                        output: WorkOutput::Branch,
                    },
                ),
                WorkflowStage::new(
                    "submit",
                    Stage::Submit {
                        forge: "local".into(),
                    },
                ),
                WorkflowStage::new(
                    "checks",
                    Stage::Command {
                        command: "test -f hello.txt".into(),
                    },
                ),
                WorkflowStage::new(
                    "review",
                    Stage::Approval {
                        by: Approver::Role("reviewer".into()),
                        count: 1,
                        bind_head: true,
                    },
                ),
                WorkflowStage::new(
                    "approve",
                    Stage::Approval {
                        by: Approver::Human,
                        count: 1,
                        bind_head: true,
                    },
                ),
                WorkflowStage::new("merge", Stage::Merge),
            ],
        }))
        .unwrap();
    }
    f.start();
    Fixture::hook_as(
        &f.home,
        "claude",
        SESSION,
        "SessionStart",
        json!({"source":"startup"}),
    );
    Fixture::hook_as(
        &f.home,
        "reviewer",
        OTHER,
        "SessionStart",
        json!({"source":"startup"}),
    );
    let mut dev = Channel::new(&f.home);
    let mut reviewer = Channel::for_instance(&f.home, "reviewer");
    let response = operator_request(
        &f.home,
        None,
        ClientRequest::Operator {
            data: OperatorData {
                request_id: id(),
                command: OperatorCommand::TaskCreate {
                    title: "Native Claude pipeline".into(),
                    role: "dev".into(),
                    team_id: "native".into(),
                    workflow_id: Some("native".into()),
                },
            },
        },
    );
    let ClientResponse::CommandResult { data } = response else {
        panic!("{response:?}")
    };
    let CommandResult::TaskCreated { data } = data.result else {
        panic!("{data:?}")
    };
    let task = data.task_id;
    let work_id = format!("dispatch:{task}/work/1");
    let work = notification(&dev, &work_id);
    let work_receipt = Channel::receipt(&work);
    assert!(
        work["params"]["content"]
            .as_str()
            .unwrap()
            .contains("Commit hello.txt")
    );
    dev.written_barrier();
    let worktree = f.home.join("worktrees").join(&task);
    fs::write(worktree.join("hello.txt"), "Native Claude work\n").unwrap();
    git(&worktree, &["add", "hello.txt"]);
    git_as(&worktree, &["commit", "-m", "Native work"], Some(&f.home));
    let head = git(&worktree, &["rev-parse", "HEAD"]);
    // Completing the task is deliberately not an explicit ACK.
    accepted(&f.home, "claude", &["done", &format!("{task}/work/1")]);
    let review_id = format!("dispatch:{task}/review/1/reviewer");
    let review = notification(&reviewer, &review_id);
    let review_receipt = Channel::receipt(&review);
    assert!(
        review["params"]["content"]
            .as_str()
            .unwrap()
            .contains(&head)
    );
    reviewer.written_barrier();
    assert_ne!(
        reviewer.ack(std::slice::from_ref(&review_receipt))["result"]["isError"],
        true
    );
    accepted(
        &f.home,
        "reviewer",
        &["review", "approve", &format!("{task}/review/1")],
    );
    let approval = wait_stage(&f.home, &task, "approve")
        .attention
        .into_iter()
        .find(|a| {
            a.task_id.as_deref() == Some(&task) && a.actions.contains(&AttentionAction::Approve)
        })
        .unwrap()
        .attention_id
        .unwrap();
    assert_eq!(git(&repo, &["rev-parse", "main"]), initial);
    dev.close();
    reviewer.close();
    f.stop();
    {
        let store = f.store();
        assert_eq!(
            block_on(store.message(&work_id)).unwrap().unwrap().state,
            DeliveryState::Sent
        );
        assert_eq!(
            block_on(store.message(&review_id)).unwrap().unwrap().state,
            DeliveryState::Confirmed
        );
    }
    // Restart before human approval: no new dispatch or merge is allowed.
    f.start();
    wait_stage(&f.home, &task, "approve");
    let mut dev = Channel::new(&f.home);
    let mut reviewer = Channel::for_instance(&f.home, "reviewer");
    assert!(dev.output.recv_timeout(Duration::from_millis(250)).is_err());
    assert!(
        reviewer
            .output
            .recv_timeout(Duration::from_millis(250))
            .is_err()
    );
    let response = operator_request(
        &f.home,
        None,
        ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: id(),
                attention_id: approval,
                action: AttentionAction::Approve,
                note: None,
            },
        },
    );
    assert!(
        matches!(response, ClientResponse::CommandResult { ref data }
        if data.result == CommandResult::Accepted),
        "{response:?}"
    );
    wait_stage(&f.home, &task, "done");
    // A valid late ACK survives task completion and worktree deletion.
    assert_ne!(
        dev.ack(std::slice::from_ref(&work_receipt))["result"]["isError"],
        true
    );
    dev.close();
    reviewer.close();
    f.stop();
    for _ in 3..=4 {
        f.start();
        wait_stage(&f.home, &task, "done");
        f.stop();
    }
    assert_eq!(
        git(&repo, &["rev-list", "--merges", "--count", "main"]),
        "1"
    );
    assert_eq!(
        git(&repo, &["show", "main:hello.txt"]),
        "Native Claude work"
    );
    assert!(!worktree.exists());
    let store = f.store();
    assert_eq!(block_on(store.messages_to("claude")).unwrap().len(), 1);
    assert_eq!(block_on(store.messages_to("reviewer")).unwrap().len(), 1);
    assert_eq!(
        block_on(store.message(&work_id)).unwrap().unwrap().state,
        DeliveryState::Confirmed
    );
    assert!(block_on(store.bindings()).unwrap().is_empty());
}
