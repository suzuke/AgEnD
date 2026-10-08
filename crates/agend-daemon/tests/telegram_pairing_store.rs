//! Native SQLite pairing publication, stale operators and restart recovery.
use agend_core::{config::SecretRef, telegram::pairing::*, traits::Clock};
use agend_daemon::store::SqliteStore;
use agend_testkit::{block_on, tempdir::TempDir};
struct At(u64);
impl Clock for At {
    fn now_unix_ms(&self) -> u64 {
        self.0
    }
}
fn session(n: u8, now: u64) -> TelegramPairing {
    TelegramPairing::new(
        format!("00000000-0000-4000-8000-{n:012}"),
        SecretRef::Env("TELEGRAM_TEST_TOKEN".into()),
        123,
        "test_bot".into(),
        &At(now),
    )
    .unwrap()
}
fn observed(mut session: TelegramPairing) -> TelegramPairing {
    let command = session.command();
    assert!(
        session
            .observe(
                PairingCandidate {
                    chat_id: 42,
                    user_id: 42,
                    topic_id: None
                },
                &command,
                session.created_at_ms / 1000,
                &At(session.created_at_ms)
            )
            .unwrap()
    );
    session.offset = 101;
    session
}
#[test]
fn candidate_and_cursor_survive_reopen_without_auto_confirmation() {
    let dir = TempDir::new("telegram-pair-store").unwrap();
    let store = SqliteStore::open(dir.path(), 0).unwrap();
    let pending = block_on(store.begin_telegram_pairing(&session(1, 1000), None, 1000)).unwrap();
    let found = observed(pending.session.clone());
    let published = block_on(store.observe_telegram_pairing(&pending, &found, 1000)).unwrap();
    drop(store);
    let store = SqliteStore::open(dir.path(), 0).unwrap();
    assert_eq!(
        block_on(store.telegram_pairing()).unwrap(),
        Some(published.clone())
    );
    assert_eq!(published.phase, PairingPhase::Pending);
    assert_eq!(published.session.offset, 101);
    assert!(block_on(store.cancel_telegram_pairing(&pending)).is_err());
    assert!(block_on(store.observe_telegram_pairing(&pending, &found, 1000)).is_err());
    let candidate = found.candidate.unwrap();
    let mut wrong = candidate.clone();
    wrong.user_id += 1;
    assert!(block_on(store.confirm_telegram_pairing(&published, &wrong, 1000)).is_err());
    let confirmed = block_on(store.confirm_telegram_pairing(&published, &candidate, 1000)).unwrap();
    assert!(block_on(store.confirm_telegram_pairing(&published, &candidate, 1000)).is_err());
    drop(store);
    let store = SqliteStore::open(dir.path(), 0).unwrap();
    block_on(store.prune(1_000_000_000)).unwrap();
    assert_eq!(block_on(store.telegram_pairing()).unwrap(), Some(confirmed));
}
#[test]
fn observations_cannot_change_identity_destination_or_regress_cursor() {
    let dir = TempDir::new("telegram-pair-store").unwrap();
    let store = SqliteStore::open(dir.path(), 0).unwrap();
    let pending = block_on(store.begin_telegram_pairing(&session(1, 1000), None, 1000)).unwrap();
    let found = observed(pending.session.clone());
    for field in 0..7 {
        let mut bad = found.clone();
        match field {
            0 => bad.id = session(2, 1000).id,
            1 => bad.token = SecretRef::Env("OTHER_TOKEN".into()),
            2 => bad.bot_id += 1,
            3 => bad.bot_username = "other_bot".into(),
            4 => bad.expires_at_ms += 1,
            5 => bad.created_at_ms += 1,
            _ => bad.offset = 0,
        }
        assert!(
            block_on(store.observe_telegram_pairing(&pending, &bad, 1000)).is_err(),
            "field {field}"
        );
        assert_eq!(
            block_on(store.telegram_pairing()).unwrap(),
            Some(pending.clone())
        );
    }
    let published = block_on(store.observe_telegram_pairing(&pending, &found, 1000)).unwrap();
    let mut bad = found.clone();
    bad.offset -= 1;
    assert!(block_on(store.observe_telegram_pairing(&published, &bad, 1000)).is_err());
    bad = found;
    bad.offset += 1;
    bad.candidate.as_mut().unwrap().user_id += 1;
    assert!(block_on(store.observe_telegram_pairing(&published, &bad, 1000)).is_err());
}
#[test]
fn expiry_and_explicit_replacement_prevent_old_operator_actions() {
    let dir = TempDir::new("telegram-pair-store").unwrap();
    let store = SqliteStore::open(dir.path(), 0).unwrap();
    let pending = block_on(store.begin_telegram_pairing(&session(1, 1000), None, 1000)).unwrap();
    assert!(
        block_on(store.begin_telegram_pairing(&session(2, 1000), Some(&pending.session.id), 1000))
            .is_err()
    );
    let found = observed(pending.session.clone());
    let expiry = found.expires_at_ms;
    assert!(block_on(store.observe_telegram_pairing(&pending, &found, expiry)).is_err());
    let published = block_on(store.observe_telegram_pairing(&pending, &found, 1000)).unwrap();
    assert!(
        block_on(store.confirm_telegram_pairing(
            &published,
            found.candidate.as_ref().unwrap(),
            expiry
        ))
        .is_err()
    );
    assert!(block_on(store.begin_telegram_pairing(&session(2, expiry), None, expiry)).is_err());
    let next = block_on(store.begin_telegram_pairing(
        &session(2, expiry),
        Some(&published.session.id),
        expiry,
    ))
    .unwrap();
    assert!(block_on(store.cancel_telegram_pairing(&published)).is_err());
    let cancelled = block_on(store.cancel_telegram_pairing(&next)).unwrap();
    assert!(
        block_on(store.begin_telegram_pairing(
            &session(2, expiry),
            Some(&cancelled.session.id),
            expiry
        ))
        .is_err()
    );
    block_on(store.begin_telegram_pairing(
        &session(3, expiry),
        Some(&cancelled.session.id),
        expiry,
    ))
    .unwrap();
}

#[test]
fn concurrent_cancel_and_observation_have_only_one_winner() {
    let dir = TempDir::new("telegram-pair-store").unwrap();
    let store = SqliteStore::open(dir.path(), 0).unwrap();
    let pending = block_on(store.begin_telegram_pairing(&session(1, 1000), None, 1000)).unwrap();
    let found = observed(pending.session.clone());
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let cancel = scope.spawn(|| {
            barrier.wait();
            block_on(store.cancel_telegram_pairing(&pending))
        });
        let publish = scope.spawn(|| {
            barrier.wait();
            block_on(store.observe_telegram_pairing(&pending, &found, 1000))
        });
        let cancel = cancel.join().unwrap();
        let publish = publish.join().unwrap();
        assert_ne!(cancel.is_ok(), publish.is_ok());
        let winner = cancel.or(publish).unwrap();
        assert_eq!(block_on(store.telegram_pairing()).unwrap(), Some(winner));
    });
}
