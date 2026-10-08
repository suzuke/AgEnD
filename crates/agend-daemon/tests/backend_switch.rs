//! Native SQLite transitions: persistence, atomic program changes and stale CAS.
use agend_core::{
    model::Backend,
    runtime_records::{BackendSwitchPhase, Instance, InstanceStatus},
    setup::backend::ImportedBackend,
    traits::HolderLaunch,
};
use agend_daemon::store::SqliteStore;
use agend_testkit::{block_on, tempdir::TempDir};
fn fixture(store: &SqliteStore) -> (Instance, ImportedBackend) {
    fixture_backend(store, Backend::Codex)
}
fn fixture_backend(store: &SqliteStore, backend: Backend) -> (Instance, ImportedBackend) {
    let instance = Instance {
        id: "switch-1".into(),
        backend,
        program: "/managed/old/program".into(),
        args: vec![],
        working_directory: "/workspace".into(),
        session_id: Some("11111111-1111-4111-8111-111111111111".into()),
        status: InstanceStatus::Running,
        session_started: true,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "push".into(),
    };
    block_on(store.add_instance(&instance)).unwrap();
    let old = ImportedBackend {
        format: 1,
        backend: backend.as_str().into(),
        version: "1".into(),
        sha256: "a".repeat(64),
        bytes: 42,
    };
    let launch = HolderLaunch {
        instance_id: instance.id.clone(),
        backend: instance.backend,
        executable: instance.program.clone(),
        args: vec![],
        working_directory: instance.working_directory.clone(),
    };
    block_on(store.prepare_managed_launch(&instance, &launch, old.clone(), None)).unwrap();
    (
        instance,
        ImportedBackend {
            version: "2".into(),
            sha256: "b".repeat(64),
            ..old
        },
    )
}
#[test]
fn program_and_phase_survive_reopen_and_rollback_without_replaying_stale_requests() {
    let root = TempDir::new("backend-switch-store").unwrap();
    let home = root.path().join("home");
    let store = SqliteStore::open(&home, 0).unwrap();
    let (instance, target) = fixture(&store);
    let prepared = block_on(store.prepare_backend_switch(
        &instance,
        target.clone(),
        "/managed/new/program",
        None,
    ))
    .unwrap();
    assert_eq!(
        block_on(store.instance(&instance.id)).unwrap().unwrap(),
        instance
    );
    assert!(
        block_on(store.prepare_backend_switch(
            &instance,
            target,
            "/managed/new/program",
            Some(&prepared.id)
        ))
        .is_err()
    );
    drop(store);
    let store = SqliteStore::open(&home, 1).unwrap();
    assert_eq!(
        block_on(store.backend_switch(&instance.id)).unwrap(),
        Some(prepared.clone())
    );
    let committed = block_on(store.commit_backend_switch(&prepared, false)).unwrap();
    assert_eq!(committed.phase, BackendSwitchPhase::Committed);
    assert!(block_on(store.cancel_backend_switch(&committed)).is_err());
    assert_eq!(
        block_on(store.instance(&instance.id))
            .unwrap()
            .unwrap()
            .program,
        "/managed/new/program"
    );
    assert!(block_on(store.commit_backend_switch(&prepared, false)).is_err());
    drop(store);
    let store = SqliteStore::open(&home, 2).unwrap();
    assert_eq!(
        block_on(store.backend_switch(&instance.id)).unwrap(),
        Some(committed.clone())
    );
    let restored = block_on(store.commit_backend_switch(&committed, true)).unwrap();
    assert_eq!(restored.phase, BackendSwitchPhase::RolledBack);
    assert_eq!(
        block_on(store.instance(&instance.id)).unwrap().unwrap(),
        instance
    );
    assert!(block_on(store.commit_backend_switch(&committed, true)).is_err());
    block_on(store.remove_instance(&instance.id)).unwrap();
    assert!(
        block_on(store.backend_switch(&instance.id))
            .unwrap()
            .is_none()
    );
}
#[test]
fn changed_configuration_refuses_commit_and_preserves_prepared_record() {
    let root = TempDir::new("backend-switch-conflict").unwrap();
    let home = root.path().join("home");
    let store = SqliteStore::open(&home, 0).unwrap();
    let (instance, target) = fixture(&store);
    let prepared =
        block_on(store.prepare_backend_switch(&instance, target, "/managed/new/program", None))
            .unwrap();
    drop(store);
    let db = rusqlite::Connection::open(home.join("agend.db")).unwrap();
    db.execute(
        "UPDATE instances SET program='/operator/changed' WHERE id=?1",
        [&instance.id],
    )
    .unwrap();
    drop(db);
    let store = SqliteStore::open(&home, 1).unwrap();
    assert!(block_on(store.commit_backend_switch(&prepared, false)).is_err());
    assert_eq!(
        block_on(store.backend_switch(&instance.id)).unwrap(),
        Some(prepared)
    );
    assert_eq!(
        block_on(store.instance(&instance.id))
            .unwrap()
            .unwrap()
            .program,
        "/operator/changed"
    );
}

