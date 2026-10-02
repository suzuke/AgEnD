//! Forward Branch role handoff must retain commits across a queued restart.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::pipeline::workflow::{Stage, WorkOutput, Workflow};
use agend_core::protocol::client::*;
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;

#[test]
fn two_branch_authors_merge_both_contributions_after_a_queued_handoff_restart() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    {
        let store = SqliteStore::open(&lab.home, 0).unwrap();
        block_on(store.join_team("g10", "g10-dev", "alpha")).unwrap();
        block_on(store.join_team("g10", "g10-hold", "spare")).unwrap();
        // The test, rather than the auto-worker, controls the final review receipt.
        block_on(store.join_team("g10", "g10-rev", "idle")).unwrap();
        let mut reviewer = block_on(store.instance("g10-hold")).unwrap().unwrap();
        reviewer.id = "g10-manual-review".into();
        reviewer.working_directory = lab
            .home
            .join("workspace/g10-manual-review")
            .display()
            .to_string();
        reviewer.session_id = Some(agend_daemon::store::instances::new_session_id().unwrap());
        std::fs::create_dir_all(&reviewer.working_directory).unwrap();
        block_on(store.add_instance(&reviewer)).unwrap();
        block_on(store.join_team("g10", "g10-manual-review", "reviewer")).unwrap();
        block_on(store.set_inbox_delivery("g10-manual-review")).unwrap();
        let mut workflow = Workflow::builtin_planned();
        workflow.id = "branch-handoff".into();
        if let Stage::Work { role, output, .. } = &mut workflow.stages[0].stage {
            *role = "alpha".into();
            *output = WorkOutput::Branch;
        }
        if let Stage::Work { role, .. } = &mut workflow.stages[2].stage {
            *role = "beta".into();
        }
        if let Stage::Command { command } = &mut workflow.stages[4].stage {
            *command = "test -f first.txt && test -f second.txt".into();
        }
        assert!(agend_daemon::pipeline::validate(workflow.clone()).is_ok());
        block_on(store.save_workflow(&workflow)).unwrap();
    }
    lab.boot(None).unwrap();
    let CommandResult::TaskCreated { data } = lab
        .operator(OperatorCommand::TaskCreate {
            title: "two branch authors".into(),
            role: "alpha".into(),
            team_id: "g10".into(),
            workflow_id: Some("branch-handoff".into()),
        })
        .unwrap()
    else {
        panic!("not created")
    };
    let task = data.task_id;
    let wt = lab.home.join("worktrees").join(&task);
    std::fs::write(wt.join("first.txt"), "first contribution\n").unwrap();
    common::git(&wt, &["add", "first.txt"]).unwrap();
    common::git(&wt, &["commit", "-m", "First contribution"]).unwrap();
    lab.agent(
        "g10-dev",
        AgentCommand::Done {
            task_id: task.clone(),
            identity: Some(ResultIdentity {
                stage_id: "plan".into(),
                attempt: 1,
            }),
        },
    )
    .unwrap();
    lab.request(
        None,
        ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: "approve-first".into(),
                attention_id: format!("approval:{task}/plan_review/1"),
                action: AttentionAction::Approve,
                note: None,
            },
        },
    )
    .unwrap();
    common::wait_until(&lab, || {
        Ok(lab.fleet()?.tasks.iter().any(|t| {
            t.task_id == task && t.current_stage.as_deref() == Some("work") && t.assignee.is_none()
        }))
    })
    .unwrap();
    lab.stop(true);
    lab.boot(None).unwrap();
    lab.operator(OperatorCommand::TeamJoin {
        team_id: "g10".into(),
        instance_id: "g10-hold".into(),
        role: "beta".into(),
    })
    .unwrap();
    common::wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .tasks
            .iter()
            .any(|t| t.task_id == task && t.assignee.as_deref() == Some("g10-hold")))
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(wt.join("first.txt")).unwrap(),
        "first contribution\n"
    );
    std::fs::write(wt.join("second.txt"), "second contribution\n").unwrap();
    common::git(&wt, &["add", "second.txt"]).unwrap();
    common::git(&wt, &["commit", "-m", "Second contribution"]).unwrap();
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
    lab.wait_stage(&task, "review").unwrap();
    lab.agent(
        "g10-manual-review",
        AgentCommand::ReviewApprove {
            task_id: task.clone(),
            identity: Some(ResultIdentity {
                stage_id: "review".into(),
                attempt: 1,
            }),
        },
    )
    .unwrap();
    lab.wait_stage(&task, "done").unwrap();
    let tree = common::git(&lab.repo(), &["ls-tree", "--name-only", "main"]).unwrap();
    assert!(
        tree.lines().any(|p| p == "first.txt"),
        "first author's commit lost: {tree}"
    );
    assert!(
        tree.lines().any(|p| p == "second.txt"),
        "second author's commit lost: {tree}"
    );
}
