//! Real daemon boots restore disk-version reminders; only local fake CLI runs.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::{
    model::Backend,
    protocol::client::AttentionAction,
    runtime_records::{Instance, InstanceStatus},
};
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};
const BIN: &str = env!("CARGO_BIN_EXE_agend");
fn script(home: &Path, version: &str) {
    let program = home.join("backend");
    fs::write(
        &program,
        format!("#!/bin/sh\n[ \"$1\" = --version ] || exit 9\nprintf '{version}\\n'\n"),
    )
    .unwrap();
    fs::set_permissions(program, fs::Permissions::from_mode(0o700)).unwrap();
}
#[test]
fn native_daemon_detects_changed_disk_version_and_retains_exact_ack_across_three_boots() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g13-version-restart");
    let home = lab.home(0).canonicalize().unwrap();
    fs::write(
        home.join("config.toml"),
        "registry_checks = false\nbackend_version_checks = true\n",
    )
    .unwrap();
    script(&home, "fake 1.0");
    let program = home.join("backend");
    let store = SqliteStore::open(&home, 100).unwrap();
    store_setup(&store, &home, &program);
    let ticket = block_on(store.begin_system_version_check("version-test", 100))
        .unwrap()
        .unwrap();
    let baseline = agend_daemon::backend_versions::system_version::observe(
        &home,
        Backend::Codex,
        program.to_str().unwrap(),
        &home,
        &BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
    )
    .unwrap()
    .unwrap();
    assert!(block_on(store.finish_system_version_check(&ticket, 100, Ok(baseline))).unwrap());
    drop(store);
    script(&home, "fake 2.0");
    let mut attention_id = None;
    for boot in 0..3 {
        let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
        daemon.ready().unwrap();
        let socket = home.join("run/daemon.sock");
        let mut client = agend_client::Client::connect_once(&socket, None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let item = client.get_fleet().unwrap().attention.into_iter().find(|a| {
                a.attention_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with("backend-version:"))
            });
            if boot == 2 {
                assert!(item.is_none(), "acknowledged version reappeared");
                if Instant::now() >= deadline {
                    break;
                }
            } else if let Some(item) = item {
                assert!(item.reason.contains("fake 2.0"));
                assert!(item.reason.contains("does not identify the running holder"));
                let id = item.attention_id.unwrap();
                if boot == 0 {
                    attention_id = Some(id)
                } else {
                    assert_eq!(attention_id.as_deref(), Some(id.as_str()));
                    let mut agent =
                        agend_client::Client::connect_once(&socket, Some("version-test".into()))
                            .unwrap();
                    assert!(
                        matches!(agent.resolve_attention(&id,AttentionAction::Acknowledge),Err(agend_client::ClientError::Daemon {code,..}) if code==agend_core::protocol::client::error_code::FORBIDDEN)
                    );
                    assert!(
                        client
                            .resolve_attention(&id, AttentionAction::Retry)
                            .is_err()
                    );
                    client
                        .resolve_attention(&id, AttentionAction::Acknowledge)
                        .unwrap();
                }
                break;
            } else {
                assert!(
                    Instant::now() < deadline,
                    "no version reminder on boot {boot}"
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        drop(client);
        daemon.interrupt().unwrap();
        let store = SqliteStore::open(&home, 0).unwrap();
        let row = block_on(store.system_version_observation("version-test"))
            .unwrap()
            .unwrap();
        assert_eq!(row.latest.unwrap().version_output, "fake 2.0");
        assert_eq!(row.revision, 1);
        assert_eq!(row.acknowledged_revision, u64::from(boot >= 1));
        assert!(
            block_on(store.registry_observation(Backend::Codex))
                .unwrap()
                .is_none()
        );
    }
    assert!(lab.running_holders().is_empty());
}
fn store_setup(store: &SqliteStore, home: &Path, program: &Path) {
    block_on(store.add_instance(&Instance {
        id: "version-test".into(),
        backend: Backend::Codex,
        program: program.to_str().unwrap().into(),
        args: vec![],
        working_directory: home.to_str().unwrap().into(),
        session_id: None,
        status: InstanceStatus::Failed,
        session_started: false,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "push".into(),
    }))
    .unwrap();
}