#[test]
fn replaced_launch_reservation_cannot_be_committed_by_an_old_switch() {
    let root = TempDir::new("backend-switch-launch-conflict").unwrap();
    let store = SqliteStore::open(&root.path().join("home"), 0).unwrap();
    let (instance, target) = fixture(&store);
    let prepared =
        block_on(store.prepare_backend_switch(&instance, target, "/managed/new/program", None))
            .unwrap();
    let launch = HolderLaunch {
        instance_id: instance.id.clone(),
        backend: instance.backend,
        executable: instance.program.clone(),
        args: instance.args.clone(),
        working_directory: instance.working_directory.clone(),
    };
    block_on(store.prepare_managed_launch(
        &instance,
        &launch,
        prepared.previous.artifact.clone(),
        Some(&prepared.previous.binding),
    ))
    .unwrap();
    assert!(block_on(store.commit_backend_switch(&prepared, false)).is_err());
    assert_eq!(
        block_on(store.backend_switch(&instance.id)).unwrap(),
        Some(prepared)
    );
    assert_eq!(
        block_on(store.instance(&instance.id)).unwrap(),
        Some(instance)
    );
}

#[test]
fn cancelling_prepared_switch_preserves_running_agent_and_allows_a_new_request() {
    let root = TempDir::new("backend-switch-cancel").unwrap();
    let home = root.path().join("home");
    let store = SqliteStore::open(&home, 0).unwrap();
    let (instance, target) = fixture(&store);
    let prepared = block_on(store.prepare_backend_switch(
        &instance,
        target.clone(),
        "/managed/new/program",
        None,
    ))
    .unwrap();
    drop(store);
    // Persist a PID using the same native column the runtime owns. This test
    // performs no process IO; cancellation must not require clearing the PID.
    let db = rusqlite::Connection::open(home.join("agend.db")).unwrap();
    db.execute(
        "UPDATE instances SET agent_pid=23456 WHERE id=?1",
        [&instance.id],
    )
    .unwrap();
    drop(db);
    let store = SqliteStore::open(&home, 1).unwrap();
    let running = block_on(store.instance(&instance.id)).unwrap().unwrap();
    let cancelled = block_on(store.cancel_backend_switch(&prepared)).unwrap();
    assert_eq!(cancelled.phase, BackendSwitchPhase::Cancelled);
    assert_eq!(
        block_on(store.instance(&instance.id)).unwrap(),
        Some(running.clone())
    );
    assert!(block_on(store.commit_backend_switch(&prepared, false)).is_err());
    assert!(block_on(store.cancel_backend_switch(&prepared)).is_err());
    assert!(
        block_on(store.prepare_backend_switch(
            &running,
            target.clone(),
            "/managed/new/program",
            Some(&prepared.id)
        ))
        .is_ok()
    );
}

