//! Native daemon/CLI reconciliation of an interrupted prepared switch.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::{
    model::Backend,
    runtime_records::{Instance, InstanceStatus},
    setup::backend::ImportedBackend,
    traits::HolderLaunch,
};
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;
use std::{
    path::Path,
    process::{Command, Output},
};
const BIN: &str = env!("CARGO_BIN_EXE_agend");
fn cli(home: &Path, args: &[&str], agent: bool) -> Output {
    let mut command = Command::new(BIN);
    command
        .env_clear()
        .env("AGEND_HOME", home)
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin");
    if agent {
        command.env("AGEND_INSTANCE", "managed");
    }
    command
        .args(["backend", "switch"])
        .args(args)
        .arg("--json")
        .output()
        .unwrap()
}
fn value(out: Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn native_status_cancel_and_restart_preserve_program_and_refuse_agents_and_stale_ids() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g13-switch-rpc");
    let home = lab.home(1);
    let store = SqliteStore::open(&home, 0).unwrap();
    let instance = Instance {
        id: "managed".into(),
        backend: Backend::Claude,
        program: "/managed/old/program".into(),
        args: vec![],
        working_directory: home.to_string_lossy().into_owned(),
        session_id: Some("11111111-1111-4111-8111-111111111111".into()),
        status: InstanceStatus::Failed,
        session_started: true,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "push".into(),
    };
    block_on(store.add_instance(&instance)).unwrap();
    let old = ImportedBackend {
        format: 1,
        backend: "claude".into(),
        version: "old".into(),
        sha256: "a".repeat(64),
        bytes: 42,
    };
    block_on(store.prepare_managed_launch(
        &instance,
        &HolderLaunch {
            instance_id: instance.id.clone(),
            backend: instance.backend,
            executable: instance.program.clone(),
            args: vec![],
            working_directory: instance.working_directory.clone(),
        },
        old.clone(),
        None,
    ))
    .unwrap();
    let record = block_on(store.prepare_backend_switch(
        &instance,
        ImportedBackend {
            version: "new".into(),
            sha256: "b".repeat(64),
            ..old
        },
        "/managed/new/program",
        None,
    ))
    .unwrap();
    let empty = Instance {
        id: "empty".into(),
        ..instance.clone()
    };
    block_on(store.add_instance(&empty)).unwrap();
    drop(store);
    // Failed fixtures deliberately start no backend; this checks recovery of
    // the native persisted request, not successful canary admission or upgrade.
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    assert_eq!(
        value(cli(&home, &["status", "managed"], false))["phase"],
        "prepared"
    );
    assert!(value(cli(&home, &["status", "empty"], false)).is_null());
    // Exercise the deadline-aware decoder against the real daemon producer,
    // including the nullable result which the ordinary CLI decoder also reads.
    for (id, expected) in [("managed", Some(record.clone())), ("empty", None)] {
        use agend_core::protocol::client::{
            BackendSwitchCommand, ClientRequest, ClientResponse, CommandResult, DAEMON_SOCKET,
            OperatorCommand, OperatorData, V1_8,
        };
        let response = agend_client::exchange_once(
            &home.join(DAEMON_SOCKET),
            None,
            V1_8,
            &ClientRequest::Operator {
                data: OperatorData {
                    request_id: format!("once-{id}"),
                    command: OperatorCommand::BackendSwitch {
                        operation: BackendSwitchCommand::Status {
                            instance_id: id.into(),
                        },
                    },
                },
            },
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        )
        .unwrap();
        let ClientResponse::CommandResult { data } = response else {
            panic!("unexpected status response: {response:?}");
        };
        assert_eq!(
            data.result,
            CommandResult::BackendSwitch {
                data: expected.map(Box::new)
            }
        );
    }

    let denied = cli(&home, &["status", "managed"], true);
    assert!(!denied.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&denied.stdout).unwrap()["error"]["code"],
        "forbidden"
    );
    assert!(
        !cli(
            &home,
            &[
                "cancel",
                "managed",
                "--switch-id",
                "33333333-3333-4333-8333-333333333333"
            ],
            false
        )
        .status
        .success()
    );
    assert_eq!(
        value(cli(&home, &["status", "managed"], false))["id"],
        record.id
    );
    assert_eq!(
        value(cli(
            &home,
            &["cancel", "managed", "--switch-id", &record.id],
            false
        ))["phase"],
        "cancelled"
    );
    assert!(
        !cli(
            &home,
            &["cancel", "managed", "--switch-id", &record.id],
            false
        )
        .status
        .success()
    );
    assert!(
        !cli(
            &home,
            &[
                "prepare",
                "managed",
                "--version",
                "new",
                "--previous",
                &record.id
            ],
            false
        )
        .status
        .success()
    );
    daemon.interrupt().unwrap();
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    assert_eq!(
        value(cli(&home, &["status", "managed"], false))["phase"],
        "cancelled"
    );
    daemon.interrupt().unwrap();
    let store = SqliteStore::open(&home, 1).unwrap();
    assert_eq!(
        block_on(store.instance("managed"))
            .unwrap()
            .unwrap()
            .program,
        instance.program
    );
    assert!(lab.running_holders().is_empty());
}
