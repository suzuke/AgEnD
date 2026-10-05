use super::ClaudeDriver;
use crate::store::{Instance, InstanceStatus, SqliteStore};
use agend_core::{
    model::{Backend, DeliveryState},
    policy::busy::BusyLevel,
    runtime_records::{ClaudeAck, ClaudeRoute, NewClaudeDelivery, NewDriverEvent},
    traits::{AgentMessage, Driver, DriverEventKind},
};
use agend_testkit::tempdir::TempDir;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SESSION: &str = "11111111-1111-4111-8111-111111111111";
const DELIVERY: &str = "22222222-2222-4222-8222-222222222222";

async fn store(home: &std::path::Path, status: InstanceStatus) -> Arc<SqliteStore> {
    let store = Arc::new(SqliteStore::open(home, 1).unwrap());
    store
        .add_instance(&Instance {
            id: "claude".into(),
            backend: Backend::Claude,
            program: "claude".into(),
            args: vec![],
            working_directory: home.display().to_string(),
            session_id: Some(SESSION.into()),
            status,
            session_started: status == InstanceStatus::Running,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        })
        .await
        .unwrap();
    store
}

fn message() -> AgentMessage {
    AgentMessage {
        id: "33333333-3333-4333-8333-333333333333".into(),
        from: "operator".into(),
        task_id: None,
        body: "Full native content: 繁中 é".into(),
    }
}

