//! The same durable receipt/CAS contract runs against the fake and real SQLite.
use agend_core::{
    model::DeliveryState,
    pipeline::{
        ports::PipelineStore,
        state::{PipelineEvent, PipelineState, step},
        task::{Task, TaskStatus},
        workflow::Workflow,
    },
    policy::busy::BusyLevel,
    runtime_records::NewMessage,
    traits::{CasResult, StoredEvent, TaskProgress},
};
use agend_testkit::{block_on, fakes::FakeStore, tempdir::TempDir};

async fn contract<S: PipelineStore>(store: &S)
where
    S::Error: std::fmt::Debug,
{
    let workflow = Workflow::builtin_research();
    store.save_workflow(&workflow).await.unwrap();
    let mut task = Task::new("t-1", "receipt", "general", "research", 1);
    let state = PipelineState::new("t-1", agend_daemon::pipeline::validate(workflow).unwrap());
    store
        .create_pipeline_task(
            &task,
            &serde_json::to_string(&state.snapshot()).unwrap(),
            100,
        )
        .await
        .unwrap();
    let (next, _) = step(&state, PipelineEvent::Start).unwrap();
    task.status = TaskStatus::Running;
    let progress = TaskProgress {
        pipeline: serde_json::to_string(&next.snapshot()).unwrap(),
        stage_entered_at_unix_ms: 200,
        merge_intent: None,
        block_reason: None,
    };
    let event = StoredEvent {
        id: "accepted".into(),
        occurred_at_unix_ms: 200,
        kind: "pipeline".into(),
        detail: "start".into(),
    };
    for id in ["bad", "good", "conflict"] {
        store
            .claim_message(
                &NewMessage {
                    id: id.into(),
                    from_instance: "daemon".into(),
                    to_instance: "writer".into(),
                    task_id: Some(task.id.clone()),
                    body: id.into(),
                    level: BusyLevel::Queue,
                },
                100,
            )
            .await
            .unwrap();
        store
            .advance_message(id, DeliveryState::Sent, None, 100)
            .await
            .unwrap();
    }
    store
        .advance_message("bad", DeliveryState::Failed, None, 100)
        .await
        .unwrap();
    assert!(
        store
            .advance_pipeline(&task, 1, &progress, &event, Some("bad"))
            .await
            .is_err()
    );
    assert_eq!(store.load_task(&task.id).await.unwrap().unwrap().version, 1);
    assert!(store.load_events(&task.id).await.unwrap().is_empty());
    assert_eq!(
        store
            .load_task_progress(&task.id)
            .await
            .unwrap()
            .unwrap()
            .stage_entered_at_unix_ms,
        100
    );
    assert_eq!(
        store
            .advance_pipeline(&task, 1, &progress, &event, Some("good"))
            .await
            .unwrap(),
        CasResult::Written { new_version: 2 }
    );
    assert_eq!(
        store.message("good").await.unwrap().unwrap().state,
        DeliveryState::Confirmed
    );
    assert_eq!(
        store.load_events(&task.id).await.unwrap(),
        vec![event.clone()]
    );
    assert!(matches!(
        store
            .advance_pipeline(&task, 1, &progress, &event, Some("conflict"))
            .await
            .unwrap(),
        CasResult::Conflict { .. }
    ));
    assert_eq!(
        store.message("conflict").await.unwrap().unwrap().state,
        DeliveryState::Sent
    );
    assert_eq!(store.load_events(&task.id).await.unwrap(), vec![event]);
}
#[test]
fn fake_and_sqlite_advance_and_confirm_in_one_transaction() {
    let fake = FakeStore::new();
    agend_testkit::block_on(fake.add_team(&agend_core::runtime_records::Team {
        id: "general".into(),
        repo: None,
        default_workflow: "research".into(),
    }))
    .unwrap();
    block_on(contract(&fake));
    let home = TempDir::new("g10-ports").unwrap();
    let real = agend_daemon::store::SqliteStore::open(home.path(), 0).unwrap();
    block_on(contract(&real));
}
