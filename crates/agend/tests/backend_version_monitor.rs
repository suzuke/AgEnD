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
        diagnostic_contract(&socket, &home, client.daemon().boot_id);
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

fn diagnostic_contract(socket: &Path, home: &Path, boot: Option<u64>) {
    use agend_core::protocol::client::{
        ClientRequest, ClientResponse, CommandResult, OperatorCommand, OperatorData, V1_9,
    };
    let query = |id: &str, caller| {
        agend_client::exchange_once(
            socket,
            caller,
            V1_9,
            &ClientRequest::Operator {
                data: OperatorData {
                    request_id: "snapshot".into(),
                    command: OperatorCommand::BackendDiagnostic {
                        instance_id: id.into(),
                    },
                },
            },
            Instant::now() + Duration::from_secs(3),
        )
    };
    let snapshot = || {
        let ClientResponse::CommandResult { data } = query("version-test", None).unwrap() else {
            panic!("missing result")
        };
        let CommandResult::BackendDiagnostic { data: reply } = data.result else {
            panic!("missing snapshot")
        };
        assert_eq!(Some(reply.boot_id), boot);
        reply.snapshot.unwrap()
    };
    old_protocol_refuses_snapshot(socket);
    let before = snapshot();
    assert_eq!(
        before.configured_program,
        home.join("backend").to_str().unwrap()
    );
    let observed = before.external_version.as_ref().unwrap();
    assert_eq!(observed.latest.as_ref().unwrap().version_output, "fake 2.0");
    assert!(observed.completed_ms.is_some());
    assert!(before.managed_reservation.is_none());
    assert!(matches!(query("version-test", Some("version-test".into())),
        Err(agend_client::ClientError::Daemon { code, .. }) if code == agend_core::protocol::client::error_code::FORBIDDEN));
    assert!(
        matches!(query("absent", None).unwrap(), ClientResponse::CommandResult { data }
        if matches!(&data.result, CommandResult::BackendDiagnostic { data } if data.snapshot.is_none()))
    );
    assert!(query("../invalid", None).is_err());
    assert_eq!(snapshot(), before, "read-only RPC changed durable evidence");
    let out = std::process::Command::new(BIN)
        .env_clear()
        .env("HOME", home)
        .env("AGEND_HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "doctor"])
        .output()
        .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    let row = rows
        .iter()
        .find(|r| r["check"] == "observation/version-test")
        .unwrap();
    let detail = row["detail"].as_str().unwrap();
    assert!(detail.contains("fake 2.0"));
    assert!(detail.contains(&format!("daemon boot {boot:?}")));
    assert!(detail.contains("may predate the current attempt"));
    assert!(detail.contains("live authentication and running daemon binary digest unknown"));
    assert_eq!(row["status"], "warn");
}

fn old_protocol_refuses_snapshot(socket: &Path) {
    use agend_core::protocol::client::{
        ClientHello, ClientRequest, ClientResponse, OperatorCommand, OperatorData, V1_8,
    };
    use std::io::{BufRead, BufReader, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut send = |request: ClientRequest| {
        serde_json::to_writer(&mut stream, &request).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str::<ClientResponse>(&line).unwrap()
    };
    assert!(matches!(
        send(ClientRequest::Hello {
            data: ClientHello {
                supported: vec![V1_8],
                caller: None
            }
        }),
        ClientResponse::Hello { .. }
    ));
    let response = send(ClientRequest::Operator {
        data: OperatorData {
            request_id: "old-diagnostic".into(),
            command: OperatorCommand::BackendDiagnostic {
                instance_id: "version-test".into(),
            },
        },
    });
    assert!(
        matches!(response, ClientResponse::Error { data } if data.code == agend_core::protocol::client::error_code::NOT_SUPPORTED)
    );
}
