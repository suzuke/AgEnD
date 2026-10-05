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
    traits::{CasResult, Store, StoredEvent, TaskProgress},
};
use agend_testkit::{block_on, fakes::FakeStore, tempdir::TempDir};

#[test]
fn claude_pipeline_receipts_observe_driver_state_and_task_results_never_replace_ack() {
    use agend_core::{model::Backend, runtime_records::*};
    use agend_daemon::store::SqliteStore;
    const SESSION: &str = "11111111-1111-4111-8111-111111111111";
    const DELIVERY: &str = "22222222-2222-4222-8222-222222222222";
    let dir = TempDir::new("g12-pipeline-ack").unwrap();
    let s = SqliteStore::open(dir.path(), 0).unwrap();
    block_on(async {
        s.add_instance(&Instance {
            id: "writer".into(),
            backend: Backend::Claude,
            program: "claude".into(),
            args: vec![],
            working_directory: dir.path().display().to_string(),
            session_id: Some(SESSION.into()),
            status: InstanceStatus::Running,
            session_started: true,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        })
        .await
        .unwrap();
        let (task, progress, event) = prepare(&s).await;
        let message = "dispatch:t-1/work/1";
        s.claim_message(
            &NewMessage {
                id: message.into(),
                from_instance: "daemon".into(),
                to_instance: "writer".into(),
                task_id: Some(task.id.clone()),
                body: "native dispatch".into(),
                level: BusyLevel::Queue,
            },
            100,
        )
        .await
        .unwrap();
        PipelineStore::advance_message(&s, message, DeliveryState::Queued, None, 101)
            .await
            .unwrap();
        assert!(
            PipelineStore::advance_message(&s, message, DeliveryState::Sent, None, 102)
                .await
                .is_err()
        );
        s.reserve_claude_delivery(
            NewClaudeDelivery {
                message_id: message.into(),
                delivery_id: DELIVERY.into(),
                instance_id: "writer".into(),
                session_id: SESSION.into(),
                route: ClaudeRoute::Channel,
            },
            103,
        )
        .await
        .unwrap();
        s.claude_delivery_written(message, DELIVERY, 104)
            .await
            .unwrap();
        PipelineStore::advance_message(&s, message, DeliveryState::Sent, None, 105)
            .await
            .unwrap();
        assert!(
            PipelineStore::advance_message(&s, message, DeliveryState::Confirmed, None, 106)
                .await
                .is_err()
        );
        s.advance_pipeline(&task, 1, &progress, &event, Some(message))
            .await
            .unwrap();
        assert_eq!(
            s.message(message).await.unwrap().unwrap().state,
            DeliveryState::Sent
        );
        assert!(
            s.claude_delivery(message)
                .await
                .unwrap()
                .unwrap()
                .attempt
                .unwrap()
                .confirmed_at_unix_ms
                .is_none()
        );
        s.acknowledge_claude(
            ClaudeAck {
                message_id: message.into(),
                delivery_id: DELIVERY.into(),
                instance_id: "writer".into(),
                session_id: SESSION.into(),
            },
            107,
        )
        .await
        .unwrap();
        // A Written receipt racing with ACK is an observation of an older
        // state, not permission to regress the stored confirmation.
        PipelineStore::advance_message(&s, message, DeliveryState::Sent, None, 108)
            .await
            .unwrap();
        assert_eq!(
            s.message(message).await.unwrap().unwrap().state,
            DeliveryState::Confirmed
        );
        assert!(
            s.advance_message(message, DeliveryState::Confirmed, None, 109)
                .await
                .is_err()
        );
    });
}

async fn prepare<S: PipelineStore>(store: &S) -> (Task, TaskProgress, StoredEvent)
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
    (task, progress, event)
}

