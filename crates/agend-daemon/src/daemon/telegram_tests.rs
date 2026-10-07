//! Child-process daemon lifecycle against a loopback Telegram producer.
use super::*;
use agend_core::{
    config::{SecretRef, TelegramConfig},
    telegram::{TelegramDelivery, TelegramDestination, TelegramStore},
    traits::{Notification, NotificationSeverity},
};
use agend_testkit::{block_on, tempdir::TempDir};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
};

const TOKEN: &str = "123:abcdefghijklmnopqrstuvwxyz_123456789";

#[test]
#[ignore = "child entry point; launched by the lifecycle test"]
fn child() {
    let home = PathBuf::from(std::env::var_os("AGEND_TELEGRAM_TEST_HOME").unwrap());
    let origin = std::env::var("AGEND_TELEGRAM_TEST_ORIGIN").unwrap();
    let config = TelegramConfig {
        token: SecretRef::Env("UNUSED".into()),
        chat_id: 42,
        allow_user_ids: vec![7],
        needs_you_topic: None,
        team_topics: Default::default(),
    };
    let token = crate::notifier::config::Token::parse(TOKEN.into()).unwrap();
    let api = Arc::new(crate::notifier::http::Api::local_test(
        crate::notifier::config::Token::parse(TOKEN.into()).unwrap(),
        origin,
    ));
    let store = SqliteStore::open(&home, 0).unwrap();
    let _signals = stop_flag::install().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let stopped = runtime
        .block_on(serve(
            home,
            std::env::current_exe().unwrap(),
            store,
            agend_core::policy::codex_input::CodexInputPolicy::approved(),
            Some((config, token)),
            Some(api),
        ))
        .unwrap();
    assert!(matches!(stopped, Stopped::Signal("SIGINT")));
    runtime.shutdown_timeout(Duration::from_secs(1));
}

struct Process(Child);
impl Process {
    fn start(home: &Path, origin: &str, boot: usize) -> Self {
        let log = fs::File::create(home.join(format!("boot-{boot}.log"))).unwrap();
        Self(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "daemon::telegram_tests::child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("AGEND_TELEGRAM_TEST_HOME", home)
                .env("AGEND_TELEGRAM_TEST_ORIGIN", origin)
                .env_remove("AGEND_INSTANCE")
                .stdout(Stdio::from(log.try_clone().unwrap()))
                .stderr(Stdio::from(log))
                .spawn()
                .unwrap(),
        )
    }
    fn interrupt(&mut self) {
        assert!(self.0.try_wait().unwrap().is_none());
        // Only signal the still-owned, unreaped child.
        assert_eq!(unsafe { libc::kill(self.0.id() as i32, libc::SIGINT) }, 0);
    }
    fn finish(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success(), "daemon child: {status}");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "daemon did not finish bounded shutdown"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

struct Native {
    origin: String,
    sent: Arc<Mutex<Vec<Value>>>,
    release: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Native {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let release = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (record, barrier, ending) = (sent.clone(), release.clone(), stop.clone());
        let worker = thread::spawn(move || {
            let mut clients = Vec::new();
            while !ending.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (record, barrier, ending) =
                            (record.clone(), barrier.clone(), ending.clone());
                        clients.push(thread::spawn(move || {
                            respond(stream, record, barrier, ending)
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        Self {
            origin,
            sent,
            release,
            stop,
            worker: Some(worker),
        }
    }
    fn wait_sent(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.sent.lock().unwrap().len() < count {
            assert!(Instant::now() < deadline, "missing Telegram send {count}");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.release.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn respond(
    stream: TcpStream,
    sent: Arc<Mutex<Vec<Value>>>,
    release: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let method = line
        .split_whitespace()
        .nth(1)
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_owned();
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 {
            return;
        }
        if header == "\r\n" {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse::<usize>().unwrap();
        }
    }
    assert!(length <= 1024 * 1024);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let request: Value = serde_json::from_slice(&body).unwrap();
    let result = match method.as_str() {
        "getMe" => {
            serde_json::from_str::<Value>(include_str!("../../tests/fixtures/telegram/get-me.json"))
                .unwrap()["result"]
                .clone()
        }
        "getUpdates" => json!([]),
        "sendMessage" => {
            let index = {
                let mut rows = sent.lock().unwrap();
                rows.push(request.clone());
                rows.len()
            };
            if index == 1 {
                let deadline = Instant::now() + Duration::from_secs(15);
                while !release.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
                    assert!(Instant::now() < deadline, "receipt barrier expired");
                    thread::sleep(Duration::from_millis(5));
                }
            }
            let mut receipt = serde_json::from_str::<Value>(include_str!(
                "../../tests/fixtures/telegram/message.json"
            ))
            .unwrap()["result"]
                .clone();
            receipt["message_id"] = json!(499 + index);
            receipt["text"] = request["text"].clone();
            receipt
        }
        _ => panic!("unexpected method {method}"),
    };
    let body = json!({"ok":true,"result":result}).to_string();
    let _ = write!(
        reader.get_mut(),
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

#[test]
fn active_shutdown_saves_the_receipt_and_restart_only_sends_the_remaining_part() {
    let dir = TempDir::new("g12d-stop").unwrap();
    let home = dir.path();
    let native = Native::start();
    let row = TelegramDelivery::new(
        "shutdown-parts".into(),
        TelegramDestination {
            bot_id: 123456789,
            chat_id: 42,
            topic_id: None,
        },
        Notification {
            severity: NotificationSeverity::Info,
            title: "Shutdown".into(),
            body: "繁中é\n".repeat(1200),
            task_id: None,
        },
        1,
    );
    assert_eq!(row.parts.len(), 2);
    let store = SqliteStore::open(home, 0).unwrap();
    block_on(store.enqueue_telegram(&row)).unwrap();
    drop(store);
    let mut first = Process::start(home, &native.origin, 1);
    native.wait_sent(1);
    first.interrupt();
    let deadline = Instant::now() + Duration::from_secs(3);
    while home.join(DAEMON_SOCKET).exists() {
        assert!(Instant::now() < deadline, "daemon did not begin shutdown");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        first.0.try_wait().unwrap().is_none(),
        "daemon abandoned active receipt"
    );
    native.release.store(true, Ordering::SeqCst);
    first.finish();
    let store = SqliteStore::open(home, 0).unwrap();
    let saved = block_on(store.telegram_delivery(&row.id)).unwrap().unwrap();
    assert_eq!(saved.next_part, 1);
    assert_eq!(saved.message_ids, vec![500]);
    assert!(!saved.in_flight && !saved.outcome_unknown);
    drop(store);
    let mut second = Process::start(home, &native.origin, 2);
    native.wait_sent(2);
    second.interrupt();
    second.finish();
    let store = SqliteStore::open(home, 0).unwrap();
    let saved = block_on(store.telegram_delivery(&row.id)).unwrap().unwrap();
    assert!(saved.complete());
    assert_eq!(saved.message_ids, vec![500, 501]);
    assert!(!saved.in_flight && !saved.outcome_unknown);
    assert_eq!(saved.notification, row.notification);
    let sent = native.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    for (request, expected) in sent.iter().zip(&row.parts) {
        assert_eq!(request["text"].as_str(), Some(expected.as_str()));
    }
}
