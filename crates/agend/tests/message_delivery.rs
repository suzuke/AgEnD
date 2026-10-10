//! Native persisted delivery receipts, without launching backend processes.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_client::{Client, ClientError};
use agend_core::protocol::client::{DAEMON_SOCKET, MessageDeliveryState};
use agend_core::{model::DeliveryState, policy::busy::BusyLevel};
use agend_daemon::store::{NewMessage, SqliteStore};
use agend_testkit::block_on;
use std::{path::Path, time::Duration};
const BIN: &str = env!("CARGO_BIN_EXE_agend");

#[test]
fn native_receipts_preserve_identity_states_and_operator_boundary_across_restart() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g13rc");
    let home = lab.home(0);
    let now = agend_daemon::log::now_unix_ms();
    let store = SqliteStore::open(&home, now).unwrap();
    for (id, state) in [
        ("queued", DeliveryState::Queued),
        ("sent", DeliveryState::Sent),
        ("confirmed", DeliveryState::Confirmed),
        ("failed", DeliveryState::Failed),
    ] {
        block_on(store.claim_message(
            &NewMessage {
                id: id.into(),
                from_instance: "operator".into(),
                to_instance: "canary".into(),
                task_id: None,
                body: "private canary payload".into(),
                level: BusyLevel::Queue,
            },
            now,
        ))
        .unwrap();
        if state != DeliveryState::Queued {
            block_on(store.mark_message_attempted(id, now + 1)).unwrap();
            block_on(store.advance_message(
                id,
                DeliveryState::Sent,
                Some(format!("turn-{id}")),
                now + 2,
            ))
            .unwrap();
            if state != DeliveryState::Sent {
                block_on(store.advance_message(id, state, None, now + 3)).unwrap();
            }
        }
    }
    drop(store);
    for _ in 0..2 {
        let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
        daemon.ready().unwrap();
        let mut client = Client::connect_once(&home.join(DAEMON_SOCKET), None).unwrap();
        for (id, state) in [
            ("queued", MessageDeliveryState::Queued),
            ("sent", MessageDeliveryState::Sent),
            ("confirmed", MessageDeliveryState::Confirmed),
            ("failed", MessageDeliveryState::Failed),
        ] {
            let data = client
                .message_delivery(id, Duration::from_secs(2))
                .unwrap()
                .unwrap();
            assert_eq!(data.message_id, id);
            assert_eq!(data.to_instance, "canary");
            assert_eq!(data.from_instance, "operator");
            assert_eq!(data.state, state);
            assert_eq!(
                data.attempted_at_unix_ms,
                (id != "queued").then_some(now + 1)
            );
            let wire = serde_json::to_string(&data).unwrap();
            assert!(!wire.contains("private canary payload"));
            assert!(!wire.contains("body"));
        }
        assert!(
            client
                .message_delivery("absent", Duration::from_secs(2))
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            client.message_delivery("", Duration::from_secs(2)),
            Err(ClientError::Daemon { .. })
        ));
        let mut agent =
            Client::connect_once(&home.join(DAEMON_SOCKET), Some("canary".into())).unwrap();
        assert!(
            matches!(agent.message_delivery("confirmed", Duration::from_secs(2)),
            Err(ClientError::Daemon { code, .. }) if code == "forbidden")
        );
        drop(agent);
        drop(client);
        daemon.interrupt().unwrap();
        assert!(lab.running_holders().is_empty());
    }
    let store = SqliteStore::open(&home, now).unwrap();
    assert_eq!(
        block_on(store.message("queued")).unwrap().unwrap().state,
        DeliveryState::Queued
    );
    assert_eq!(
        block_on(store.message("sent")).unwrap().unwrap().state,
        DeliveryState::Sent
    );
}

