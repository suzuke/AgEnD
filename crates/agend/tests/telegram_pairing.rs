//! Operator RPC against the native daemon; no Telegram network or token access.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::{config::SecretRef, protocol::client::*, telegram::pairing::*};
use agend_daemon::store::SqliteStore;
use agend_testkit::{block_on, fake_daemon::ProbeClient, fakes::FakeClock};
use std::{
    path::Path,
    time::{Duration, Instant},
};
const BIN: &str = env!("CARGO_BIN_EXE_agend");
fn request(operation: PairingOperation) -> ClientRequest {
    ClientRequest::Operator {
        data: OperatorData {
            request_id: "pairing-test".into(),
            command: OperatorCommand::TelegramPairing { operation },
        },
    }
}
fn call(
    home: &Path,
    caller: Option<&str>,
    operation: PairingOperation,
) -> Result<ClientResponse, agend_client::ClientError> {
    agend_client::exchange_once(
        &home.join(DAEMON_SOCKET),
        caller.map(str::to_owned),
        V1_9,
        &request(operation),
        Instant::now() + Duration::from_secs(5),
    )
}
fn record(response: Result<ClientResponse, agend_client::ClientError>) -> Option<PairingRecord> {
    match response.unwrap() {
        ClientResponse::CommandResult { data } => {
            assert_eq!(data.request_id, "pairing-test");
            let CommandResult::TelegramPairing { data } = data.result else {
                panic!("wrong result");
            };
            data.map(|r| *r)
        }
        other => panic!("unexpected reply {other:?}"),
    }
}
fn refused(response: Result<ClientResponse, agend_client::ClientError>, code: &str) {
    match response {
        Err(agend_client::ClientError::Daemon { code: actual, .. }) => assert_eq!(actual, code),
        Ok(ClientResponse::Error { data }) => assert_eq!(data.code, code),
        other => panic!("expected refusal: {other:?}"),
    }
}
#[test]
fn native_pairing_rpc_recovers_receipt_and_refuses_agents_and_old_protocol() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g13-pair-rpc");
    let home = lab.home(1);
    std::fs::write(home.join("config.toml"), "# operator-owned configuration\n").unwrap();
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    assert_eq!(record(call(&home, None, PairingOperation::Status)), None);
    refused(
        call(&home, Some("agent"), PairingOperation::Status),
        error_code::FORBIDDEN,
    );
    let mut old = ProbeClient::connect(&home.join(DAEMON_SOCKET)).unwrap();
    let mut hello = ClientRequest::hello_as(None);
    if let ClientRequest::Hello { data } = &mut hello {
        data.supported = vec![V1_8];
    }
    assert!(
        matches!(old.request(&hello).unwrap(), ClientResponse::Hello { data } if data.selected == V1_8)
    );
    refused(
        Ok(old.request(&request(PairingOperation::Status)).unwrap()),
        error_code::NOT_SUPPORTED,
    );
    drop(old);
    daemon.kill9().unwrap();
    let store = SqliteStore::open(&home, 0).unwrap();
    let session = TelegramPairing::new(
        "11111111-1111-4111-8111-111111111111".into(),
        SecretRef::Env("UNUSED_PAIRING_TOKEN".into()),
        123,
        "fixture_bot".into(),
        &FakeClock::new(1000),
    )
    .unwrap();
    let pending = block_on(store.begin_telegram_pairing(&session, None, 1000)).unwrap();
    drop(store);
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    assert_eq!(
        record(call(&home, None, PairingOperation::Status)),
        Some(pending.clone())
    );
    // Expiry is checked before resolving the absent token and before HTTP.
    refused(
        call(
            &home,
            None,
            PairingOperation::Poll {
                id: session.id.clone(),
            },
        ),
        error_code::INVALID_REQUEST,
    );
    refused(
        call(
            &home,
            Some("agent"),
            PairingOperation::Cancel {
                id: session.id.clone(),
            },
        ),
        error_code::FORBIDDEN,
    );
    refused(
        call(&home, None, PairingOperation::Cancel { id: "stale".into() }),
        error_code::INVALID_REQUEST,
    );
    assert_eq!(
        record(call(&home, None, PairingOperation::Status)),
        Some(pending)
    );
    let cancelled = record(call(
        &home,
        None,
        PairingOperation::Cancel {
            id: session.id.clone(),
        },
    ))
    .unwrap();
    assert_eq!(cancelled.phase, PairingPhase::Cancelled);
    let cli = |agent: bool| {
        let mut cmd = std::process::Command::new(BIN);
        cmd.env_clear()
            .env("AGEND_HOME", &home)
            .env("HOME", &home)
            .args(["telegram", "setup", "status", "--json"]);
        if agent {
            cmd.env("AGEND_INSTANCE", "agent");
        }
        cmd.output().unwrap()
    };
    let output = cli(false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let decoded: Option<PairingRecord> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(decoded, Some(cancelled.clone()));
    assert!(!cli(true).status.success());
    let invalid = std::process::Command::new(BIN)
        .env_clear()
        .env("AGEND_HOME", &home)
        .args(["telegram", "setup", "begin", "--token-file", "relative"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());

    daemon.kill9().unwrap();
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    assert_eq!(
        record(call(&home, None, PairingOperation::Status)),
        Some(cancelled)
    );
    refused(
        call(&home, None, PairingOperation::Cancel { id: session.id }),
        error_code::INVALID_REQUEST,
    );
    daemon.interrupt().unwrap();
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        "# operator-owned configuration\n"
    );
}
