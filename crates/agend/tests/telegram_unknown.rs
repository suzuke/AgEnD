//! Real daemon restart and operator socket disposition without live Telegram.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::{
    protocol::client::AttentionAction,
    telegram::{TelegramDelivery, TelegramDestination, TelegramStore},
    traits::{Notification, NotificationSeverity},
};
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;
use std::{
    path::Path,
    time::{Duration, Instant},
};
#[test]
fn restart_exposes_unknown_without_credentials_and_operator_disposition_is_durable() {
    let lab = lab::Lab::with_prefix(Path::new(env!("CARGO_BIN_EXE_agend")), "g12d-unknown");
    let home = lab.home(0);
    let store = SqliteStore::open(&home, 0).unwrap();
    let row = TelegramDelivery::new(
        "native-unknown".into(),
        TelegramDestination {
            bot_id: 1,
            chat_id: 42,
            topic_id: None,
        },
        Notification {
            severity: NotificationSeverity::Info,
            title: "Unknown".into(),
            body: "original body".into(),
            task_id: None,
        },
        1,
    );
    block_on(store.enqueue_telegram(&row)).unwrap();
    assert!(block_on(store.claim_telegram_part(&row.id, 0)).unwrap());
    drop(store);
    let id = "telegram-delivery:native-unknown";
    for boot in 0..3 {
        let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
        daemon.ready().unwrap();
        let socket = home.join("run/daemon.sock");
        let mut client = agend_client::Client::connect(&socket, None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let visible = client
                .get_fleet()
                .unwrap()
                .attention
                .iter()
                .any(|item| item.attention_id.as_deref() == Some(id));
            if boot == 2 {
                assert!(!visible, "abandoned notification reappeared");
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            if visible {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "wrong unknown attention state on boot {boot}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        if boot == 1 {
            let mut agent =
                agend_client::Client::connect(&socket, Some("untrusted".into())).unwrap();
            assert!(
                matches!(agent.resolve_attention(id, AttentionAction::Abandon), Err(agend_client::ClientError::Daemon { code, .. }) if code == agend_core::protocol::client::error_code::FORBIDDEN)
            );
            assert!(
                client
                    .resolve_attention(id, AttentionAction::Retry)
                    .is_err()
            );
            client
                .resolve_attention(id, AttentionAction::Abandon)
                .unwrap();
            assert!(
                client
                    .resolve_attention(id, AttentionAction::Abandon)
                    .is_err()
            );
        }
        drop(client);
        daemon.interrupt().unwrap();
        let store = SqliteStore::open(&home, 0).unwrap();
        let saved = block_on(store.telegram_delivery(&row.id)).unwrap().unwrap();
        assert!(saved.in_flight && saved.outcome_unknown);
        assert!(saved.message_ids.is_empty());
        assert_eq!(saved.notification, row.notification);
        assert_eq!(saved.abandoned_by_operator.is_some(), boot >= 1);
        assert!(!block_on(store.claim_telegram_part(&row.id, 0)).unwrap());
        drop(store);
    }
    assert!(lab.running_holders().is_empty());
}