#[test]
fn native_operator_send_is_attributed_bounded_and_deduplicated_without_agent_impersonation() {
    use agend_core::model::Backend;
    use agend_core::protocol::client::*;
    use agend_daemon::store::{Instance, InstanceStatus};
    use std::time::Instant;
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g13op");
    let home = lab.home(0);
    let now = agend_daemon::log::now_unix_ms();
    let store = SqliteStore::open(&home, now).unwrap();
    block_on(store.add_instance(&Instance {
        id: "sink".into(),
        backend: Backend::Claude,
        program: "/never-start".into(),
        args: vec![],
        working_directory: home.display().to_string(),
        session_id: None,
        status: InstanceStatus::Failed,
        session_started: false,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "inbox".into(),
    }))
    .unwrap();
    drop(store);
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    let request = |caller: Option<&str>, command| {
        agend_client::exchange_once(
            &home.join(DAEMON_SOCKET),
            caller.map(str::to_owned),
            V1_7,
            &ClientRequest::Operator {
                data: OperatorData {
                    request_id: "rpc".into(),
                    command,
                },
            },
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap_or_else(|error| match error {
            ClientError::Daemon { code, message } => ClientResponse::Error {
                data: ErrorData {
                    request_id: Some("rpc".into()),
                    code,
                    message,
                },
            },
            other => panic!("unexpected client failure: {other}"),
        })
    };
    assert!(matches!(request(None, OperatorCommand::InstanceAdd {
        instance_id: "@operator".into(), backend: "claude".into(), working_directory: None,
        program: Some("/never-start".into()), args: vec![],
    }), ClientResponse::Error { data } if data.code=="invalid_request"));
    let impersonation = agend_client::exchange_once(
        &home.join(DAEMON_SOCKET),
        Some(OPERATOR_MESSAGE_SENDER.into()),
        V1_7,
        &ClientRequest::Command {
            data: ClientCommandData {
                request_id: "reserved".into(),
                command: AgentCommand::Send {
                    to: "sink".into(),
                    message: "impersonation".into(),
                    level: None,
                    message_id: Some("cce2d39d-d868-4fdc-a9c9-90e3e4c6b4ed".into()),
                },
            },
        },
        Instant::now() + Duration::from_secs(2),
    );
    assert!(matches!(impersonation, Err(ClientError::Daemon { code, .. }) if code=="forbidden"));
    let id = "b9de5e37-793b-4f15-a6f3-8a4bc4fcae02";
    let send = |body: &str| OperatorCommand::SendMessage {
        to: "sink".into(),
        message: body.into(),
        message_id: id.into(),
    };
    for command in [
        OperatorCommand::MessageOutcome {
            message_id: "missing".into(),
        },
        send("hello"),
        OperatorCommand::DriverStatus {
            instance_id: "sink".into(),
        },
    ] {
        assert!(
            matches!(request(Some("sink"), command), ClientResponse::Error { data } if data.code=="forbidden")
        );
    }
    for _ in 0..2 {
        assert!(matches!(
            request(None, send("hello")),
            ClientResponse::CommandResult {
                data: ClientCommandResultData {
                    result: CommandResult::Accepted,
                    ..
                }
            }
        ));
    }
    assert!(
        matches!(request(None, send("changed")), ClientResponse::Error { data } if data.code=="invalid_request")
    );
    assert!(
        matches!(request(None, OperatorCommand::DriverStatus { instance_id: "sink".into() }), ClientResponse::CommandResult { data: ClientCommandResultData { result: CommandResult::DriverStatus { data }, .. } } if data.state==AgentState::Failed)
    );
    for command in [
        OperatorCommand::DriverStatus {
            instance_id: "absent".into(),
        },
        OperatorCommand::SendMessage {
            to: "absent".into(),
            message: "hello".into(),
            message_id: "cbf54c22-d5fd-416a-bbaf-8bb1d5e9d49f".into(),
        },
    ] {
        assert!(
            matches!(request(None,command), ClientResponse::Error { data } if data.code=="unknown_instance")
        );
    }
    daemon.interrupt().unwrap();
    assert!(lab.running_holders().is_empty());
    let store = SqliteStore::open(&home, now).unwrap();
    let message = block_on(store.message(id)).unwrap().unwrap();
    assert_eq!(message.from_instance, OPERATOR_MESSAGE_SENDER);
    assert_eq!(message.body, "hello");
    assert_eq!(message.state, DeliveryState::Queued);
    assert!(
        block_on(store.message("cbf54c22-d5fd-416a-bbaf-8bb1d5e9d49f"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn historical_agent_operator_messages_cannot_be_adopted_as_human_sends() {
    use agend_core::{model::Backend, protocol::client::*};
    use agend_daemon::store::{Instance, InstanceStatus};
    use std::time::Instant;
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g13id");
    let home = lab.home(0);
    let now = agend_daemon::log::now_unix_ms();
    let store = SqliteStore::open(&home, now).unwrap();
    block_on(store.add_instance(&Instance {
        id: "operator".into(),
        backend: Backend::Claude,
        program: "/never-start".into(),
        args: vec![],
        working_directory: home.display().to_string(),
        session_id: None,
        status: InstanceStatus::Failed,
        session_started: false,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "inbox".into(),
    }))
    .unwrap();
    let id = "6e6c095c-f841-4b24-b43c-6c6ed52168dd";
    block_on(store.claim_message(
        &NewMessage {
            id: id.into(),
            from_instance: "operator".into(),
            to_instance: "operator".into(),
            task_id: None,
            body: "same content".into(),
            level: BusyLevel::Queue,
        },
        now,
    ))
    .unwrap();
    drop(store);
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    let send = |message_id: &str| {
        agend_client::exchange_once(
            &home.join(DAEMON_SOCKET),
            None,
            V1_7,
            &ClientRequest::Operator {
                data: OperatorData {
                    request_id: "history".into(),
                    command: OperatorCommand::SendMessage {
                        to: "operator".into(),
                        message: "same content".into(),
                        message_id: message_id.into(),
                    },
                },
            },
            Instant::now() + Duration::from_secs(2),
        )
    };
    assert!(matches!(send(id), Err(ClientError::Daemon { code, .. }) if code=="invalid_request"));
    let fresh = "c5b081bf-eceb-4813-82a2-424b94471008";
    assert!(matches!(
        send(fresh),
        Ok(ClientResponse::CommandResult {
            data: ClientCommandResultData {
                result: CommandResult::Accepted,
                ..
            }
        })
    ));
    daemon.interrupt().unwrap();
    assert!(lab.running_holders().is_empty());
    let store = SqliteStore::open(&home, now).unwrap();
    assert_eq!(
        block_on(store.message(id)).unwrap().unwrap().from_instance,
        "operator"
    );
    assert_eq!(
        block_on(store.message(fresh))
            .unwrap()
            .unwrap()
            .from_instance,
        OPERATOR_MESSAGE_SENDER
    );
    assert!(block_on(store.instance("operator")).unwrap().is_some());
}
