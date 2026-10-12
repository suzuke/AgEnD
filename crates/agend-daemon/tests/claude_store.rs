//! Actual store producer, concurrent reservations, retention, and four process boots.
#![cfg(unix)]
use agend_core::model::{Backend, DeliveryState};
use agend_core::policy::busy::BusyLevel;
use agend_core::runtime_records::*;
use agend_daemon::store::{DB_FILE, SqliteStore, retention::DAY_MS};
use agend_testkit::{block_on, tempdir::TempDir};
use std::{
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

const SESSION: &str = "11111111-1111-4111-8111-111111111111";
const DELIVERY: &str = "22222222-2222-4222-8222-222222222222";
const OTHER: &str = "33333333-3333-4333-8333-333333333333";
fn instance(id: &str, backend: Backend, delivery: &str) -> Instance {
    Instance {
        id: id.into(),
        backend,
        program: "/bin/false".into(),
        args: vec![],
        working_directory: "/tmp".into(),
        session_id: Some(SESSION.into()),
        status: InstanceStatus::Running,
        session_started: true,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: delivery.into(),
    }
}
fn message(id: &str, to: &str) -> NewMessage {
    NewMessage {
        id: id.into(),
        from_instance: "operator".into(),
        to_instance: to.into(),
        task_id: None,
        body: "完整內容 é".into(),
        level: BusyLevel::Queue,
    }
}
fn reservation(id: &str) -> NewClaudeDelivery {
    NewClaudeDelivery {
        message_id: id.into(),
        delivery_id: DELIVERY.into(),
        instance_id: "claude".into(),
        session_id: SESSION.into(),
        route: ClaudeRoute::Channel,
    }
}
fn ack(id: &str) -> ClaudeAck {
    ClaudeAck {
        message_id: id.into(),
        delivery_id: DELIVERY.into(),
        instance_id: "claude".into(),
        session_id: SESSION.into(),
    }
}
fn event(id: &str, occurred: u64) -> NewDriverEvent {
    NewDriverEvent {
        id: id.into(),
        instance_id: "claude".into(),
        session_id: SESSION.into(),
        kind: "Stop".into(),
        payload: serde_json::json!({"stop_hook_active":false,"text":"繁中 é"}).to_string(),
        occurred_at_unix_ms: occurred,
        replayed: false,
    }
}
fn seed(store: &SqliteStore, id: &str, now: u64) {
    block_on(store.add_instance(&instance("claude", Backend::Claude, "push"))).unwrap();
    assert!(matches!(
        block_on(store.claim_message(&message(id, "claude"), now)).unwrap(),
        Claim::Inserted(_)
    ));
}

#[test]
fn concurrent_reservation_permits_only_one_write_and_retains_unknown_outcome() {
    let home = TempDir::new("claude-reserve").unwrap();
    let store = Arc::new(SqliteStore::open(home.path(), 0).unwrap());
    seed(&store, "m", 0);
    let workers: Vec<_> = (0..16)
        .map(|_| {
            let store = store.clone();
            thread::spawn(move || {
                block_on(store.reserve_claude_delivery(reservation("m"), 1)).unwrap()
            })
        })
        .collect();
    let starts = workers
        .into_iter()
        .map(|w| w.join().unwrap())
        .filter(|r| matches!(r, ClaudeReservation::Started(_)))
        .count();
    assert_eq!(starts, 1);
    let stored = block_on(store.claude_delivery("m")).unwrap().unwrap();
    let stored_message = block_on(store.message("m")).unwrap().unwrap();
    assert_eq!(stored_message.state, DeliveryState::Queued);
    assert!(stored.outcome_unknown(&stored_message));
    assert!(!stored.may_start(&stored_message));
    for change in 0..4 {
        let mut r = reservation("m");
        match change {
            0 => r.delivery_id = OTHER.into(),
            1 => r.session_id = OTHER.into(),
            2 => r.instance_id = "other".into(),
            _ => r.route = ClaudeRoute::Stop,
        }
        assert!(block_on(store.reserve_claude_delivery(r, 2)).is_err());
    }
    let mut changed = message("m", "claude");
    changed.body = "different".into();
    assert!(matches!(
        block_on(store.claim_message(&changed, 3)).unwrap(),
        Claim::Different(_)
    ));
    drop(store);
    let store = SqliteStore::open(home.path(), 4).unwrap();
    assert!(matches!(
        block_on(store.reserve_claude_delivery(reservation("m"), 5)).unwrap(),
        ClaudeReservation::Existing(_)
    ));
    assert!(
        block_on(store.claude_delivery("m"))
            .unwrap()
            .unwrap()
            .outcome_unknown(&block_on(store.message("m")).unwrap().unwrap())
    );
}

#[test]
fn ack_requires_full_identity_and_delayed_ack_uses_delivery_session() {
    let home = TempDir::new("claude-ack").unwrap();
    let store = SqliteStore::open(home.path(), 0).unwrap();
    seed(&store, "m", 0);
    assert!(block_on(store.acknowledge_claude(ack("m"), 1)).is_err());
    let mut r = reservation("m");
    r.session_id = OTHER.into();
    assert!(block_on(store.reserve_claude_delivery(r, 1)).is_err());
    block_on(store.reserve_claude_delivery(reservation("m"), 2)).unwrap();
    for change in 0..4 {
        let mut a = ack("m");
        match change {
            0 => a.message_id = "absent".into(),
            1 => a.delivery_id = OTHER.into(),
            2 => a.instance_id = "other".into(),
            _ => a.session_id = OTHER.into(),
        }
        assert!(block_on(store.acknowledge_claude(a, 3)).is_err());
    }
    block_on(store.set_session_id("claude", OTHER)).unwrap();
    block_on(store.remove_instance("claude")).unwrap();
    assert_eq!(
        block_on(store.acknowledge_claude(ack("m"), 100 * DAY_MS)).unwrap(),
        ClaudeAckResult::Confirmed
    );
    let stored = block_on(store.message("m")).unwrap().unwrap();
    assert_eq!(stored.state, DeliveryState::Confirmed);
    assert_eq!(stored.updated_at_unix_ms, 100 * DAY_MS);
    assert_eq!(
        block_on(store.acknowledge_claude(ack("m"), 110 * DAY_MS)).unwrap(),
        ClaudeAckResult::AlreadyConfirmed
    );
    assert_eq!(block_on(store.message("m")).unwrap().unwrap(), stored);
    assert_eq!(
        block_on(store.claude_delivery_written("m", DELIVERY, 120 * DAY_MS))
            .unwrap()
            .attempt
            .unwrap()
            .sent_at_unix_ms,
        Some(100 * DAY_MS)
    );
    assert!(block_on(store.abandon_claude_delivery("m", "operator chose", 120 * DAY_MS)).is_err());
}

#[test]
fn written_is_sent_until_ack_and_abandonment_is_explicit_terminal_and_idempotent() {
    let home = TempDir::new("claude-written").unwrap();
    let store = SqliteStore::open(home.path(), 0).unwrap();
    seed(&store, "m", 0);
    assert!(block_on(store.claude_delivery_written("m", DELIVERY, 1)).is_err());
    block_on(store.reserve_claude_delivery(reservation("m"), 1)).unwrap();
    assert!(block_on(store.claude_delivery_written("m", OTHER, 2)).is_err());
    block_on(store.claude_delivery_written("m", DELIVERY, 3)).unwrap();
    assert_eq!(
        block_on(store.message("m")).unwrap().unwrap().state,
        DeliveryState::Sent
    );
    let stored = block_on(store.abandon_claude_delivery("m", "operator abandoned", 4)).unwrap();
    assert_eq!(
        block_on(store.message("m")).unwrap().unwrap().state,
        DeliveryState::Failed
    );
    assert_eq!(
        block_on(store.abandon_claude_delivery("m", "duplicate", 5)).unwrap(),
        stored
    );
    assert!(block_on(store.acknowledge_claude(ack("m"), 6)).is_err());
    assert!(block_on(store.claude_delivery_written("m", DELIVERY, 6)).is_err());
    assert!(matches!(
        block_on(store.reserve_claude_delivery(reservation("m"), 6)).unwrap(),
        ClaudeReservation::Existing(_)
    ));
    block_on(store.claim_message(&message("unstarted", "claude"), 7)).unwrap();
    assert!(block_on(store.abandon_claude_delivery("unstarted", " ", 8)).is_err());
    block_on(store.abandon_claude_delivery("unstarted", "never send", 8)).unwrap();
    assert!(block_on(store.reserve_claude_delivery(reservation("unstarted"), 9)).is_err());
}

#[test]
fn event_identity_order_ingestion_retention_and_unresolved_message_retention_are_separate() {
    let home = TempDir::new("claude-retention").unwrap();
    let store = SqliteStore::open(home.path(), 0).unwrap();
    seed(&store, "unknown", 0);
    block_on(store.claim_message(&message("queued", "claude"), 0)).unwrap();
    block_on(store.claim_message(&message("sent", "claude"), 0)).unwrap();
    block_on(store.reserve_claude_delivery(reservation("unknown"), 1)).unwrap();
    let mut r = reservation("sent");
    r.delivery_id = OTHER.into();
    r.route = ClaudeRoute::Stop;
    block_on(store.reserve_claude_delivery(r, 1)).unwrap();
    block_on(store.claude_delivery_written("sent", OTHER, 2)).unwrap();
    for (id, backend, delivery) in [
        ("codex", Backend::Codex, "push"),
        ("inbox", Backend::Claude, "inbox"),
    ] {
        block_on(store.add_instance(&instance(id, backend, delivery))).unwrap();
        block_on(store.claim_message(&message(id, id), 0)).unwrap();
        assert!(block_on(store.claude_delivery(id)).unwrap().is_none());
    }
    let old = block_on(store.append_driver_event(event(DELIVERY, 0), 0)).unwrap();
    assert!(old.inserted);
    let mut duplicate = event(DELIVERY, 0);
    duplicate.replayed = true;
    let repeated = block_on(store.append_driver_event(duplicate, DAY_MS)).unwrap();
    assert!(!repeated.inserted);
    assert_eq!(old.event, repeated.event);
    let mut conflict = event(DELIVERY, 0);
    conflict.payload = "{}".into();
    assert!(block_on(store.append_driver_event(conflict, 1)).is_err());
    let recent = block_on(store.append_driver_event(event(OTHER, 0), 13 * DAY_MS)).unwrap();
    assert!(recent.event.seq > old.event.seq);
    block_on(store.prune(14 * DAY_MS)).unwrap();
    assert_eq!(block_on(store.driver_events_after(0, 10)).unwrap().len(), 2);
    block_on(store.prune(14 * DAY_MS + 1)).unwrap();
    assert_eq!(
        block_on(store.driver_events_after(0, 10)).unwrap(),
        vec![recent.event]
    );
    for payload in ["not json", "[]", "null"] {
        let mut e = event(SESSION, 0);
        e.payload = payload.into();
        assert!(block_on(store.append_driver_event(e, 1)).is_err());
    }
    assert!(block_on(store.driver_events_after(-1, 10)).is_err());
    assert!(block_on(store.driver_events_after(0, 1025)).is_err());
    block_on(store.remove_instance("claude")).unwrap();
    block_on(store.prune(30 * DAY_MS)).unwrap();
    assert!(block_on(store.message("codex")).unwrap().is_some());
    block_on(store.prune(30 * DAY_MS + 1)).unwrap();
    assert!(block_on(store.message("codex")).unwrap().is_none());
    assert!(block_on(store.message("inbox")).unwrap().is_none());
    block_on(store.prune(100 * DAY_MS)).unwrap();
    for id in ["queued", "unknown", "sent"] {
        assert!(block_on(store.message(id)).unwrap().is_some());
        assert!(block_on(store.claude_delivery(id)).unwrap().is_some());
    }
    assert!(
        block_on(store.driver_events_after(0, 10))
            .unwrap()
            .is_empty()
    );
    block_on(store.acknowledge_claude(ack("unknown"), 100 * DAY_MS)).unwrap();
    block_on(store.abandon_claude_delivery("queued", "human", 100 * DAY_MS)).unwrap();
    block_on(store.prune(130 * DAY_MS)).unwrap();
    assert!(block_on(store.message("unknown")).unwrap().is_some());
    let report = block_on(store.prune(130 * DAY_MS + 1)).unwrap();
    assert_eq!(report.table("claude_deliveries").unwrap().before, 3);
    assert_eq!(report.table("claude_deliveries").unwrap().after, 1);
    assert!(block_on(store.message("unknown")).unwrap().is_none());
    assert!(block_on(store.message("queued")).unwrap().is_none());
    assert!(block_on(store.message("sent")).unwrap().is_some());
}

#[test]
fn snapshot_restores_pending_identity_and_event_seq_without_replay() {
    let home = TempDir::new("claude-snapshot").unwrap();
    let store = SqliteStore::open(home.path(), 0).unwrap();
    seed(&store, "m", 0);
    block_on(store.reserve_claude_delivery(reservation("m"), 1)).unwrap();
    let e = block_on(store.append_driver_event(event(DELIVERY, 0), 2))
        .unwrap()
        .event;
    let snapshot = block_on(store.snapshot(3)).unwrap();
    assert!(snapshot.taken && !snapshot.empty);
    let saved = TempDir::new("claude-snapshot-restored").unwrap();
    std::fs::copy(snapshot.path, saved.path().join(DB_FILE)).unwrap();
    let restored = SqliteStore::open(saved.path(), 4).unwrap();
    assert!(matches!(
        block_on(restored.reserve_claude_delivery(reservation("m"), 5)).unwrap(),
        ClaudeReservation::Existing(_)
    ));
    assert_eq!(
        block_on(restored.driver_events_after(0, 10)).unwrap(),
        vec![e]
    );
    block_on(restored.acknowledge_claude(ack("m"), 6)).unwrap();
    assert_eq!(
        block_on(store.message("m")).unwrap().unwrap().state,
        DeliveryState::Queued
    );
}

#[test]
fn child_boot() {
    let Some(home) = std::env::var_os("AGEND_CLAUDE_STORE_CHILD") else {
        return;
    };
    let boot: u32 = std::env::var("AGEND_CLAUDE_STORE_BOOT")
        .unwrap()
        .parse()
        .unwrap();
    let store = SqliteStore::open(std::path::Path::new(&home), u64::from(boot)).unwrap();
    if boot == 1 {
        seed(&store, "m", 0);
        block_on(store.reserve_claude_delivery(reservation("m"), 1)).unwrap();
        // Parent kills this process after the committed intent, before any write.
        std::fs::write(
            std::path::Path::new(&home).join("intent-ready"),
            b"committed",
        )
        .unwrap();
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    } else {
        let m = block_on(store.message("m"))
            .unwrap()
            .expect("same home retains message");
        if boot == 2 {
            assert_eq!(m.state, DeliveryState::Queued);
            assert!(
                block_on(store.claude_delivery("m"))
                    .unwrap()
                    .unwrap()
                    .outcome_unknown(&m)
            );
            assert!(matches!(
                block_on(store.reserve_claude_delivery(reservation("m"), 2)).unwrap(),
                ClaudeReservation::Existing(_)
            ));
        }
        if boot == 2 {
            block_on(store.claim_message(&message("written", "claude"), 2)).unwrap();
            let mut r = reservation("written");
            r.delivery_id = OTHER.into();
            r.route = ClaudeRoute::Stop;
            block_on(store.reserve_claude_delivery(r, 2)).unwrap();
            block_on(store.claude_delivery_written("written", OTHER, 2)).unwrap();
        }
        if boot == 3 {
            assert_eq!(
                block_on(store.acknowledge_claude(ack("m"), 3)).unwrap(),
                ClaudeAckResult::Confirmed
            );
        }
        if boot == 4 {
            assert_eq!(m.state, DeliveryState::Confirmed);
            assert_eq!(m.updated_at_unix_ms, 3);
            assert_eq!(
                block_on(store.message("written")).unwrap().unwrap().state,
                DeliveryState::Sent
            );
            assert_eq!(
                block_on(store.acknowledge_claude(ack("m"), 4)).unwrap(),
                ClaudeAckResult::AlreadyConfirmed
            );
        }
    }
    println!("BOOT {boot} PID {}", std::process::id());
}
fn boot(home: &std::path::Path, n: u32) -> (bool, u32) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "child_boot", "--nocapture"])
        .current_dir(home)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("AGEND_") {
            command.env_remove(key);
        }
    }
    let mut child = command
        .env("AGEND_CLAUDE_STORE_CHILD", home)
        .env("AGEND_CLAUDE_STORE_BOOT", n.to_string())
        .spawn()
        .unwrap();
    let pid = child.id();
    println!("store boot {n}: pid={pid}");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if n == 1 && home.join("intent-ready").exists() {
            child.kill().unwrap();
            assert!(!child.wait().unwrap().success());
            return (true, pid);
        }
        if let Some(status) = child.try_wait().unwrap() {
            return (status.success(), pid);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("child boot timed out");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn four_distinct_process_boots_and_new_home_negative_control() {
    let home = TempDir::new("claude-four-boots").unwrap();
    let mut pids = std::collections::BTreeSet::new();
    for n in 1..=4 {
        let (success, pid) = boot(home.path(), n);
        assert!(success, "boot {n}");
        assert!(pids.insert(pid), "distinct process per boot");
    }
    let empty = TempDir::new("claude-negative-home").unwrap();
    assert!(!boot(empty.path(), 2).0);
}

#[test]
fn event_only_database_is_nonempty_and_seq_is_never_reused_after_prune() {
    let home = TempDir::new("claude-events-only").unwrap();
    let store = SqliteStore::open(home.path(), 0).unwrap();
    let mut oversized = event(DELIVERY, 0);
    oversized.payload = serde_json::json!({"text":"x".repeat(1024*1024)}).to_string();
    assert!(block_on(store.append_driver_event(oversized, 0)).is_err());
    let first = block_on(store.append_driver_event(event(DELIVERY, 0), 0))
        .unwrap()
        .event;
    let snapshot = block_on(store.snapshot(0)).unwrap();
    assert!(snapshot.taken && !snapshot.empty);
    block_on(store.prune(14 * DAY_MS + 1)).unwrap();
    assert!(
        block_on(store.driver_events_after(0, 1))
            .unwrap()
            .is_empty()
    );
    let next = block_on(store.append_driver_event(event(OTHER, 1), 14 * DAY_MS + 1))
        .unwrap()
        .event;
    assert!(next.seq > first.seq);
    // A pruned event has no lasting deduplication promise; old ACK tuples live elsewhere.
    let replay = block_on(store.append_driver_event(event(DELIVERY, 0), 14 * DAY_MS + 2)).unwrap();
    assert!(replay.inserted);
    assert!(replay.event.seq > next.seq);
    assert_eq!(
        block_on(store.driver_events_after(first.seq, 1)).unwrap(),
        vec![next]
    );
}

#[test]
fn pending_claude_data_alone_is_snapshotted_after_instance_removal() {
    let home = TempDir::new("claude-pending-only").unwrap();
    let store = SqliteStore::open(home.path(), 0).unwrap();
    seed(&store, "m", 0);
    block_on(store.reserve_claude_delivery(reservation("m"), 1)).unwrap();
    block_on(store.remove_instance("claude")).unwrap();
    block_on(store.prune(500 * DAY_MS)).unwrap();
    let snapshot = block_on(store.snapshot(500 * DAY_MS)).unwrap();
    assert!(snapshot.taken && !snapshot.empty);
    let saved = TempDir::new("claude-pending-restored").unwrap();
    std::fs::copy(snapshot.path, saved.path().join(DB_FILE)).unwrap();
    let restored = SqliteStore::open(saved.path(), 500 * DAY_MS + 1).unwrap();
    assert!(block_on(restored.instance("claude")).unwrap().is_none());
    assert!(
        block_on(restored.claude_delivery("m"))
            .unwrap()
            .unwrap()
            .outcome_unknown(&block_on(restored.message("m")).unwrap().unwrap())
    );
    assert_eq!(
        block_on(restored.acknowledge_claude(ack("m"), 500 * DAY_MS + 2)).unwrap(),
        ClaudeAckResult::Confirmed
    );
}

#[test]
fn execution_outcome_requires_native_post_ack_and_stop_and_survives_reopen() {
    let home = TempDir::new("g13-claude-outcome").unwrap();
    let store = SqliteStore::open(home.path(), 0).unwrap();
    seed(&store, "m", 0);
    block_on(store.reserve_claude_delivery(reservation("m"), 1)).unwrap();
    block_on(store.acknowledge_claude(ack("m"), 2)).unwrap();
    assert_eq!(block_on(store.claude_message_outcome("m")).unwrap(), None);
    let captured: serde_json::Value = serde_json::from_str(include_str!(
        "../src/driver/claude/fixtures/ack-stop-outcome.json"
    ))
    .unwrap();
    for (index, kind, now) in [(1, "PostToolUse", 3), (2, "Stop", 4)] {
        let mut payload = captured["events"][index]["payload"].clone();
        payload["session_id"] = serde_json::json!(SESSION);
        if kind == "PostToolUse" {
            payload["tool_input"]["receipts"][0] =
                serde_json::json!({"message_id":"m","delivery_id":DELIVERY,"session_id":SESSION});
        }
        let e = NewDriverEvent {
            id: if index == 1 { DELIVERY } else { OTHER }.into(),
            instance_id: "claude".into(),
            session_id: SESSION.into(),
            kind: kind.into(),
            payload: payload.to_string(),
            occurred_at_unix_ms: now,
            replayed: false,
        };
        block_on(store.append_driver_event(e, now)).unwrap();
        if kind == "PostToolUse" {
            assert_eq!(block_on(store.claude_message_outcome("m")).unwrap(), None);
        }
    }
    let before = block_on(store.message("m")).unwrap();
    let expected = Some("9a9acc3a-7936-42c6-91da-96feec7f4b9e".to_string());
    assert_eq!(
        block_on(store.claude_message_outcome("m")).unwrap(),
        expected
    );
    assert_eq!(block_on(store.message("m")).unwrap(), before);
    drop(store);
    let store = SqliteStore::open(home.path(), 5).unwrap();
    assert_eq!(
        block_on(store.claude_message_outcome("m")).unwrap(),
        expected
    );
    assert_eq!(
        block_on(store.claude_message_outcome("absent")).unwrap(),
        None
    );
}
