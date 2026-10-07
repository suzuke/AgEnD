//! Whole-queue tests inject all five contract fakes; no git, files or processes.
use super::*;
use agend_core::{
    model::{Backend, DeliveryState},
    traits::{Forge, Store},
};
use agend_testkit::fakes::{
    DriverCall, FakeClock, FakeDriver, FakePipelineExecutor, FakeStore, ForgeCall, ScriptedCommand,
};
use std::time::Duration;
struct Lab {
    store: Arc<FakeStore>,
    driver: FakeDriver,
    executor: FakePipelineExecutor,
    clock: FakeClock,
    fleet: Arc<crate::fleet::Fleet>,
    handle: Handle,
    worker: tokio::task::JoinHandle<()>,
}
impl Lab {
    async fn new() -> Self {
        let store = Arc::new(FakeStore::new());
        store
            .add_team(&Team {
                id: "general".into(),
                repo: None,
                default_workflow: "research".into(),
            })
            .await
            .unwrap();
        store
            .add_team(&Team {
                id: "code-team".into(),
                repo: Some("fake-repo".into()),
                default_workflow: "code".into(),
            })
            .await
            .unwrap();
        let driver = FakeDriver::new();
        for (id, role) in [("writer", "dev"), ("reviewer", "reviewer")] {
            store.insert_instance(
                Instance {
                    id: id.into(),
                    backend: Backend::Codex,
                    program: "fake".into(),
                    args: Vec::new(),
                    working_directory: "fake-workspace".into(),
                    session_id: None,
                    status: InstanceStatus::Running,
                    session_started: true,
                    agent_pid: None,
                    legacy_no_thread: false,
                    delivery: "push".into(),
                },
                Member {
                    id: id.into(),
                    team: "code-team".into(),
                    role: role.into(),
                    delivery: "push".into(),
                },
            );
            driver.add_instance(id);
        }
        let executor = FakePipelineExecutor::new(store.clone());
        executor.runner.on(
            "--version",
            ScriptedCommand::exits(0).stdout(b"git version 2.43.0"),
        );
        executor.runner.on("cargo test", ScriptedCommand::exits(0));
        let clock = FakeClock::new(1_000);
        let fleet = Arc::new(crate::fleet::Fleet::new(1_000));
        let (handle, worker) = start_with(
            Path::new("fake-home"),
            store.clone(),
            fleet.clone(),
            driver.clone(),
            executor.clone(),
            clock.clone(),
        )
        .await
        .unwrap();
        Self {
            store,
            driver,
            executor,
            clock,
            fleet,
            handle,
            worker,
        }
    }
    async fn create(&self) -> String {
        let CommandResult::TaskCreated { data } = self
            .handle
            .operator(OperatorCommand::TaskCreate {
                title: "fake queue".into(),
                role: "dev".into(),
                team_id: "code-team".into(),
                workflow_id: None,
            })
            .await
            .unwrap()
        else {
            panic!("not created")
        };
        data.task_id
    }
    async fn stage(&self, id: &str, stage: &str) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if self
                    .fleet
                    .view()
                    .tasks
                    .iter()
                    .any(|t| t.task_id == id && t.current_stage.as_deref() == Some(stage))
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    async fn done(&self, id: &str, attempt: u32) -> Reply {
        self.handle
            .agent(
                Some("writer".into()),
                AgentCommand::Done {
                    task_id: id.into(),
                    identity: Some(ResultIdentity {
                        stage_id: "work".into(),
                        attempt,
                    }),
                },
            )
            .await
    }
}
impl Drop for Lab {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

#[tokio::test]
async fn queue_uses_all_fakes_and_commits_before_effects_then_merges_once() {
    let lab = Lab::new().await;
    let task = lab.create().await;
    let binding = lab
        .store
        .bindings()
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.task == task)
        .unwrap();
    let branch = binding.branch.unwrap();
    lab.executor.forge.push(&branch);
    assert_eq!(
        lab.store
            .message(&format!("dispatch:{task}/work/1"))
            .await
            .unwrap()
            .unwrap()
            .state,
        DeliveryState::Sent
    );
    lab.clock.advance(50);
    lab.done(&task, 1).await.unwrap();
    lab.stage(&task, "review").await;
    assert!(
        lab.store
            .events(&task)
            .iter()
            .any(|e| e.occurred_at_unix_ms == 1050)
    );
    lab.handle
        .agent(
            Some("reviewer".into()),
            AgentCommand::ReviewApprove {
                task_id: task.clone(),
                identity: Some(ResultIdentity {
                    stage_id: "review".into(),
                    attempt: 1,
                }),
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if lab
                .store
                .load_task(&task)
                .await
                .unwrap()
                .unwrap()
                .task
                .status
                == TaskStatus::Done
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(lab.executor.forge.merges().len(), 1);
    assert_eq!(
        lab.executor
            .runner
            .calls()
            .iter()
            .filter(|c| c.command == "cargo test")
            .count(),
        1
    );
    assert_eq!(
        lab.driver
            .calls()
            .iter()
            .filter(|c| matches!(c, DriverCall::Deliver { .. }))
            .count(),
        2
    );
    assert!(lab.store.bindings().await.unwrap().is_empty());
    assert!(lab.executor.projection("writer").is_none());
}

#[tokio::test]
async fn failed_commit_and_stale_result_do_not_execute_the_next_stage() {
    let lab = Lab::new().await;
    let task = lab.create().await;
    let b = lab.store.bindings().await.unwrap().pop().unwrap();
    lab.executor.forge.push(b.branch.as_deref().unwrap());
    // Record the observed head first so the failing CAS is the work result.
    let head = lab
        .executor
        .forge
        .head(b.branch.as_deref().unwrap())
        .await
        .unwrap();
    lab.handle
        .tx
        .send(Input::Event(
            task.clone(),
            PipelineEvent::CommitCreated {
                head: head.clone(),
                patch_id: format!("patch:{head}"),
            },
        ))
        .unwrap();
    let _ = lab
        .handle
        .agent(Some("writer".into()), AgentCommand::Status)
        .await
        .unwrap();
    let effects = lab.executor.effects();
    let calls = lab
        .executor
        .forge
        .calls()
        .iter()
        .filter(|c| matches!(c, ForgeCall::Submit(_) | ForgeCall::MergeIfHeadIs(_)))
        .count();
    lab.store.fail_next("advance_task", "disk full");
    assert!(lab.done(&task, 1).await.is_err());
    assert_eq!(
        lab.executor
            .effects()
            .iter()
            .filter(|e| !e.starts_with("git:"))
            .collect::<Vec<_>>(),
        effects
            .iter()
            .filter(|e| !e.starts_with("git:"))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        lab.executor
            .forge
            .calls()
            .iter()
            .filter(|c| matches!(c, ForgeCall::Submit(_) | ForgeCall::MergeIfHeadIs(_)))
            .count(),
        calls
    );
    let events = lab.store.events(&task).len();
    assert!(lab.done(&task, 99).await.is_err());
    assert_eq!(lab.store.events(&task).len(), events);
}

#[tokio::test]
async fn check_failure_reworks_original_holder_and_rejects_old_attempt() {
    let lab = Lab::new().await;
    lab.executor
        .runner
        .on("cargo test", ScriptedCommand::exits(1));
    // Replace the initial success response: consume it before creating work.
    use agend_core::traits::Runner;
    lab.executor
        .runner
        .run("cargo test", "fake-repo", 100)
        .await
        .unwrap();
    let task = lab.create().await;
    let b = lab.store.bindings().await.unwrap().pop().unwrap();
    lab.executor.forge.push(b.branch.as_deref().unwrap());
    lab.done(&task, 1).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if lab
                .store
                .message(&format!("dispatch:{task}/work/2"))
                .await
                .unwrap()
                .is_some()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        lab.store
            .load_task(&task)
            .await
            .unwrap()
            .unwrap()
            .task
            .assignee
            .as_deref(),
        Some("writer")
    );
    assert!(lab.done(&task, 1).await.is_err());
    assert_eq!(lab.executor.forge.merges().len(), 0);
}

#[tokio::test]
async fn one_failed_boot_dispatch_does_not_prevent_other_tasks_from_starting() {
    let lab = Lab::new().await;
    lab.worker.abort();
    for id in ["t-1", "t-2"] {
        let task = Task::new(id, id, "code-team", "code", 1).set_requires_repo(true);
        let state = PipelineState::new(id, validate(Workflow::builtin_code()).unwrap());
        lab.store
            .create_pipeline_task(
                &task,
                &serde_json::to_string(&state.snapshot()).unwrap(),
                1000,
            )
            .await
            .unwrap();
    }
    lab.driver.fail_next("deliver", "backend unavailable");
    let (_handle, worker) = start_with(
        Path::new("fake-home"),
        lab.store.clone(),
        lab.fleet.clone(),
        lab.driver.clone(),
        lab.executor.clone(),
        lab.clock.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        lab.store
            .load_task("t-1")
            .await
            .unwrap()
            .unwrap()
            .task
            .status,
        TaskStatus::Failed
    );
    assert_eq!(
        lab.store
            .load_task("t-2")
            .await
            .unwrap()
            .unwrap()
            .task
            .status,
        TaskStatus::Running
    );
    assert!(
        lab.store
            .message("dispatch:t-2/work/1")
            .await
            .unwrap()
            .is_some()
    );
    let failed = lab.store.load_task("t-1").await.unwrap().unwrap().task;
    assert!(failed.assignee.is_none());
    let bindings = lab.store.bindings().await.unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].task, "t-2");
    assert_eq!(bindings[0].instance, "writer");
    worker.abort();
}

