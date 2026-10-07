//! Native socket producer, recorded Telegram shape, real SQLite and NTF suite.
use super::{config::Token, delivery::TelegramNotifier, http::Api};
use crate::store::SqliteStore;
use agend_core::{
    telegram::{TelegramDestination, TelegramStore},
    traits::{Notification, NotificationSeverity},
};
use agend_testkit::{
    contract::notifier::{self, NotifierFixture},
    tempdir::TempDir,
};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

struct Server {
    origin: String,
    received: Arc<Mutex<Vec<String>>>,
    requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(lose_reply_at: Option<usize>, corrupt_at: Option<usize>) -> Self {
        Self::with_pause(lose_reply_at, corrupt_at, false)
    }
    fn with_pause(lose_reply_at: Option<usize>, corrupt_at: Option<usize>, paused: bool) -> Self {
        let pause = Arc::new(AtomicBool::new(paused));
        let held = pause.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let output = received.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_log = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let ending = stop.clone();
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::SeqCst) {
                let (stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                // Accepted sockets inherit O_NONBLOCK on macOS.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                assert!(
                    request_line.starts_with("POST /bot123:abcdefghijklmnopqrstuvwxyz_123456789/")
                );
                let get_me = request_line.contains("/getMe ");
                assert!(get_me || request_line.contains("/sendMessage "));
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                if get_me {
                    assert_eq!(request, serde_json::json!({}));
                    let body = include_str!("../../tests/fixtures/telegram/get-me.json");
                    write!(
                        reader.get_mut(),
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    continue;
                }
                request_log.lock().unwrap().push(request.clone());
                assert_eq!(request["chat_id"], 42);
                assert!(request.get("parse_mode").is_none());
                let text = request["text"].as_str().unwrap();
                assert!(text.encode_utf16().count() <= 4096);
                let index = {
                    let mut out = output.lock().unwrap();
                    out.push(text.to_owned());
                    out.len()
                };
                if index == 1 {
                    while held.load(Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
                if lose_reply_at == Some(index) {
                    continue;
                }
                let mut reply: Value = serde_json::from_str(include_str!(
                    "../../tests/fixtures/telegram/message.json"
                ))
                .unwrap();
                reply["result"]["message_id"] = (index as i64).into();
                reply["result"]["text"] = if corrupt_at == Some(index) {
                    "truncated".into()
                } else {
                    text.into()
                };
                if let Some(topic) = request.get("message_thread_id") {
                    reply["result"]["message_thread_id"] = topic.clone();
                }
                let body = reply.to_string();
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        Self {
            origin,
            pause,
            received,
            requests,
            stop,
            thread: Some(thread),
        }
    }
    fn api(&self) -> Arc<Api> {
        Arc::new(Api::with_origin(
            Token::parse("123:abcdefghijklmnopqrstuvwxyz_123456789".into()).unwrap(),
            self.origin.clone(),
        ))
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.pause.store(false, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
        let result = self.thread.take().unwrap().join();
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}
fn destination() -> TelegramDestination {
    TelegramDestination {
        bot_id: 123456789,
        chat_id: 42,
        topic_id: None,
    }
}
struct Fixture {
    notifier: TelegramNotifier<SqliteStore>,
    server: Server,
    _dir: TempDir,
}
impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new("telegram-ntf").unwrap();
        let server = Server::new(None, None);
        let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
        let notifier = TelegramNotifier::new(server.api(), store, destination());
        Self {
            notifier,
            server,
            _dir: dir,
        }
    }
}
impl NotifierFixture for Fixture {
    type Notifier = TelegramNotifier<SqliteStore>;
    type Error = String;
    fn notifier(&self) -> &Self::Notifier {
        &self.notifier
    }
    fn received(&self) -> Vec<Notification> {
        // Decode only channel-observed text, never the sender's DB or notification.
        let mut assembled = Vec::new();
        let mut payload = String::new();
        let mut expected = 1;
        for part in self.server.received.lock().unwrap().iter() {
            let (header, body) = part.split_once('\n').unwrap();
            let (index, total) = header
                .strip_prefix("AgEnD · ")
                .unwrap()
                .split_once('/')
                .unwrap();
            let index = index.parse::<usize>().unwrap();
            let total = total.parse::<usize>().unwrap();
            assert_eq!(index, expected);
            payload.push_str(body.strip_suffix("\n——").unwrap());
            if index == total {
                assembled.push(std::mem::take(&mut payload));
                expected = 1;
            } else {
                expected += 1;
            }
        }
        assert!(payload.is_empty());
        assembled
            .into_iter()
            .map(|text| {
                let (header, body) = text.split_once("\n\n").unwrap();
                let (severity, rest) = header.split_once("\nTitle: ").unwrap();
                let (title, task) = rest.split_once("\nTask: ").unwrap();
                Notification {
                    severity: match severity {
                        "[Info]" => NotificationSeverity::Info,
                        "[Attention]" => NotificationSeverity::Attention,
                        "[Error]" => NotificationSeverity::Error,
                        _ => panic!("unknown severity"),
                    },
                    title: title.into(),
                    body: body.into(),
                    task_id: (task != "—").then(|| task.into()),
                }
            })
            .collect()
    }
}
#[test]
fn native_telegram_notifier_satisfies_all_ntf_contracts() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _entered = rt.enter();
    let report = notifier::run("Telegram/native HTTP + SQLite", Fixture::new);
    assert!(report.all_passed(), "{report:#?}");
}

#[tokio::test]
async fn long_unicode_parts_keep_all_content_and_finished_delivery_never_sends_again() {
    let fixture = Fixture::new();
    let note = Notification {
        severity: NotificationSeverity::Attention,
        title: "  padded \t".into(),
        body: "\n  é 繁中 ✅ 🧑‍🔧\n".repeat(1700),
        task_id: Some("T-long".into()),
    };
    fixture
        .notifier
        .notify_with_id("long", &note, 1)
        .await
        .unwrap();
    assert_eq!(fixture.received(), vec![note.clone()]);
    let count = fixture.server.received.lock().unwrap().len();
    assert!(count > 4);
    fixture.notifier.resume("long").await.unwrap();
    fixture
        .notifier
        .notify_with_id("long", &note, 2)
        .await
        .unwrap();
    assert_eq!(fixture.server.received.lock().unwrap().len(), count);
}

#[tokio::test]
async fn unknown_or_corrupted_receipt_stops_parts_and_never_replays_after_db_reopen() {
    for corrupt in [false, true] {
        let dir = TempDir::new("telegram-unknown").unwrap();
        let server = Server::new((!corrupt).then_some(2), corrupt.then_some(2));
        let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
        let notifier = TelegramNotifier::new(server.api(), store.clone(), destination());
        let note = Notification {
            severity: NotificationSeverity::Error,
            title: "multi".into(),
            body: "x".repeat(13000),
            task_id: None,
        };
        assert!(notifier.notify_with_id("unknown", &note, 1).await.is_err());
        let row = store.telegram_delivery("unknown").await.unwrap().unwrap();
        assert_eq!(row.next_part, 1);
        assert!(row.in_flight);
        drop(notifier);
        drop(store);
        let store = Arc::new(SqliteStore::open(dir.path(), 1).unwrap());
        let notifier = TelegramNotifier::new(server.api(), store, destination());
        assert!(
            notifier
                .resume("unknown")
                .await
                .unwrap_err()
                .contains("unknown")
        );
        assert_eq!(server.received.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn changed_bot_token_is_refused_before_sending_or_claiming_a_pending_part() {
    let dir = TempDir::new("telegram-identity").unwrap();
    let server = Server::new(None, None);
    let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
    let mut expected = destination();
    expected.bot_id += 1;
    let notifier = TelegramNotifier::new(server.api(), store.clone(), expected);
    let note = Notification {
        severity: NotificationSeverity::Info,
        title: "identity".into(),
        body: "must not send".into(),
        task_id: None,
    };
    assert!(
        notifier
            .notify_with_id("identity", &note, 1)
            .await
            .unwrap_err()
            .contains("different bot")
    );
    assert!(server.received.lock().unwrap().is_empty());
    let row = store.telegram_delivery("identity").await.unwrap().unwrap();
    assert!(!row.in_flight);
    assert_eq!(row.next_part, 0);
}

#[tokio::test]
async fn separate_notifiers_competing_for_one_delivery_publish_only_once() {
    let dir = TempDir::new("telegram-concurrent").unwrap();
    let server = Server::new(None, None);
    let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
    let first = TelegramNotifier::new(server.api(), store.clone(), destination());
    let second = TelegramNotifier::new(server.api(), store.clone(), destination());
    let note = Notification {
        severity: NotificationSeverity::Info,
        title: "concurrent".into(),
        body: "one notification".into(),
        task_id: None,
    };
    let (a, b) = tokio::join!(
        first.notify_with_id("same", &note, 1),
        second.notify_with_id("same", &note, 1)
    );
    assert!(a.is_ok() || b.is_ok());
    assert_eq!(server.received.lock().unwrap().len(), 1);
    assert!(
        store
            .telegram_delivery("same")
            .await
            .unwrap()
            .unwrap()
            .complete()
    );
    first.resume("same").await.unwrap();
    second.resume("same").await.unwrap();
    assert_eq!(server.received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn native_worker_observes_fleet_and_finishes_receipt_before_shutdown() {
    use agend_core::protocol::client::{AttentionAction, AttentionRequiredData};
    let dir = TempDir::new("telegram-worker").unwrap();
    let server = Server::new(None, None);
    let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
    let fleet = Arc::new(crate::fleet::Fleet::new(1));
    let item = AttentionRequiredData {
        reason: "Approve 繁中 🧑‍🔧".into(),
        task_id: Some("t-1".into()),
        ask: None,
        recap: None,
        attention_id: Some("approval:t-1/approve/1".into()),
        unblocks: Some(1),
        waiting_since_unix_ms: Some(1),
        if_ignored: Some("Task waits".into()),
        actions: vec![AttentionAction::Approve, AttentionAction::RequestChanges],
        instance_id: None,
    };
    fleet.upsert_attention(item.clone());
    let config = agend_core::config::TelegramConfig {
        token: agend_core::config::SecretRef::Env("NOT_USED".into()),
        chat_id: 42,
        allow_user_ids: vec![],
        needs_you_topic: None,
        team_topics: Default::default(),
    };
    let worker = super::worker::start_with_api(config, server.api(), store.clone(), fleet);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if server.received.lock().unwrap().len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    worker.stop().await;
    let mut rebooted = item;
    rebooted.waiting_since_unix_ms = Some(9999);
    let notice = super::worker::notice(&rebooted).unwrap();
    let rows = store
        .observe_telegram(&[notice], &destination(), 2)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].complete());
    assert!(
        rows[0]
            .notification
            .body
            .contains("approve, request_changes")
    );
    assert_eq!(server.received.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn stop_and_resolved_attention_never_start_another_part_after_active_receipt() {
    use agend_core::protocol::client::AttentionRequiredData;
    for mode in 0..3 {
        let stopping = mode == 0;
        let dir = TempDir::new("telegram-stop-part").unwrap();
        let server = Server::with_pause(None, None, true);
        let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
        let fleet = Arc::new(crate::fleet::Fleet::new(1));
        let item = AttentionRequiredData {
            reason: "繁中 🧑‍🔧 ".repeat(2000),
            task_id: None,
            ask: None,
            recap: None,
            attention_id: Some("held".into()),
            unblocks: None,
            waiting_since_unix_ms: Some(1),
            if_ignored: None,
            actions: vec![],
            instance_id: None,
        };
        fleet.upsert_attention(item.clone());
        let initial = store
            .observe_telegram(&[super::worker::notice(&item).unwrap()], &destination(), 1)
            .await
            .unwrap()
            .remove(0);
        let config = agend_core::config::TelegramConfig {
            token: agend_core::config::SecretRef::Env("NOT_USED".into()),
            chat_id: 42,
            allow_user_ids: vec![],
            needs_you_topic: None,
            team_topics: Default::default(),
        };
        let worker =
            super::worker::start_with_api(config, server.api(), store.clone(), fleet.clone());
        tokio::time::timeout(Duration::from_secs(3), async {
            while server.received.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        if stopping {
            worker.request_stop();
        } else if mode == 1 {
            fleet.dismiss("held");
        } else {
            let mut updated = item;
            updated.reason.push_str("\nChanged question");
            fleet.upsert_attention(updated);
        }
        server.pause.store(false, Ordering::SeqCst);
        if !stopping {
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if store
                        .telegram_delivery(&initial.id)
                        .await
                        .unwrap()
                        .unwrap()
                        .abandoned
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        worker.stop().await;
        let row = store.telegram_delivery(&initial.id).await.unwrap().unwrap();
        assert_eq!(row.next_part, 1);
        assert!(!row.in_flight);
        assert_eq!(row.abandoned, !stopping);
        if mode == 2 {
            let sent = server.received.lock().unwrap();
            assert!(sent.len() <= 2);
            assert!(
                sent.iter().all(|part| part.starts_with("AgEnD · 1/")),
                "old continuation was sent"
            );
        } else {
            assert_eq!(server.received.lock().unwrap().len(), 1);
        }
        if stopping {
            let notifier = TelegramNotifier::new(server.api(), store.clone(), destination());
            notifier.resume(&initial.id).await.unwrap();
            assert!(
                store
                    .telegram_delivery(&initial.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .complete()
            );
            assert_eq!(server.received.lock().unwrap().len(), row.parts.len());
        }
    }
}

#[tokio::test]
async fn worker_routes_team_summaries_separately_and_does_not_repeat_them_after_reopen() {
    use agend_core::{
        config::{SecretRef, TelegramConfig},
        protocol::client::{AttentionRequiredData, TaskView},
    };
    let dir = TempDir::new("telegram-topics").unwrap();
    let server = Server::new(None, None);
    let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
    let fleet = Arc::new(crate::fleet::Fleet::new(1));
    fleet.set_tasks(vec![TaskView {
        task_id: "t-1".into(),
        title: "Keep complete 繁中 summary".into(),
        team_id: "alpha".into(),
        assignee: None,
        status: "running".into(),
        current_stage: Some("checks".into()),
        stages: vec!["checks".into()],
        pipeline: None,
    }]);
    fleet.raise(AttentionRequiredData {
        reason: "Needs human".into(),
        task_id: None,
        ask: None,
        recap: None,
        attention_id: Some("manual".into()),
        unblocks: None,
        waiting_since_unix_ms: None,
        if_ignored: None,
        actions: vec![],
        instance_id: None,
    });
    let config = TelegramConfig {
        token: SecretRef::Env("UNUSED".into()),
        chat_id: 42,
        allow_user_ids: vec![],
        needs_you_topic: Some(10),
        team_topics: [("alpha".into(), 20), ("beta".into(), 30)].into(),
    };
    let worker =
        super::worker::start_with_api(config.clone(), server.api(), store.clone(), fleet.clone());
    wait_sends(&server, 3).await;
    worker.stop().await;
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 3);
    for (topic, text) in [
        (10, "Needs human"),
        (20, "Keep complete 繁中 summary"),
        (30, "No tasks."),
    ] {
        let req = requests
            .iter()
            .find(|v| v["message_thread_id"] == topic)
            .unwrap();
        assert!(req["text"].as_str().unwrap().contains(text));
        if topic != 10 {
            assert!(req.get("reply_markup").is_none());
        }
    }
    drop(store);
    let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
    let worker = super::worker::start_with_api(config, server.api(), store, fleet.clone());
    tokio::time::sleep(Duration::from_millis(1150)).await;
    assert_eq!(server.requests.lock().unwrap().len(), 3);
    let mut tasks = fleet.view().tasks;
    tasks[0].status = "done".into();
    fleet.sync_tasks(tasks);
    wait_sends(&server, 4).await;
    worker.stop().await;
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[3]["message_thread_id"], 20);
    assert!(
        requests[3]["text"]
            .as_str()
            .unwrap()
            .contains("Status: done")
    );
}

async fn wait_sends(server: &Server, count: usize) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while server.received.lock().unwrap().len() < count {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn worker_recovers_unsent_auxiliary_rows_but_never_replays_unknown_or_foreign_rows() {
    use agend_core::{
        config::{SecretRef, TelegramConfig},
        telegram::TelegramDelivery,
    };
    let dir = TempDir::new("telegram-auxiliary").unwrap();
    let server = Server::new(None, None);
    let store = SqliteStore::open(dir.path(), 0).unwrap();
    let note = Notification {
        severity: NotificationSeverity::Info,
        title: "Action result".into(),
        body: "Accepted".into(),
        task_id: None,
    };
    for id in ["unknown", "foreign", "pending"] {
        let mut target = destination();
        if id == "foreign" {
            target.bot_id += 1;
        }
        store
            .enqueue_telegram(&TelegramDelivery::new(id.into(), target, note.clone(), 0))
            .await
            .unwrap();
    }
    assert!(store.claim_telegram_part("unknown", 0).await.unwrap());
    drop(store);
    let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
    let config = TelegramConfig {
        token: SecretRef::Env("UNUSED".into()),
        chat_id: 42,
        allow_user_ids: vec![],
        needs_you_topic: None,
        team_topics: Default::default(),
    };
    let worker = super::worker::start_with_api(
        config,
        server.api(),
        store.clone(),
        Arc::new(crate::fleet::Fleet::new(1)),
    );
    wait_sends(&server, 1).await;
    worker.stop().await;
    assert!(
        store
            .telegram_delivery("pending")
            .await
            .unwrap()
            .unwrap()
            .complete()
    );
    assert!(
        store
            .telegram_delivery("unknown")
            .await
            .unwrap()
            .unwrap()
            .in_flight
    );
    assert!(
        !store
            .telegram_delivery("foreign")
            .await
            .unwrap()
            .unwrap()
            .complete()
    );
    assert_eq!(server.received.lock().unwrap().len(), 1);
}