#[test]
fn prepared_switch_holds_all_three_native_reservations_across_reopen_until_cancel() {
    use agend_core::{
        policy::busy::BusyLevel,
        runtime_records::{ClaudeReservation, ClaudeRoute, NewClaudeDelivery, NewMessage},
    };
    for backend in [Backend::Claude, Backend::Codex, Backend::Opencode] {
        let root = TempDir::new("backend-switch-dispatch").unwrap();
        let store = SqliteStore::open(root.path(), 0).unwrap();
        let (instance, target) = fixture_backend(&store, backend);
        block_on(store.claim_message(
            &NewMessage {
                id: "pending".into(),
                from_instance: "operator".into(),
                to_instance: instance.id.clone(),
                task_id: None,
                body: "keep queued during switch".into(),
                level: BusyLevel::Queue,
            },
            0,
        ))
        .unwrap();
        let prepared =
            block_on(store.prepare_backend_switch(&instance, target, "/managed/new/program", None))
                .unwrap();
        drop(store);
        let store = SqliteStore::open(root.path(), 1).unwrap();
        let reserve = |route| NewClaudeDelivery {
            message_id: "pending".into(),
            delivery_id: "22222222-2222-4222-8222-222222222222".into(),
            instance_id: instance.id.clone(),
            session_id: instance.session_id.clone().unwrap(),
            route,
        };
        for _ in 0..2 {
            match backend {
                Backend::Claude => {
                    for route in [ClaudeRoute::Channel, ClaudeRoute::Stop] {
                        assert_eq!(
                            block_on(store.reserve_claude_delivery(reserve(route), 2)).unwrap(),
                            ClaudeReservation::Paused
                        );
                    }
                }
                Backend::Codex => {
                    assert!(!block_on(store.begin_codex_attempt("pending", 2)).unwrap())
                }
                Backend::Opencode => assert!(
                    !block_on(store.begin_opencode_attempt(
                        "pending",
                        &instance.id,
                        instance.session_id.as_ref().unwrap(),
                        "native-message",
                        2
                    ))
                    .unwrap()
                ),
            }
            let row = block_on(store.message("pending")).unwrap().unwrap();
            assert_eq!(row.state, agend_core::model::DeliveryState::Queued);
            assert_eq!(row.attempted_at_unix_ms, None);
        }
        block_on(store.cancel_backend_switch(&prepared)).unwrap();
        match backend {
            Backend::Claude => assert!(matches!(
                block_on(store.reserve_claude_delivery(reserve(ClaudeRoute::Channel), 3)).unwrap(),
                ClaudeReservation::Started(_)
            )),
            Backend::Codex => assert!(block_on(store.begin_codex_attempt("pending", 3)).unwrap()),
            Backend::Opencode => assert!(
                block_on(store.begin_opencode_attempt(
                    "pending",
                    &instance.id,
                    instance.session_id.as_ref().unwrap(),
                    "native-message",
                    3
                ))
                .unwrap()
            ),
        }
        assert_eq!(
            block_on(store.message("pending"))
                .unwrap()
                .unwrap()
                .attempted_at_unix_ms,
            Some(3)
        );
    }
}

#[test]
fn prepared_switch_still_accepts_receipts_for_previously_reserved_content() {
    use agend_core::{
        model::DeliveryState,
        policy::busy::BusyLevel,
        runtime_records::{
            ClaudeAck, ClaudeAckResult, ClaudeReservation, ClaudeRoute, NewClaudeDelivery,
            NewMessage,
        },
    };
    for backend in [Backend::Claude, Backend::Opencode] {
        let root = TempDir::new("backend-switch-receipts").unwrap();
        let store = SqliteStore::open(root.path(), 0).unwrap();
        let (instance, target) = fixture_backend(&store, backend);
        block_on(store.claim_message(
            &NewMessage {
                id: "inflight".into(),
                from_instance: "operator".into(),
                to_instance: instance.id.clone(),
                task_id: None,
                body: "already reserved".into(),
                level: BusyLevel::Queue,
            },
            0,
        ))
        .unwrap();
        let session = instance.session_id.clone().unwrap();
        let delivery = "22222222-2222-4222-8222-222222222222";
        if backend == Backend::Claude {
            assert!(matches!(
                block_on(store.reserve_claude_delivery(
                    NewClaudeDelivery {
                        message_id: "inflight".into(),
                        delivery_id: delivery.into(),
                        instance_id: instance.id.clone(),
                        session_id: session.clone(),
                        route: ClaudeRoute::Channel,
                    },
                    1
                ))
                .unwrap(),
                ClaudeReservation::Started(_)
            ));
        } else {
            assert!(
                block_on(store.begin_opencode_attempt(
                    "inflight",
                    &instance.id,
                    &session,
                    "native-message",
                    1
                ))
                .unwrap()
            );
        }
        block_on(store.prepare_backend_switch(&instance, target, "/managed/new/program", None))
            .unwrap();
        if backend == Backend::Claude {
            assert_eq!(
                block_on(store.acknowledge_claude(
                    ClaudeAck {
                        message_id: "inflight".into(),
                        delivery_id: delivery.into(),
                        instance_id: instance.id.clone(),
                        session_id: session,
                    },
                    2
                ))
                .unwrap(),
                ClaudeAckResult::Confirmed
            );
        } else {
            assert!(
                block_on(store.confirm_opencode_attempt("inflight", &session, "native-message", 2))
                    .unwrap()
                    .is_some()
            );
        }
        assert_eq!(
            block_on(store.message("inflight")).unwrap().unwrap().state,
            DeliveryState::Confirmed
        );
    }
}