#[tokio::test]
async fn failed_human_approval_commit_keeps_attention_and_publishes_no_resolution() {
    use agend_core::pipeline::workflow::WorkflowStage;
    let lab = Lab::new().await;
    let mut workflow = Workflow::builtin_code();
    workflow.id = "human-code".into();
    workflow.stages.insert(
        workflow.stages.len() - 1,
        WorkflowStage::new(
            "approve",
            Stage::Approval {
                by: Approver::Human,
                count: 1,
                bind_head: true,
            },
        ),
    );
    lab.store.save_workflow(&workflow).await.unwrap();
    let CommandResult::TaskCreated { data } = lab
        .handle
        .operator(OperatorCommand::TaskCreate {
            title: "atomic human approval".into(),
            role: "dev".into(),
            team_id: "code-team".into(),
            workflow_id: Some("human-code".into()),
        })
        .await
        .unwrap()
    else {
        panic!("not created")
    };
    let task = data.task_id;
    let binding = lab
        .store
        .bindings()
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.task == task)
        .unwrap();
    lab.executor.forge.push(binding.branch.as_deref().unwrap());
    lab.done(&task, 1).await.unwrap();
    lab.stage(&task, "review").await;
    lab.handle
        .agent(
            Some("reviewer".into()),
            AgentCommand::ReviewApprove {
                task_id: task.clone(),
                identity: Some(ResultIdentity {
                    stage_id: "review".into(),
                    attempt: 1,
                }),
            },
        )
        .await
        .unwrap();
    lab.stage(&task, "approve").await;
    let id = format!("approval:{task}/approve/1");
    let before = lab.fleet.attention(&id).unwrap();
    let mut subscription = lab
        .fleet
        .subscribe(Some(lab.fleet.view().as_of_event_id))
        .unwrap();
    lab.store.fail_next("advance_task", "disk full");
    assert!(
        lab.handle
            .resolve(ResolveAttentionData {
                request_id: "failed-approval".into(),
                attention_id: id.clone(),
                action: AttentionAction::Approve,
                note: None,
            })
            .await
            .is_err()
    );
    // Drain the serialized queue before observing the resulting view/events.
    lab.handle
        .agent(Some("writer".into()), AgentCommand::Status)
        .await
        .unwrap();
    assert_eq!(lab.fleet.attention(&id), Some(before));
    while let Ok(event) = subscription.live.try_recv() {
        assert!(!matches!(
            event.event,
            DaemonEvent::AttentionResolved { .. }
        ));
    }
    assert!(lab.executor.forge.merges().is_empty());
}

