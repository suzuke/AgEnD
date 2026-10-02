//! Workflow admission must reject a human quorum this single-operator runtime cannot finish.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::pipeline::workflow::{Approver, Stage, Workflow};
use agend_core::protocol::client::*;
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;

#[test]
fn unsupported_human_quorum_is_rejected_by_check_apply_and_task_create() {
    let mut workflow = Workflow::builtin_planned();
    workflow.id = "human-two".into();
    for stage in &mut workflow.stages {
        if let Stage::Approval {
            by: Approver::Human,
            count,
            ..
        } = &mut stage.stage
        {
            *count = 2;
        }
    }
    // The abstract core can model distinct humans; this runtime has no such identities.
    assert!(
        workflow
            .clone()
            .validated(&["planner".into(), "dev".into(), "reviewer".into()])
            .is_ok()
    );
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    {
        let store = SqliteStore::open(&lab.home, 0).unwrap();
        block_on(store.save_workflow(&workflow)).unwrap();
    }
    lab.boot(None).unwrap();
    let CommandResult::Text { text: definition } = lab
        .operator(OperatorCommand::WorkflowShow {
            workflow_id: "human-two".into(),
        })
        .unwrap()
    else {
        panic!("workflow show did not return its definition")
    };
    for command in [
        OperatorCommand::WorkflowCheck {
            toml: definition.clone(),
        },
        OperatorCommand::WorkflowApply { toml: definition },
        OperatorCommand::TaskCreate {
            title: "cannot finish human quorum".into(),
            role: "planner".into(),
            team_id: "g10".into(),
            workflow_id: Some("human-two".into()),
        },
    ] {
        let refusal = lab.operator(command).unwrap_err();
        assert!(
            refusal.contains("human approval count must be 1"),
            "{refusal}"
        );
    }
    assert!(
        lab.fleet().unwrap().tasks.is_empty(),
        "rejected creation left a task behind"
    );
    assert!(agend_daemon::pipeline::validate(Workflow::builtin_planned()).is_ok());
}