async fn state_event(store: &SqliteStore, session: &str, busy: bool) {
    store
        .append_driver_event(
            NewDriverEvent {
                id: crate::store::instances::new_session_id().unwrap(),
                instance_id: "claude".into(),
                session_id: session.into(),
                kind: "AgendState".into(),
                payload: serde_json::json!({"busy":busy}).to_string(),
                occurred_at_unix_ms: 1,
                replayed: false,
            },
            1,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn idle_receipt_waits_for_a_real_write_without_confirming_or_reserving_again() {
    let dir = TempDir::new("g12-driver-receipt").unwrap();
    let store = store(dir.path(), InstanceStatus::Running).await;
    state_event(&store, SESSION, false).await;
    let driver = ClaudeDriver::new(store.clone());
    let message = message();
    let writer_store = store.clone();
    let writer_id = message.id.clone();
    let writer = tokio::spawn(async move {
        let end = Instant::now() + Duration::from_secs(2);
        while writer_store.message(&writer_id).await.unwrap().is_none() {
            assert!(Instant::now() < end);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        writer_store
            .reserve_claude_delivery(
                NewClaudeDelivery {
                    message_id: writer_id.clone(),
                    delivery_id: DELIVERY.into(),
                    instance_id: "claude".into(),
                    session_id: SESSION.into(),
                    route: ClaudeRoute::Channel,
                },
                2,
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        writer_store
            .claude_delivery_written(&writer_id, DELIVERY, 3)
            .await
            .unwrap();
    });
    assert_eq!(
        driver
            .deliver("claude", &message, BusyLevel::Queue)
            .await
            .unwrap()
            .state,
        DeliveryState::Sent
    );
    writer.await.unwrap();
    assert_eq!(
        driver
            .deliver("claude", &message, BusyLevel::Queue)
            .await
            .unwrap()
            .state,
        DeliveryState::Sent
    );
    assert_eq!(store.messages_to("claude").await.unwrap().len(), 1);
    let attempt = store
        .claude_delivery(&message.id)
        .await
        .unwrap()
        .unwrap()
        .attempt
        .unwrap();
    assert_eq!(attempt.delivery_id, DELIVERY);
    assert!(attempt.confirmed_at_unix_ms.is_none());
}

#[tokio::test]
async fn stale_idle_without_a_content_writer_expires_queued_and_never_starts_an_attempt() {
    let dir = TempDir::new("g12-driver-stale-idle").unwrap();
    let store = store(dir.path(), InstanceStatus::Running).await;
    state_event(&store, SESSION, false).await;
    drop(store);
    let store = Arc::new(SqliteStore::open(dir.path(), 2).unwrap());
    let driver = ClaudeDriver::new(store.clone());
    let message = message();
    let start = Instant::now();
    assert_eq!(
        driver
            .deliver("claude", &message, BusyLevel::Queue)
            .await
            .unwrap()
            .state,
        DeliveryState::Queued
    );
    assert!(
        start.elapsed() < Duration::from_secs(6),
        "receipt wait exceeded its bound"
    );
    let row = store.message(&message.id).await.unwrap().unwrap();
    let delivery = store.claude_delivery(&message.id).await.unwrap().unwrap();
    assert!(row.attempted_at_unix_ms.is_none());
    assert!(delivery.attempt.is_none());
    assert!(delivery.abandoned_at_unix_ms.is_none());
}

#[tokio::test]
async fn busy_or_other_session_idle_does_not_wait_for_a_write_receipt() {
    let dir = TempDir::new("g12-driver-busy-receipt").unwrap();
    let store = store(dir.path(), InstanceStatus::Running).await;
    state_event(&store, DELIVERY, false).await;
    let driver = ClaudeDriver::new(store.clone());
    let message = message();
    let start = Instant::now();
    driver
        .deliver("claude", &message, BusyLevel::Queue)
        .await
        .unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
    state_event(&store, SESSION, false).await;
    state_event(&store, SESSION, true).await;
    let start = Instant::now();
    assert_eq!(
        driver
            .deliver("claude", &message, BusyLevel::Queue)
            .await
            .unwrap()
            .state,
        DeliveryState::Queued
    );
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn failed_instance_never_abandons_unsent_messages_even_after_reopen() {
    let dir = TempDir::new("g12-driver-unsent").unwrap();
    let store = store(dir.path(), InstanceStatus::Failed).await;
    let driver = ClaudeDriver::new(store.clone());
    let message = message();
    for _ in 0..2 {
        assert_eq!(
            driver
                .deliver("claude", &message, BusyLevel::Queue)
                .await
                .unwrap()
                .state,
            DeliveryState::Queued
        );
        let delivery = store.claude_delivery(&message.id).await.unwrap().unwrap();
        assert!(delivery.attempt.is_none());
        assert!(delivery.abandoned_at_unix_ms.is_none());
    }
    drop(driver);
    drop(store);
    let store = Arc::new(SqliteStore::open(dir.path(), 2).unwrap());
    let driver = ClaudeDriver::new(store.clone());
    assert_eq!(
        driver
            .deliver("claude", &message, BusyLevel::Queue)
            .await
            .unwrap()
            .state,
        DeliveryState::Queued
    );
    assert!(
        store
            .claude_delivery(&message.id)
            .await
            .unwrap()
            .unwrap()
            .abandoned_at_unix_ms
            .is_none()
    );
    store
        .set_instance_status("claude", InstanceStatus::Running)
        .await
        .unwrap();
    store
        .reserve_claude_delivery(
            NewClaudeDelivery {
                message_id: message.id,
                delivery_id: DELIVERY.into(),
                instance_id: "claude".into(),
                session_id: SESSION.into(),
                route: ClaudeRoute::Channel,
            },
            3,
        )
        .await
        .expect("a recovered instance must still be able to deliver the retained message");
}

#[tokio::test]
async fn failed_instance_keeps_unknown_attempt_and_only_explicit_ack_confirms() {
    let dir = TempDir::new("g12-driver-attempt").unwrap();
    let store = store(dir.path(), InstanceStatus::Running).await;
    let driver = ClaudeDriver::new(store.clone());
    let message = message();
    driver
        .deliver("claude", &message, BusyLevel::Queue)
        .await
        .unwrap();
    store
        .reserve_claude_delivery(
            NewClaudeDelivery {
                message_id: message.id.clone(),
                delivery_id: DELIVERY.into(),
                instance_id: "claude".into(),
                session_id: SESSION.into(),
                route: ClaudeRoute::Stop,
            },
            2,
        )
        .await
        .unwrap();
    store
        .set_instance_status("claude", InstanceStatus::Failed)
        .await
        .unwrap();
    assert_eq!(
        driver
            .deliver("claude", &message, BusyLevel::Queue)
            .await
            .unwrap()
            .state,
        DeliveryState::Queued
    );
    let row = store.message(&message.id).await.unwrap().unwrap();
    let delivery = store.claude_delivery(&message.id).await.unwrap().unwrap();
    assert!(delivery.outcome_unknown(&row));
    let ack = ClaudeAck {
        message_id: message.id.clone(),
        delivery_id: DELIVERY.into(),
        instance_id: "claude".into(),
        session_id: SESSION.into(),
    };
    store.acknowledge_claude(ack.clone(), 3).await.unwrap();
    store.acknowledge_claude(ack, 4).await.unwrap();
    assert_eq!(
        driver
            .deliver("claude", &message, BusyLevel::Queue)
            .await
            .unwrap()
            .state,
        DeliveryState::Confirmed
    );
    let events = driver.events("claude", None).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].kind,
        DriverEventKind::MessageConfirmed {
            message_id: message.id
        }
    );
    assert!(
        driver
            .events("claude", Some(&events[0].cursor))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn hook_and_other_instance_prefixes_cannot_hide_later_replayed_completion() {
    let dir = TempDir::new("g12-driver-events").unwrap();
    let store = store(dir.path(), InstanceStatus::Running).await;
    for index in 0..1025 {
        store
            .append_driver_event(
                NewDriverEvent {
                    id: format!("00000000-0000-4000-8000-{index:012x}"),
                    instance_id: if index % 2 == 0 { "other" } else { "claude" }.into(),
                    session_id: SESSION.into(),
                    kind: if index % 2 == 0 {
                        "AgendState"
                    } else {
                        "UserPromptSubmit"
                    }
                    .into(),
                    payload: serde_json::json!({"busy": true}).to_string(),
                    occurred_at_unix_ms: 1,
                    replayed: false,
                },
                1,
            )
            .await
            .unwrap();
    }
    for (index, active) in [(1025, true), (1026, false)] {
        store
            .append_driver_event(
                NewDriverEvent {
                    id: format!("00000000-0000-4000-8000-{index:012x}"),
                    instance_id: "claude".into(),
                    session_id: SESSION.into(),
                    kind: "Stop".into(),
                    payload: serde_json::json!({"stop_hook_active": active}).to_string(),
                    occurred_at_unix_ms: 1,
                    replayed: true,
                },
                2,
            )
            .await
            .unwrap();
    }
    let driver = ClaudeDriver::new(store);
    let events = driver.events("claude", None).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].kind,
        DriverEventKind::TurnCompleted { summary: None }
    );
    assert!(
        driver
            .events("claude", Some(&events[0].cursor))
            .await
            .unwrap()
            .is_empty()
    );
    for cursor in ["codex:0", "claude:-1", "claude:bad"] {
        assert!(driver.events("claude", Some(cursor)).await.is_err());
    }
    assert!(driver.events("missing", None).await.is_err());
}
