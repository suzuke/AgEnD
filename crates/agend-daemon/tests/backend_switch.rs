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
    let instance = Instance {
        id: "switch-1".into(),
        backend: Backend::Codex,
        program: "/managed/old/program".into(),
        args: vec![],
        working_directory: "/workspace".into(),
        session_id: Some("native-session".into()),
        status: InstanceStatus::Running,
        session_started: true,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "push".into(),
    };
    block_on(store.add_instance(&instance)).unwrap();
    let old = ImportedBackend {
        format: 1,
        backend: "codex".into(),
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
