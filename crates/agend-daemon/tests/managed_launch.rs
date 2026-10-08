//! Real SQLite reservations survive reopening without allowing stale adoption.
use agend_core::{
    model::Backend,
    runtime_records::{Instance, InstanceStatus},
    setup::backend::ImportedBackend,
    traits::HolderLaunch,
};
use agend_daemon::store::SqliteStore;
use agend_testkit::{block_on, tempdir::TempDir};

fn input() -> (Instance, HolderLaunch, ImportedBackend) {
    let instance = Instance {
        id: "managed-1".into(),
        backend: Backend::Codex,
        program: "/managed/codex/program".into(),
        args: vec!["app-server".into()],
        working_directory: "/workspace".into(),
        session_id: Some("session-1".into()),
        status: InstanceStatus::New,
        session_started: false,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "push".into(),
    };
    let launch = HolderLaunch {
        instance_id: instance.id.clone(),
        backend: instance.backend,
        executable: instance.program.clone(),
        args: instance.args.clone(),
        working_directory: instance.working_directory.clone(),
    };
    let artifact = ImportedBackend {
        format: 1,
        backend: "codex".into(),
        version: "0.158.0".into(),
        sha256: "a".repeat(64),
        bytes: 42,
    };
    (instance, launch, artifact)
}

#[test]
fn persisted_intent_requires_cas_and_instance_removal_prevents_old_receipt_adoption() {
    let root = TempDir::new("managed-launch-store").unwrap();
    let home = root.path().join("home");
    let store = SqliteStore::open(&home, 0).unwrap();
    let (instance, launch, artifact) = input();
    block_on(store.add_instance(&instance)).unwrap();
    let first =
        block_on(store.prepare_managed_launch(&instance, &launch, artifact.clone(), None)).unwrap();
    assert_eq!(first.args, launch.args);
    assert_eq!(first.artifact, artifact);
    assert_eq!(first.session_id, instance.session_id);
    // An unknown/lost result does not authorize a second reservation.
    assert!(
        block_on(store.prepare_managed_launch(&instance, &launch, artifact.clone(), None)).is_err()
    );
    drop(store);
    let store = SqliteStore::open(&home, 1).unwrap();
    assert_eq!(
        block_on(store.managed_launch(&instance.id)).unwrap(),
        Some(first.clone())
    );
    let second = block_on(store.prepare_managed_launch(
        &instance,
        &launch,
        artifact.clone(),
        Some(&first.binding),
    ))
    .unwrap();
    assert_ne!(second.binding, first.binding);
    assert!(
        block_on(store.prepare_managed_launch(
            &instance,
            &launch,
            artifact.clone(),
            Some(&first.binding)
        ))
        .is_err()
    );
    assert_eq!(
        block_on(store.managed_launch(&instance.id)).unwrap(),
        Some(second.clone())
    );
    block_on(store.remove_instance(&instance.id)).unwrap();
    block_on(store.add_instance(&instance)).unwrap();
    assert!(
        block_on(store.managed_launch(&instance.id))
            .unwrap()
            .is_none()
    );
    assert!(
        block_on(store.prepare_managed_launch(
            &instance,
            &launch,
            artifact.clone(),
            Some(&second.binding)
        ))
        .is_err()
    );
    let third = block_on(store.prepare_managed_launch(&instance, &launch, artifact, None)).unwrap();
    assert_ne!(third.binding, second.binding);
}

#[test]
fn changed_instance_or_invalid_artifact_cannot_publish_or_replace_an_intent() {
    let root = TempDir::new("managed-launch-refuse").unwrap();
    let store = SqliteStore::open(&root.path().join("home"), 0).unwrap();
    let (instance, launch, artifact) = input();
    assert!(
        block_on(store.prepare_managed_launch(&instance, &launch, artifact.clone(), None)).is_err()
    );
    block_on(store.add_instance(&instance)).unwrap();
    let first =
        block_on(store.prepare_managed_launch(&instance, &launch, artifact.clone(), None)).unwrap();
    let mut wrong = artifact.clone();
    wrong.backend = "claude".into();
    assert!(
        block_on(store.prepare_managed_launch(&instance, &launch, wrong, Some(&first.binding)))
            .is_err()
    );
    let mut wrong = artifact.clone();
    wrong.sha256 = "not-a-digest".into();
    assert!(
        block_on(store.prepare_managed_launch(&instance, &launch, wrong, Some(&first.binding)))
            .is_err()
    );
    block_on(store.set_instance_status(&instance.id, InstanceStatus::Running)).unwrap();
    assert!(
        block_on(store.prepare_managed_launch(&instance, &launch, artifact, Some(&first.binding)))
            .is_err()
    );
    assert_eq!(
        block_on(store.managed_launch(&instance.id)).unwrap(),
        Some(first)
    );
}
