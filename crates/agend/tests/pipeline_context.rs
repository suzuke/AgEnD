//! Regression probes for approved work products, damaged refs and headless review.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;

#[test]
fn approved_planner_result_is_available_to_the_forward_dev_after_restart() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    {
        let store = SqliteStore::open(&lab.home, 0).unwrap();
        block_on(store.join_team("g10", "g10-dev", "planner")).unwrap();
        block_on(store.join_team("g10", "g10-hold", "dev")).unwrap();
    }
    lab.boot(None).unwrap();
    let CommandResult::TaskCreated { data } = lab
        .operator(OperatorCommand::TaskCreate {
            title: "Implement the approved plan".into(),
            role: "planner".into(),
            team_id: "g10".into(),
            workflow_id: Some("planned".into()),
        })
        .unwrap()
    else {
        panic!("not created")
    };
    let task = data.task_id;
    let plan = "APPROVED_PLAN_R3: write feature_zeta.txt containing exact bytes 7429";
    lab.agent(
        "g10-dev",
        AgentCommand::Result {
            task_id: task.clone(),
            summary: plan.into(),
            output: Some("artifact-r3-result".into()),
            identity: Some(ResultIdentity {
                stage_id: "plan".into(),
                attempt: 1,
            }),
        },
    )
    .unwrap();
    let before = lab.wait_stage(&task, "plan_review").unwrap();
    let recap = before
        .attention
        .iter()
        .find(|a| a.task_id.as_deref() == Some(&task))
        .unwrap()
        .recap
        .as_ref()
        .unwrap();
    assert!(
        recap
            .decisions
            .iter()
            .any(|d| d.contains(plan) && d.contains("artifact-r3-result"))
    );

    lab.stop(true);
    lab.boot(None).unwrap();
    lab.request(
        None,
        ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: "approve-plan".into(),
                attention_id: format!("approval:{task}/plan_review/1"),
                action: AttentionAction::Approve,
                note: None,
            },
        },
    )
    .unwrap();
    lab.wait_stage(&task, "work").unwrap();
    let CommandResult::Messages { data: inbox } = lab
        .agent(
            "g10-hold",
            AgentCommand::Inbox {
                after_message_id: None,
            },
        )
        .unwrap()
    else {
        panic!("no inbox")
    };
    let body = inbox
        .messages
        .iter()
        .find(|m| m.body.starts_with(&format!("dispatch {task}/work/1")))
        .unwrap()
        .body
        .clone();
    let status = lab.agent("g10-hold", AgentCommand::Status).unwrap();
    let visible = format!("{body} {status:?} {:?}", lab.fleet().unwrap());
    eprintln!("FORWARD DEV VISIBLE: {visible}");
    assert!(
        body.contains(plan),
        "approved plan is absent from every new dev-facing projection"
    );
    assert!(
        body.contains("artifact-r3-result"),
        "prior output artifact is missing"
    );
}

#[test]
fn missing_main_during_merge_recovery_does_not_prevent_the_whole_daemon_boot() {
    let mut lab = common::Lab::new(&[]).unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10", "demo", "blocked merge").unwrap();
    lab.wait_stage(&task, "approve").unwrap();
    std::fs::write(lab.repo().join("untracked-blocker"), "dirty canonical").unwrap();
    lab.approve(&task).unwrap();
    common::wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(&format!("merge-blocked:{task}"))))
    })
    .unwrap();
    lab.stop(true);
    common::git(&lab.repo(), &["update-ref", "-d", "refs/heads/main"]).unwrap();
    let result = lab.boot(None);
    eprintln!("MISSING MAIN BOOT: {result:?}");
    assert!(
        result.is_ok(),
        "one invalid repo/task must not make the entire daemon unbootable"
    );
}

#[test]
fn a_result_only_workflow_can_be_role_reviewed_in_a_team_that_has_a_repo() {
    use agend_core::pipeline::workflow::{Approver, Stage, WorkOutput, Workflow, WorkflowStage};
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    {
        let store = SqliteStore::open(&lab.home, 0).unwrap();
        // Hold the reviewer so the test can observe review and reboot before approval.
        block_on(store.join_team("g10", "g10-rev", "idle")).unwrap();
        block_on(store.join_team("g10", "g10-hold", "reviewer")).unwrap();
        let w = Workflow {
            id: "result-role-review".into(),
            version: 1,
            requires: vec![],
            allow_unreviewed: false,
            stages: vec![
                WorkflowStage::new(
                    "work",
                    Stage::Work {
                        role: "dev".into(),
                        instructions: "Report a result".into(),
                        output: WorkOutput::Result,
                    },
                ),
                WorkflowStage::new(
                    "review",
                    Stage::Approval {
                        by: Approver::Role("reviewer".into()),
                        count: 1,
                        bind_head: false,
                    },
                ),
            ],
        };
        assert!(agend_daemon::pipeline::validate(w.clone()).is_ok());
        block_on(store.save_workflow(&w)).unwrap();
    }
    lab.boot(None).unwrap();
    let task = lab
        .create("g10", "result-role-review", "Research result")
        .unwrap();
    let response = lab.agent(
        "g10-dev",
        AgentCommand::Result {
            task_id: task.clone(),
            summary: "Research concluded".into(),
            output: None,
            identity: Some(ResultIdentity {
                stage_id: "work".into(),
                attempt: 1,
            }),
        },
    );
    eprintln!(
        "RESULT ROLE REVIEW RESPONSE: {response:?} LOGS: {}",
        lab.logs()
    );
    assert!(
        response.is_ok(),
        "a supported result-only workflow must reach unbound role review even if its team has a repo"
    );
    lab.wait_stage(&task, "review").unwrap();
    lab.stop(true);
    lab.boot(None).unwrap();
    let CommandResult::Messages { data: inbox } = lab
        .agent(
            "g10-hold",
            AgentCommand::Inbox {
                after_message_id: None,
            },
        )
        .unwrap()
    else {
        panic!("no review inbox")
    };
    assert!(
        inbox
            .messages
            .iter()
            .any(|m| m.body.contains("Research concluded"))
    );
    lab.agent(
        "g10-hold",
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
    lab.stop(false);
    let store = SqliteStore::open(&lab.home, 0).unwrap();
    assert!(block_on(store.bindings()).unwrap().is_empty());
}
