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
    fixture_delivery(store, backend, "push")
}
fn fixture_delivery(
    store: &SqliteStore,
    backend: Backend,
    delivery: &str,
) -> (Instance, ImportedBackend) {
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
        delivery: delivery.into(),
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
    let committed = block_on(store.commit_backend_switch(&prepared, false, 0)).unwrap();
    assert_eq!(committed.phase, BackendSwitchPhase::Committed);
    assert!(block_on(store.cancel_backend_switch(&committed)).is_err());
    assert_eq!(
        block_on(store.instance(&instance.id))
            .unwrap()
            .unwrap()
            .program,
        "/managed/new/program"
    );
    assert!(block_on(store.commit_backend_switch(&prepared, false, 0)).is_err());
    drop(store);
    let store = SqliteStore::open(&home, 2).unwrap();
    assert_eq!(
        block_on(store.backend_switch(&instance.id)).unwrap(),
        Some(committed.clone())
    );
    let rollback = block_on(store.prepare_backend_rollback(&committed)).unwrap();
    assert_eq!(rollback.phase, BackendSwitchPhase::RollbackPrepared);
    drop(store);
    let store = SqliteStore::open(&home, 3).unwrap();
    assert_eq!(
        block_on(store.backend_switch(&instance.id)).unwrap(),
        Some(rollback.clone())
    );
    assert!(block_on(store.prepare_backend_rollback(&committed)).is_err());
    let restored = block_on(store.commit_backend_switch(&rollback, true, 0)).unwrap();
    assert_eq!(restored.phase, BackendSwitchPhase::Restoring);
    assert_eq!(
        block_on(store.instance(&instance.id)).unwrap().unwrap(),
        instance
    );
    assert!(block_on(store.commit_backend_switch(&committed, true, 0)).is_err());
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
    assert!(block_on(store.commit_backend_switch(&prepared, false, 0)).is_err());
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
    assert!(block_on(store.commit_backend_switch(&prepared, false, 0)).is_err());
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
    assert!(block_on(store.commit_backend_switch(&prepared, false, 0)).is_err());
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

#[test]
fn prepared_switch_holds_inbox_reads_without_hiding_operator_history() {
    use agend_core::{policy::busy::BusyLevel, runtime_records::NewMessage};
    let root = TempDir::new("backend-switch-inbox").unwrap();
    let store = SqliteStore::open(root.path(), 0).unwrap();
    let (instance, target) = fixture_delivery(&store, Backend::Claude, "inbox");
    for id in ["first", "second", "third"] {
        block_on(store.claim_message(
            &NewMessage {
                id: id.into(),
                from_instance: "operator".into(),
                to_instance: instance.id.clone(),
                task_id: None,
                body: id.into(),
                level: BusyLevel::Queue,
            },
            0,
        ))
        .unwrap();
    }
    let expected = block_on(store.inbox_messages(&instance.id, None, 2))
        .unwrap()
        .unwrap();
    assert_eq!(
        expected.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["second", "third"]
    );
    assert_eq!(
        block_on(store.inbox_messages(&instance.id, Some("first"), 2))
            .unwrap()
            .unwrap(),
        expected
    );
    assert!(
        block_on(store.inbox_messages(&instance.id, Some("foreign"), 2))
            .unwrap()
            .is_none()
    );
    let prepared =
        block_on(store.prepare_backend_switch(&instance, target, "/managed/new/program", None))
            .unwrap();
    drop(store);
    let store = SqliteStore::open(root.path(), 1).unwrap();
    for after in [None, Some("first")] {
        let error = block_on(store.inbox_messages(&instance.id, after, 2)).unwrap_err();
        assert!(error.to_string().contains("inbox delivery is paused"));
    }
    assert_eq!(block_on(store.messages_to(&instance.id)).unwrap().len(), 3);
    assert_eq!(
        block_on(store.inbox_messages("other", None, 2)).unwrap(),
        Some(vec![])
    );
    block_on(store.cancel_backend_switch(&prepared)).unwrap();
    assert_eq!(
        block_on(store.inbox_messages(&instance.id, None, 2))
            .unwrap()
            .unwrap(),
        expected
    );
}

/// Native Store produces both the saved reservation and the observed snapshot.
/// These fixtures assert persistence rules, not actual process readiness.
fn next_launch(
    store: &SqliteStore,
    artifact: ImportedBackend,
) -> (Instance, agend_core::runtime_records::ManagedLaunchIntent) {
    let instance = block_on(store.instance("switch-1")).unwrap().unwrap();
    let previous = block_on(store.managed_launch(&instance.id))
        .unwrap()
        .unwrap();
    let launch = HolderLaunch {
        instance_id: instance.id.clone(),
        backend: instance.backend,
        executable: instance.program.clone(),
        args: instance.args.clone(),
        working_directory: instance.working_directory.clone(),
    };
    let intent = block_on(store.prepare_managed_launch(
        &instance,
        &launch,
        artifact,
        Some(&previous.binding),
    ))
    .unwrap();
    block_on(store.set_agent_pid(&instance.id, Some(23456))).unwrap();
    (
        block_on(store.instance(&instance.id)).unwrap().unwrap(),
        intent,
    )
}

#[test]
fn commit_and_restore_keep_all_delivery_paused_until_exact_activation_snapshot() {
    use agend_core::{
        policy::busy::BusyLevel,
        runtime_records::{ClaudeReservation, ClaudeRoute, NewClaudeDelivery, NewMessage},
    };
    for backend in [Backend::Claude, Backend::Codex, Backend::Opencode] {
        for rollback in [false, true] {
            let root = TempDir::new("backend-switch-activation").unwrap();
            let store = SqliteStore::open(root.path(), 0).unwrap();
            let (instance, target) = fixture_backend(&store, backend);
            block_on(store.claim_message(
                &NewMessage {
                    id: "pending".into(),
                    from_instance: "operator".into(),
                    to_instance: instance.id.clone(),
                    task_id: None,
                    body: "hold until native activation".into(),
                    level: BusyLevel::Queue,
                },
                0,
            ))
            .unwrap();
            let prepared = block_on(store.prepare_backend_switch(
                &instance,
                target.clone(),
                "/managed/new/program",
                None,
            ))
            .unwrap();
            let committed = block_on(store.commit_backend_switch(&prepared, false, 0)).unwrap();
            let assert_paused = |store: &SqliteStore| {
                assert!(block_on(store.inbox_messages(&instance.id, None, 20)).is_err());
                match backend {
                    Backend::Claude => {
                        for route in [ClaudeRoute::Channel, ClaudeRoute::Stop] {
                            assert_eq!(
                                block_on(store.reserve_claude_delivery(
                                    NewClaudeDelivery {
                                        message_id: "pending".into(),
                                        delivery_id: "22222222-2222-4222-8222-222222222222".into(),
                                        instance_id: instance.id.clone(),
                                        session_id: instance.session_id.clone().unwrap(),
                                        route,
                                    },
                                    1
                                ))
                                .unwrap(),
                                ClaudeReservation::Paused
                            );
                        }
                    }
                    Backend::Codex => {
                        assert!(!block_on(store.begin_codex_attempt("pending", 1)).unwrap())
                    }
                    Backend::Opencode => assert!(
                        !block_on(store.begin_opencode_attempt(
                            "pending",
                            &instance.id,
                            instance.session_id.as_deref().unwrap(),
                            "native-message",
                            1,
                        ))
                        .unwrap()
                    ),
                }
                assert_eq!(
                    block_on(store.message("pending"))
                        .unwrap()
                        .unwrap()
                        .attempted_at_unix_ms,
                    None
                );
            };
            assert_paused(&store);
            let (record, artifact) = if rollback {
                (
                    block_on(store.commit_backend_switch(&committed, true, 0)).unwrap(),
                    prepared.previous.artifact.clone(),
                )
            } else {
                (committed, target.clone())
            };
            drop(store);
            let store = SqliteStore::open(root.path(), 2).unwrap();
            assert_paused(&store);
            let current = block_on(store.instance(&instance.id)).unwrap().unwrap();
            assert!(
                block_on(store.prepare_backend_switch(
                    &current,
                    target.clone(),
                    "/managed/third/program",
                    Some(&record.id),
                ))
                .is_err()
            );
            assert!(block_on(store.cancel_backend_switch(&record)).is_err());
            // Restoring the program does not authorize the original dead launch.
            assert!(
                block_on(store.finish_backend_switch(&record, &current, &prepared.previous))
                    .is_err()
            );
            // A real replacement reservation with the wrong bytes must fail
            // even though both the supplied and stored launch match exactly.
            let mut foreign = artifact.clone();
            foreign.sha256 = "d".repeat(64);
            let (foreign_ready, foreign_launch) = next_launch(&store, foreign);
            assert!(
                block_on(store.finish_backend_switch(&record, &foreign_ready, &foreign_launch))
                    .is_err()
            );
            assert_paused(&store);
            block_on(store.set_agent_pid(&instance.id, None)).unwrap();
            let (ready, launch) = next_launch(&store, artifact);
            let mut third = target.clone();
            third.version = "3".into();
            third.sha256 = "e".repeat(64);
            assert!(
                block_on(store.prepare_backend_switch(
                    &ready,
                    third,
                    "/managed/third/program",
                    Some(&record.id),
                ))
                .is_err(),
                "an unfinished activation was overwritten"
            );
            let mut wrong = launch.clone();
            wrong.artifact.sha256 = "c".repeat(64);
            assert!(block_on(store.finish_backend_switch(&record, &ready, &wrong)).is_err());
            let mut stale = ready.clone();
            stale.agent_pid = Some(23457);
            assert!(block_on(store.finish_backend_switch(&record, &stale, &launch)).is_err());
            block_on(store.set_agent_pid(&instance.id, None)).unwrap();
            assert!(block_on(store.finish_backend_switch(&record, &ready, &launch)).is_err());
            assert_paused(&store);
            block_on(store.set_agent_pid(&instance.id, ready.agent_pid)).unwrap();
            let record =
                block_on(store.backend_switch_problem(&record, "startup was delayed", 42)).unwrap();
            let finished = block_on(store.finish_backend_switch(&record, &ready, &launch)).unwrap();
            assert!(finished.problem.is_none());
            assert!(finished.activation_deadline_unix_ms.is_none());
            assert_eq!(
                finished.phase,
                if rollback {
                    BackendSwitchPhase::RolledBack
                } else {
                    BackendSwitchPhase::Activated
                }
            );
            assert!(!finished.phase.pending());
            assert!(block_on(store.finish_backend_switch(&record, &ready, &launch)).is_err());
            drop(store);
            let store = SqliteStore::open(root.path(), 3).unwrap();
            assert_eq!(
                block_on(store.backend_switch(&instance.id)).unwrap(),
                Some(finished)
            );
            assert_eq!(
                block_on(store.inbox_messages(&instance.id, None, 20))
                    .unwrap()
                    .unwrap()
                    .len(),
                1
            );
            match backend {
                Backend::Claude => assert!(matches!(
                    block_on(store.reserve_claude_delivery(
                        NewClaudeDelivery {
                            message_id: "pending".into(),
                            delivery_id: "22222222-2222-4222-8222-222222222222".into(),
                            instance_id: instance.id.clone(),
                            session_id: instance.session_id.clone().unwrap(),
                            route: ClaudeRoute::Channel,
                        },
                        4
                    ))
                    .unwrap(),
                    ClaudeReservation::Started(_)
                )),
                Backend::Codex => {
                    assert!(block_on(store.begin_codex_attempt("pending", 4)).unwrap())
                }
                Backend::Opencode => assert!(
                    block_on(store.begin_opencode_attempt(
                        "pending",
                        &instance.id,
                        instance.session_id.as_deref().unwrap(),
                        "native-message",
                        4,
                    ))
                    .unwrap()
                ),
            }
            if !rollback {
                let active = block_on(store.backend_switch(&instance.id))
                    .unwrap()
                    .unwrap();
                let paused = block_on(store.prepare_backend_rollback(&active)).unwrap();
                assert_eq!(paused.phase, BackendSwitchPhase::RollbackPrepared);
                assert!(paused.phase.pending());
                assert!(block_on(store.inbox_messages(&instance.id, None, 20)).is_err());
                assert!(block_on(store.prepare_backend_rollback(&active)).is_err());
                assert!(block_on(store.cancel_backend_switch(&paused)).is_err());
                assert!(block_on(store.commit_backend_switch(&paused, true, 0)).is_err());
                block_on(store.set_agent_pid(&instance.id, None)).unwrap();
                let restoring = block_on(store.commit_backend_switch(&paused, true, 0)).unwrap();
                assert_eq!(restoring.phase, BackendSwitchPhase::Restoring);
            }
        }
    }
}

#[test]
fn prepared_pauses_startup_keys_without_blocking_target_activation() {
    use agend_core::runtime_records::ClaudeStartupKey;
    for cancel in [true, false] {
        let root = TempDir::new("backend-switch-startup").unwrap();
        let store = SqliteStore::open(root.path(), 0).unwrap();
        let (instance, target) = fixture_backend(&store, Backend::Claude);
        let session = instance.session_id.as_deref().unwrap();
        block_on(store.begin_claude_startup(&instance.id, session)).unwrap();
        let startup = block_on(store.claude_startup(&instance.id))
            .unwrap()
            .unwrap();
        let key = ClaudeStartupKey {
            startup,
            generation: "native-generation".into(),
            prompt: "trust_yes".into(),
            attempt: "old-attempt".into(),
        };
        let prepared =
            block_on(store.prepare_backend_switch(&instance, target, "/managed/new/program", None))
                .unwrap();
        assert!(!block_on(store.reserve_claude_startup_key(key.clone())).unwrap());
        if cancel {
            block_on(store.cancel_backend_switch(&prepared)).unwrap();
            // Refusal consumed no intent; the original startup can continue.
            assert!(block_on(store.reserve_claude_startup_key(key)).unwrap());
        } else {
            block_on(store.commit_backend_switch(&prepared, false, 0)).unwrap();
            block_on(store.begin_claude_startup(&instance.id, session)).unwrap();
            // The new launch invalidates an old reserved snapshot.
            assert!(!block_on(store.reserve_claude_startup_key(key.clone())).unwrap());
            let mut next = key;
            next.startup = block_on(store.claude_startup(&instance.id))
                .unwrap()
                .unwrap();
            next.attempt = "new-attempt".into();
            assert!(block_on(store.reserve_claude_startup_key(next)).unwrap());
        }
    }
}

#[test]
fn pending_problem_survives_reopen_rejects_stale_updates_and_clears_on_cancel() {
    let root = TempDir::new("backend-switch-problem").unwrap();
    let home = root.path().join("home");
    let store = SqliteStore::open(&home, 0).unwrap();
    let (instance, target) = fixture(&store);
    let record =
        block_on(store.prepare_backend_switch(&instance, target, "/managed/new/program", None))
            .unwrap();
    let saved =
        block_on(store.backend_switch_problem(&record, "driver disconnected", 100)).unwrap();
    assert_eq!(saved.phase, record.phase);
    assert_eq!(
        block_on(store.instance(&instance.id)).unwrap().unwrap(),
        instance
    );
    assert!(block_on(store.backend_switch_problem(&record, "stale", 200)).is_err());
    drop(store);
    let store = SqliteStore::open(&home, 0).unwrap();
    assert_eq!(
        block_on(store.backend_switch(&instance.id)).unwrap(),
        Some(saved.clone())
    );
    let again = block_on(store.backend_switch_problem(&saved, "driver disconnected", 300)).unwrap();
    assert_eq!(again, saved);
    let cancelled = block_on(store.cancel_backend_switch(&again)).unwrap();
    assert!(cancelled.problem.is_none());
    assert!(block_on(store.backend_switch_problem(&cancelled, "late", 400)).is_err());
}

#[test]
fn activation_deadline_is_atomic_durable_and_renewed_only_for_restore() {
    let root = TempDir::new("backend-switch-deadline").unwrap();
    let store = SqliteStore::open(root.path(), 0).unwrap();
    let (instance, target) = fixture(&store);
    let prepared =
        block_on(store.prepare_backend_switch(&instance, target, "/managed/new/program", None))
            .unwrap();
    assert!(!prepared.activation_expired(u64::MAX));
    let committed = block_on(store.commit_backend_switch(&prepared, false, 100)).unwrap();
    assert_eq!(committed.activation_deadline_unix_ms, Some(300_100));
    assert!(!committed.activation_expired(300_099));
    assert!(committed.activation_expired(300_100));
    assert!(!committed.activation_expired(99));
    drop(store);
    let store = SqliteStore::open(root.path(), 0).unwrap();
    assert_eq!(
        block_on(store.backend_switch(&instance.id)).unwrap(),
        Some(committed.clone())
    );
    let problem = block_on(store.backend_switch_problem(&committed, "late", 400_000)).unwrap();
    assert_eq!(
        problem.activation_deadline_unix_ms,
        committed.activation_deadline_unix_ms
    );
    let rollback = block_on(store.prepare_backend_rollback(&problem)).unwrap();
    assert!(!rollback.activation_expired(u64::MAX));
    let restoring = block_on(store.commit_backend_switch(&rollback, true, 500_000)).unwrap();
    assert_eq!(restoring.activation_deadline_unix_ms, Some(800_000));
    assert!(!restoring.activation_expired(799_999));
    assert!(restoring.activation_expired(800_000));
}
