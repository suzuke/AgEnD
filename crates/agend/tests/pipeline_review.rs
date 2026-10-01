//! Real-process regressions for multiple reviewers, planned handoff and missing git.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;

use agend_core::pipeline::workflow::Stage;
use agend_core::protocol::client::*;
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;
use std::time::{Duration, Instant};

#[test]
fn two_distinct_role_reviewers_can_complete_one_approval_stage() {
    let mut lab = common::Lab::new(&[]).unwrap();
    {
        let store = SqliteStore::open(&lab.home, 0).unwrap();
        let mut second = block_on(store.instance("g10-rev")).unwrap().unwrap();
        second.id = "g10-rev2".into();
        second.working_directory = lab.home.join("workspace/g10-rev2").display().to_string();
        second.session_id = Some(agend_daemon::store::instances::new_session_id().unwrap());
        std::fs::create_dir_all(&second.working_directory).unwrap();
        block_on(store.add_instance(&second)).unwrap();
        block_on(store.join_team("g10", "g10-rev2", "reviewer")).unwrap();
        block_on(store.set_inbox_delivery("g10-rev2")).unwrap();
        let mut workflow = common::workflow("double-review", "test -f hello.txt");
        if let Stage::Approval { count, .. } = &mut workflow.stages[3].stage {
            *count = 2;
        }
        assert!(agend_daemon::pipeline::validate(workflow.clone()).is_ok());
        block_on(store.save_workflow(&workflow)).unwrap();
    }
    lab.boot(None).unwrap();
    let task = lab.create("g10", "double-review", "two reviewers").unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    let mut completed = false;
    while Instant::now() < until {
        if lab
            .fleet()
            .unwrap()
            .tasks
            .iter()
            .any(|t| t.task_id == task && t.current_stage.as_deref() == Some("approve"))
        {
            completed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let logs = lab.logs();
    eprintln!("{logs}");
    lab.stop(true);
    let restart = lab.boot(None);
    eprintln!("restart result: {restart:?}");
    assert!(
        restart.is_ok(),
        "partial role approval must not prevent the whole daemon from restarting: {restart:?}"
    );
    assert!(
        completed,
        "two eligible distinct reviewers must finish count=2; logs: {logs}"
    );
}

#[test]
fn unavailable_git_pauses_repo_work_without_preventing_daemon_startup() {
    let mut lab = common::Lab::new(&[]).unwrap();
    lab.boot(None).unwrap();
    let _task = lab.create("g10h", "demo", "held work").unwrap();
    lab.stop(false);
    lab.environment.push((
        "PATH".into(),
        lab.agend.parent().unwrap().display().to_string(),
    ));
    let result = lab.boot(None);
    eprintln!("{result:?}");
    assert!(
        result.is_ok(),
        "P5 requires unavailable git to pause repo teams, not make the entire daemon unbootable: {result:?}"
    );
}

#[test]
fn planned_dispatch_instructs_the_plan_result_command() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    {
        let store = SqliteStore::open(&lab.home, 0).unwrap();
        block_on(store.join_team("g10", "g10-dev", "planner")).unwrap();
    }
    lab.boot(None).unwrap();
    let result = lab
        .operator(OperatorCommand::TaskCreate {
            title: "Plan work".into(),
            role: "planner".into(),
            team_id: "g10".into(),
            workflow_id: Some("planned".into()),
        })
        .unwrap();
    let CommandResult::TaskCreated { data } = result else {
        panic!("unexpected result");
    };
    let response = lab
        .agent(
            "g10-dev",
            AgentCommand::Inbox {
                after_message_id: None,
            },
        )
        .unwrap();
    eprintln!("{response:?}");
    let CommandResult::Messages { data: inbox } = response else {
        panic!("unexpected inbox");
    };
    let message = inbox
        .messages
        .iter()
        .find(|m| m.body.starts_with("dispatch "))
        .unwrap();
    assert!(
        message.body.contains("next: agend result"),
        "plan stage instructs wrong result command for {}: {}",
        data.task_id,
        message.body
    );
}

#[test]
fn planned_implementation_goes_to_the_available_dev_role() {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    {
        let store = SqliteStore::open(&lab.home, 0).unwrap();
        block_on(store.join_team("g10", "g10-dev", "planner")).unwrap();
        block_on(store.join_team("g10", "g10-hold", "dev")).unwrap();
    }
    lab.boot(None).unwrap();
    let CommandResult::TaskCreated { data } = lab
        .operator(OperatorCommand::TaskCreate {
            title: "Plan then implement".into(),
            role: "planner".into(),
            team_id: "g10".into(),
            workflow_id: Some("planned".into()),
        })
        .unwrap()
    else {
        panic!("unexpected create");
    };
    let task = data.task_id;
    lab.agent(
        "g10-dev",
        AgentCommand::Result {
            task_id: task.clone(),
            summary: "Implement hello.txt".into(),
            output: None,
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
                request_id: "approve-plan".into(),
                attention_id: format!("approval:{task}/plan_review/1"),
                action: AttentionAction::Approve,
                note: None,
            },
        },
    )
    .unwrap();
    let fleet = lab.wait_stage(&task, "work").unwrap();
    let implementation = fleet.tasks.iter().find(|t| t.task_id == task).unwrap();
    eprintln!("{implementation:?}");
    assert_eq!(
        implementation.assignee.as_deref(),
        Some("g10-hold"),
        "planned workflow specifies dev, but planner still receives implementation"
    );
}