#[tokio::test]
async fn github_workflow_selects_its_forge_for_submit_checks_and_merge() {
    let lab = Lab::new().await;
    let mut workflow = Workflow::builtin_code();
    workflow.version = 2;
    for stage in &mut workflow.stages {
        if let Stage::Submit { forge } = &mut stage.stage {
            *forge = "github".into();
        }
    }
    lab.store.save_workflow(&workflow).await.unwrap();
    let task = lab.create().await;
    let binding = lab
        .store
        .bindings()
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.task == task)
        .unwrap();
    lab.executor.forge.push(binding.branch.as_deref().unwrap());
    lab.done(&task, 1).await.unwrap();
    lab.stage(&task, "review").await;
    lab.handle
        .agent(
            Some("reviewer".into()),
            AgentCommand::ReviewApprove {
                task_id: task.clone(),
                identity: Some(ResultIdentity {
                    stage_id: "review".into(),
                    attempt: 1,
                }),
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while lab
            .store
            .load_task(&task)
            .await
            .unwrap()
            .unwrap()
            .task
            .status
            != TaskStatus::Done
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let effects = lab.executor.effects();
    assert!(effects.contains(&"find-merge:github".into()));
    assert!(effects.contains(&"prepare-main:github".into()));
    assert!(effects.iter().filter(|s| *s == "forge:github").count() >= 3);
    assert!(!effects.iter().any(|s| s == "forge:local"));
    assert_eq!(lab.executor.forge.merges().len(), 1);
}

#[tokio::test]
async fn remote_cleanup_failure_releases_local_capacity_and_waits_for_operator_retry() {
    let lab = Lab::new().await;
    let task = lab.create().await;
    lab.executor
        .set_remote_cleanup_failure(Some("remote unavailable".into()));
    lab.handle
        .operator(OperatorCommand::TaskCancel {
            task_id: task.clone(),
            reason: None,
        })
        .await
        .unwrap();
    let row = lab.store.load_task(&task).await.unwrap().unwrap();
    assert_eq!(row.task.status, TaskStatus::Cancelled);
    assert!(row.task.assignee.is_none());
    assert!(lab.store.bindings().await.unwrap().is_empty());
    assert!(lab.executor.projection("writer").is_none());
    let id = format!("cleanup-remote:{task}");
    assert!(
        lab.fleet
            .view()
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(id.as_str()))
    );
    let before = lab
        .executor
        .effects()
        .iter()
        .filter(|s| s.starts_with("cleanup-remote:"))
        .count();
    lab.handle.tx.send(Input::Wake).unwrap();
    lab.handle
        .agent(Some("writer".into()), AgentCommand::Status)
        .await
        .unwrap();
    assert_eq!(
        lab.executor
            .effects()
            .iter()
            .filter(|s| s.starts_with("cleanup-remote:"))
            .count(),
        before
    );
    lab.executor.set_remote_cleanup_failure(None);
    lab.handle
        .resolve(ResolveAttentionData {
            attention_id: id.clone(),
            action: AttentionAction::Retry,
            request_id: "cleanup-retry".into(),
            note: None,
        })
        .await
        .unwrap();
    assert!(
        !lab.fleet
            .view()
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(id.as_str()))
    );
    assert!(
        lab.executor
            .effects()
            .iter()
            .filter(|s| s.starts_with("cleanup-remote:"))
            .count()
            > before
    );
}
