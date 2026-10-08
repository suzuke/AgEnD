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
            {
                let exe = std::env::var_os("AGEND_TELEGRAM_TEST_EXE")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| std::env::current_exe().unwrap());
                let binding = crate::backend_versions::ExecutableBinding::capture_running(&exe);
                (exe, binding)
            },
            store,
            agend_core::policy::codex_input::CodexInputPolicy::approved(),
            Some((config, token)),
            Some(api),
            TestControl {
                hold_supervisor_for_stop: std::env::var("AGEND_TELEGRAM_TEST_QUEUE_STOP")
                    .as_deref()
                    == Ok("1"),
                registry_origin: None,
            },
        ))
        .unwrap();
    assert!(matches!(stopped, Stopped::Signal("SIGINT")));
    runtime.shutdown_timeout(Duration::from_secs(1));
}

struct Process(Child);
impl Process {
    fn start(home: &Path, origin: &str, boot: usize) -> Self {
        Self::start_using(home, origin, boot, None)
    }
    fn start_using(home: &Path, origin: &str, boot: usize, exe: Option<&Path>) -> Self {
        Self::start_controlled(home, origin, boot, exe, false)
    }
    fn start_controlled(
        home: &Path,
        origin: &str,
        boot: usize,
        exe: Option<&Path>,
        hold: bool,
    ) -> Self {
        let log = fs::File::create(home.join(format!("boot-{boot}.log"))).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.env_remove("AGEND_TELEGRAM_TEST_EXE");
        command.env(
            "AGEND_TELEGRAM_TEST_QUEUE_STOP",
            if hold { "1" } else { "0" },
        );
        if let Some(exe) = exe {
            command.env("AGEND_TELEGRAM_TEST_EXE", exe);
        }
        Self(
            command
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

#[derive(Default)]
struct Mobile {
    updates: Vec<Value>,
    answers: Vec<Value>,
    receipts: Vec<Value>,
}
struct Native {
    origin: String,
    sent: Arc<Mutex<Vec<Value>>>,
    mobile: Arc<Mutex<Mobile>>,
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
        let mobile = Arc::new(Mutex::new(Mobile::default()));
        let inbox = mobile.clone();
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
                        let inbox = inbox.clone();
                        clients.push(thread::spawn(move || {
                            respond(stream, record, barrier, ending, inbox)
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
            mobile,
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
    mobile: Arc<Mutex<Mobile>>,
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
        "getUpdates" => Value::Array(
            mobile
                .lock()
                .unwrap()
                .updates
                .iter()
                .filter(|u| u["update_id"].as_i64().unwrap() >= request["offset"].as_i64().unwrap())
                .cloned()
                .collect(),
        ),
        "answerCallbackQuery" => {
            mobile.lock().unwrap().answers.push(request.clone());
            json!(true)
        }
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
            receipt["reply_markup"] = request["reply_markup"].clone();
            mobile.lock().unwrap().receipts.push(receipt.clone());
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

static NEXT_HOME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct ShortHome(PathBuf);
impl ShortHome {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let serial = NEXT_HOME.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!(
            "/tmp/g12d-retry-{}-{nonce}-{serial}",
            std::process::id()
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for ShortHome {
    fn drop(&mut self) {
        if matches!(crate::runtime::files::running_holders(&self.0), Ok(holders) if holders.is_empty())
        {
            let _ = fs::remove_dir_all(&self.0);
        } else {
            eprintln!(
                "retaining owned home {}: holder cleanup unconfirmed",
                self.0.display()
            );
        }
    }
}
struct OwnedHolder(PathBuf);
impl OwnedHolder {
    fn stop(&self) -> Result<(), String> {
        if crate::runtime::files::running(&self.0, "retry-agent")
            .map_err(|e| e.to_string())?
            .is_some()
        {
            crate::runtime::shutdown_holder_within(&self.0, "retry-agent", Duration::from_secs(3))?;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while crate::runtime::files::running(&self.0, "retry-agent")
            .map_err(|e| e.to_string())?
            .is_some()
        {
            if Instant::now() >= deadline {
                return Err("owned holder did not exit".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}
impl Drop for OwnedHolder {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("owned holder cleanup: {error}");
        }
    }
}

#[test]
fn mobile_retry_reaches_the_real_supervisor_once_and_survives_restart() {
    run_mobile_retry(false);
}
#[test]
fn queued_mobile_retry_is_cancelled_by_daemon_shutdown_without_false_acceptance() {
    run_mobile_retry(true);
}
fn run_mobile_retry(stop_before_retry: bool) {
    use agend_core::{
        model::Backend,
        runtime_records::{Instance, InstanceStatus},
    };
    let exe = agend_testkit::fake_agent::locate("agend").unwrap();
    let dir = ShortHome::new();
    let home = dir.0.as_path();
    let holder = OwnedHolder(home.to_path_buf());
    let store = SqliteStore::open(home, 0).unwrap();
    let instance = Instance {
        id: "retry-agent".into(),
        backend: Backend::Claude,
        program: "/bin/bash".into(),
        args: vec![
            "-c".into(),
            "echo launched >> launches; exec sleep 600".into(),
            "fixture".into(),
        ],
        working_directory: home.display().to_string(),
        session_id: Some("test-session".into()),
        status: InstanceStatus::Failed,
        session_started: false,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "inbox".into(),
    };
    block_on(store.add_instance(&instance)).unwrap();
    drop(store);
    let native = Native::start();
    native.release.store(true, Ordering::SeqCst);
    let mut first =
        Process::start_controlled(home, &native.origin, 1, Some(&exe), stop_before_retry);
    let deadline = Instant::now() + Duration::from_secs(15);
    let receipt = loop {
        let receipt = native.mobile.lock().unwrap().receipts.first().cloned();
        if let Some(receipt) = receipt {
            break receipt;
        }
        assert!(Instant::now() < deadline, "failure notification missing");
        thread::sleep(Duration::from_millis(10));
    };
    let button = receipt["reply_markup"]["inline_keyboard"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row.as_array().unwrap())
        .find(|button| button["text"] == "retry")
        .unwrap()
        .clone();
    if stop_before_retry {
        first.interrupt();
        while !home.join("stop-queued").exists() {
            assert!(Instant::now() < deadline, "Stop was not queued");
            thread::sleep(Duration::from_millis(10));
        }
    }
    native
        .mobile
        .lock()
        .unwrap()
        .updates
        .push(json!({"update_id":1,"callback_query":{
            "id":"real-retry", "from":{"id":7,"is_bot":false}, "message":receipt,
            "data":button["callback_data"]
        }}));
    if stop_before_retry {
        let wait = Instant::now() + Duration::from_secs(5);
        while !home.join("retry-queued").exists() {
            assert!(
                Instant::now() < wait,
                "retry not queued; answers={:?}; log={}",
                native.mobile.lock().unwrap().answers,
                fs::read_to_string(home.join("boot-1.log")).unwrap_or_default()
            );
            thread::sleep(Duration::from_millis(10));
        }
        first.finish();
        let store = SqliteStore::open(home, 0).unwrap();
        let saved = block_on(store.instance("retry-agent")).unwrap().unwrap();
        assert_eq!(saved.status, InstanceStatus::Failed);
        assert!(!saved.session_started);
        drop(store);
        assert!(!home.join("launches").exists());
        assert!(
            !native
                .mobile
                .lock()
                .unwrap()
                .answers
                .iter()
                .any(|v| v["text"] == "Accepted")
        );
        let conn = rusqlite::Connection::open(home.join("agend.db")).unwrap();
        let outcome: String = conn
            .query_row(
                "SELECT outcome FROM telegram_updates WHERE update_id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(outcome, "refused");
        drop(conn);
        holder.stop().unwrap();
        return;
    }
    loop {
        let accepted = native
            .mobile
            .lock()
            .unwrap()
            .answers
            .iter()
            .any(|v| v["text"] == "Accepted");
        if accepted && home.join("launches").exists() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "retry did not launch the owned instance: {}",
            fs::read_to_string(home.join("boot-1.log")).unwrap_or_default()
        );
        thread::sleep(Duration::from_millis(10));
    }
    first.interrupt();
    first.finish();
    let store = SqliteStore::open(home, 0).unwrap();
    let saved = block_on(store.instance("retry-agent")).unwrap().unwrap();
    assert_eq!(saved.status, InstanceStatus::Running);
    assert!(saved.session_started);
    drop(store);
    let mut second = Process::start_using(home, &native.origin, 2, Some(&exe));
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        assert!(second.0.try_wait().unwrap().is_none());
        assert_eq!(
            fs::read_to_string(home.join("launches")).unwrap(),
            "launched\n"
        );
        thread::sleep(Duration::from_millis(20));
    }
    second.interrupt();
    second.finish();
    assert_eq!(
        native
            .mobile
            .lock()
            .unwrap()
            .answers
            .iter()
            .filter(|v| v["text"] == "Accepted")
            .count(),
        1
    );
    let conn = rusqlite::Connection::open(home.join("agend.db")).unwrap();
    let outcomes: Vec<String> = conn
        .prepare("SELECT outcome FROM telegram_updates ORDER BY update_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(outcomes, vec!["accepted"]);
    drop(conn);
    holder.stop().unwrap();
    assert!(
        crate::runtime::files::running_holders(home)
            .unwrap()
            .is_empty()
    );
}