async fn contract<S: PipelineStore>(store: &S)
where
    S::Error: std::fmt::Debug,
{
    let (task, progress, event) = prepare(store).await;
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
    store
        .task_note(&task.id, None, Some("pending retry".into()), true)
        .await
        .unwrap();
    assert!(
        store
            .advance_pipeline(&task, 1, &progress, &event, Some("bad"))
            .await
            .is_err()
    );
    let note = store.progress(&task.id).await.unwrap().unwrap();
    assert_eq!(note.attention_reason.as_deref(), Some("pending retry"));
    assert!(note.acknowledged);
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
    let note = store.progress(&task.id).await.unwrap().unwrap();
    assert!(note.attention_reason.is_none());
    assert!(note.acknowledged);
    assert_eq!(
        store.message("good").await.unwrap().unwrap().state,
        DeliveryState::Confirmed
    );
    assert_eq!(
        store.load_events(&task.id).await.unwrap(),
        vec![event.clone()]
    );
    store
        .task_note(&task.id, None, Some("new retry".into()), true)
        .await
        .unwrap();
    assert!(matches!(
        store
            .advance_pipeline(&task, 1, &progress, &event, Some("conflict"))
            .await
            .unwrap(),
        CasResult::Conflict { .. }
    ));
    let note = store.progress(&task.id).await.unwrap().unwrap();
    assert_eq!(note.attention_reason.as_deref(), Some("new retry"));
    assert!(note.acknowledged);
    assert_eq!(
        store.message("conflict").await.unwrap().unwrap().state,
        DeliveryState::Sent
    );
    assert_eq!(
        store.load_events(&task.id).await.unwrap(),
        vec![event.clone()]
    );
    let mut generic_event = event;
    generic_event.id = "generic".into();
    store
        .advance_task(&task, 2, &progress, &generic_event)
        .await
        .unwrap();
    let note = store.progress(&task.id).await.unwrap().unwrap();
    assert_eq!(note.attention_reason.as_deref(), Some("new retry"));
    assert!(note.acknowledged);
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

#[test]
fn sqlite_attention_clear_failure_rolls_back_version_event_receipt_and_note() {
    use agend_daemon::store::SqliteStore;
    let home = TempDir::new("g10-attention-atomic").unwrap();
    let store = SqliteStore::open(home.path(), 0).unwrap();
    let (task, progress, event) = block_on(prepare(&store));
    block_on(store.task_note(&task.id, None, Some("retry later".into()), true)).unwrap();
    block_on(store.claim_message(
        &NewMessage {
            id: "receipt".into(),
            from_instance: "daemon".into(),
            to_instance: "writer".into(),
            task_id: Some(task.id.clone()),
            body: "work".into(),
            level: BusyLevel::Queue,
        },
        100,
    ))
    .unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(home.path().join("agend.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_attention_clear BEFORE UPDATE OF attention_reason ON tasks BEGIN SELECT RAISE(ABORT, 'attention clear rejected'); END;").unwrap();
    drop(connection);
    let store = SqliteStore::open(home.path(), 0).unwrap();
    let error =
        block_on(store.advance_pipeline(&task, 1, &progress, &event, Some("receipt"))).unwrap_err();
    assert!(
        error.to_string().contains("attention clear rejected"),
        "{error}"
    );
    assert_eq!(
        block_on(store.load_task(&task.id))
            .unwrap()
            .unwrap()
            .version,
        1
    );
    assert!(block_on(store.load_events(&task.id)).unwrap().is_empty());
    let note = block_on(store.progress(&task.id)).unwrap().unwrap();
    assert_eq!(note.attention_reason.as_deref(), Some("retry later"));
    assert_eq!(note.data.stage_entered_at_unix_ms, 100);
    assert!(note.acknowledged);
    assert_eq!(
        block_on(store.message("receipt")).unwrap().unwrap().state,
        DeliveryState::Queued
    );
    drop(store);
    let connection = rusqlite::Connection::open(home.path().join("agend.db")).unwrap();
    connection
        .execute_batch("DROP TRIGGER reject_attention_clear;")
        .unwrap();
    drop(connection);
    let store = SqliteStore::open(home.path(), 0).unwrap();
    assert_eq!(
        block_on(store.advance_pipeline(&task, 1, &progress, &event, Some("receipt"))).unwrap(),
        CasResult::Written { new_version: 2 }
    );
    let note = block_on(store.progress(&task.id)).unwrap().unwrap();
    assert!(note.attention_reason.is_none());
    assert!(note.acknowledged);
    assert_eq!(
        block_on(store.message("receipt")).unwrap().unwrap().state,
        DeliveryState::Confirmed
    );
    assert_eq!(block_on(store.load_events(&task.id)).unwrap(), vec![event]);
}
