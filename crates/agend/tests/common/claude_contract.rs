//! DRV contracts through the actual Driver in isolated daemon composition
//! processes. The test RPC transports results without implementing delivery.
use super::*;
use agend_core::traits::{AgentMessage, DeliveryReceipt, Driver, DriverEvent, DriverEventKind};
use agend_daemon::{
    driver::{claude::ClaudeDriver, codex::CodexDriver},
    fleet::Fleet,
    handlers::Context,
    runtime::HolderRuntime,
    server::{self, Server},
};
use agend_testkit::contract::driver as contract;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    os::unix::net::{UnixListener, UnixStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
const HOME_ENV: &str = "AGEND_TEST_CLAUDE_DRIVER_HOME";

#[derive(Serialize, Deserialize)]
enum Request {
    Deliver {
        to: String,
        id: String,
        from: String,
        task: Option<String>,
        body: String,
        level: MessageLevel,
    },
    Events {
        instance: String,
        after: Option<String>,
    },
    Stop,
}
#[derive(Serialize, Deserialize)]
enum EventKind {
    Busy(bool),
    Confirmed(String),
    Usage(Option<u64>),
    Completed(Option<String>),
}
#[derive(Serialize, Deserialize)]
enum WireState {
    Queued,
    Sent,
    Confirmed,
    Failed,
}
impl From<DeliveryState> for WireState {
    fn from(state: DeliveryState) -> Self {
        match state {
            DeliveryState::Queued => Self::Queued,
            DeliveryState::Sent => Self::Sent,
            DeliveryState::Confirmed => Self::Confirmed,
            DeliveryState::Failed => Self::Failed,
        }
    }
}
impl From<WireState> for DeliveryState {
    fn from(state: WireState) -> Self {
        match state {
            WireState::Queued => Self::Queued,
            WireState::Sent => Self::Sent,
            WireState::Confirmed => Self::Confirmed,
            WireState::Failed => Self::Failed,
        }
    }
}
#[derive(Serialize, Deserialize)]
enum Reply {
    Receipt {
        state: WireState,
        backend: Option<String>,
    },
    Events(Vec<(String, EventKind)>),
    Stopped,
}
fn event_wire(event: DriverEvent) -> (String, EventKind) {
    let kind = match event.kind {
        DriverEventKind::BusyChanged { busy } => EventKind::Busy(busy),
        DriverEventKind::MessageConfirmed { message_id } => EventKind::Confirmed(message_id),
        DriverEventKind::UsageLimit { reset_at_unix_ms } => EventKind::Usage(reset_at_unix_ms),
        DriverEventKind::TurnCompleted { summary } => EventKind::Completed(summary),
    };
    (event.cursor, kind)
}
fn event_native((cursor, kind): (String, EventKind)) -> DriverEvent {
    DriverEvent {
        cursor,
        kind: match kind {
            EventKind::Busy(busy) => DriverEventKind::BusyChanged { busy },
            EventKind::Confirmed(message_id) => DriverEventKind::MessageConfirmed { message_id },
            EventKind::Usage(reset_at_unix_ms) => DriverEventKind::UsageLimit { reset_at_unix_ms },
            EventKind::Completed(summary) => DriverEventKind::TurnCompleted { summary },
        },
    }
}

/// A child entry point, not a standalone verification case.
#[test]
fn boot_child() {
    let Some(home) = std::env::var_os(HOME_ENV) else {
        return;
    };
    let home = PathBuf::from(home);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _entered = rt.enter();
    let store = Arc::new(SqliteStore::open(&home, 0).unwrap());
    let runtime = HolderRuntime::new(&home, Path::new(BIN), Vec::new(), Arc::new(|_| {}));
    let instance = rt.block_on(store.instance("claude")).unwrap().unwrap();
    let pid = agend_daemon::runtime::files::running(&home, "claude")
        .unwrap()
        .unwrap();
    let launch = agend_daemon::supervisor::launch(&home, &instance, true).unwrap();
    rt.block_on(runtime.attach(&launch, pid)).unwrap();
    let fleet = Arc::new(Fleet::new(0));
    let codex = CodexDriver::new(&home, store.clone(), Arc::new(|_| {}));
    let (pipeline, worker) = rt
        .block_on(agend_daemon::pipeline::start(
            &home,
            Path::new(BIN),
            store.clone(),
            fleet.clone(),
            codex.clone(),
        ))
        .unwrap();
    let (supervisor, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let ctx = Arc::new(Context {
        fleet,
        pipeline,
        runtime: runtime.clone(),
        supervisor,
        store: store.clone(),
        codex,
        exe: BIN.into(),
        restarting: AtomicBool::new(false),
        codex_input: Default::default(),
    });
    let socket = home.join(DAEMON_SOCKET);
    let listener = server::bind(&socket).unwrap();
    let server = Server::start(listener, socket, ctx);
    let rpc_path = home.join("contract.sock");
    let listener = UnixListener::bind(&rpc_path).unwrap();
    let (stream, _) = listener.accept().unwrap();
    let mut stream = BufReader::new(stream);
    let driver = ClaudeDriver::new(store);
    loop {
        let mut line = String::new();
        if stream.read_line(&mut line).unwrap() == 0 {
            break;
        }
        let request: Request = serde_json::from_str(&line).unwrap();
        let stop = matches!(request, Request::Stop);
        let reply: Result<Reply, String> = match request {
            Request::Deliver {
                to,
                id,
                from,
                task,
                body,
                level,
            } => rt
                .block_on(driver.deliver(
                    &to,
                    &AgentMessage {
                        id,
                        from,
                        task_id: task,
                        body,
                    },
                    match level {
                        MessageLevel::Queue => BusyLevel::Queue,
                        MessageLevel::Steer => BusyLevel::Steer,
                        MessageLevel::Interrupt => BusyLevel::Interrupt,
                        MessageLevel::Unknown => return,
                    },
                ))
                .map(|r| Reply::Receipt {
                    state: r.state.into(),
                    backend: r.backend_message_id,
                })
                .map_err(|e| e.to_string()),
            Request::Events { instance, after } => rt
                .block_on(driver.events(&instance, after.as_deref()))
                .map(|events| Reply::Events(events.into_iter().map(event_wire).collect()))
                .map_err(|e| e.to_string()),
            Request::Stop => Ok(Reply::Stopped),
        };
        writeln!(
            stream.get_mut(),
            "{}",
            serde_json::to_string(&reply).unwrap()
        )
        .unwrap();
        if stop {
            break;
        }
    }
    rt.block_on(server.stop());
    worker.abort();
    rt.block_on(async {
        let _ = worker.await;
    });
    runtime.detach("claude");
    fs::remove_file(rpc_path).unwrap();
}

struct Actor {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Actor {
    fn start(home: PathBuf, idle: Arc<AtomicBool>, idle_hint_after: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut channel = Channel::new(&home);
            while !stopped.load(Ordering::SeqCst) {
                let value = match channel.output.recv_timeout(Duration::from_millis(50)) {
                    Ok(v) => v,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(e) => panic!("native actor channel closed: {e}"),
                };
                if value["method"] != "notifications/claude/channel" {
                    continue;
                }
                idle.store(false, Ordering::SeqCst);
                let receipt = Channel::receipt(&value);
                let content = value["params"]["content"].as_str().unwrap();
                assert!(content.contains("Reply with one word: ok"), "{value}");
                let mut ledger = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(home.join("received.log"))
                    .unwrap();
                writeln!(ledger, "{}", receipt.message_id).unwrap();
                assert_ne!(
                    channel.ack(std::slice::from_ref(&receipt))["result"]["isError"],
                    true
                );
                assert_eq!(
                    Fixture::native_hook(&home, "Stop", json!({"stop_hook_active":false})),
                    json!({})
                );
                // This is only the actor's readiness hint. The Proxy also
                // checks the daemon's real idle record before invoking Driver.
                let until = Instant::now() + idle_hint_after;
                while Instant::now() < until && !stopped.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                if !stopped.load(Ordering::SeqCst) {
                    idle.store(true, Ordering::SeqCst);
                }
            }
            channel.close();
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Actor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let result = thread.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}
pub struct Backend {
    // Stops holders/removes the owned home only after every boot has dropped.
    fixture: Fixture,
    actor: Mutex<Option<Actor>>,
    idle: Arc<AtomicBool>,
    pids: Mutex<BTreeSet<u32>>,
    fresh_home_on_boot: bool,
    idle_hint_after: Duration,
}
impl Backend {
    fn new() -> std::rc::Rc<Self> {
        Self::with_fresh_home(false)
    }
    fn with_fresh_home(fresh_home_on_boot: bool) -> std::rc::Rc<Self> {
        let mut fixture = Fixture::new(0);
        fixture.start();
        fixture.stop();
        std::rc::Rc::new(Self {
            fixture,
            actor: Mutex::new(None),
            idle: Arc::new(AtomicBool::new(false)),
            pids: Mutex::new(BTreeSet::new()),
            fresh_home_on_boot,
            idle_hint_after: Duration::from_millis(5300),
        })
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.actor.get_mut().unwrap().take();
    }
}

pub struct Proxy {
    rpc: Mutex<BufReader<UnixStream>>,
    idle: Arc<AtomicBool>,
}
impl Proxy {
    fn request(&self, request: Request) -> Result<Reply, String> {
        let mut stream = self.rpc.lock().unwrap();
        writeln!(
            stream.get_mut(),
            "{}",
            serde_json::to_string(&request).unwrap()
        )
        .map_err(|e| e.to_string())?;
        let mut line = String::new();
        stream.read_line(&mut line).map_err(|e| e.to_string())?;
        serde_json::from_str::<Result<Reply, String>>(&line).map_err(|e| e.to_string())?
    }
}
impl Driver for Proxy {
    type Error = String;
    async fn deliver(
        &self,
        instance_id: &str,
        message: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, String> {
        if instance_id == "claude" {
            let until = Instant::now() + Duration::from_secs(12);
            loop {
                if self.idle.load(Ordering::SeqCst) {
                    let reported_idle = self
                        .events(instance_id, None)
                        .await?
                        .iter()
                        .rev()
                        .find_map(|event| match event.kind {
                            DriverEventKind::BusyChanged { busy } => Some(!busy),
                            _ => None,
                        })
                        .unwrap_or(false);
                    if reported_idle {
                        break;
                    }
                }
                if Instant::now() >= until {
                    return Err("native daemon never reported the actor idle".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        match self.request(Request::Deliver {
            to: instance_id.into(),
            id: message.id.clone(),
            from: message.from.clone(),
            task: message.task_id.clone(),
            body: message.body.clone(),
            level: match mode {
                BusyLevel::Queue => MessageLevel::Queue,
                BusyLevel::Steer => MessageLevel::Steer,
                BusyLevel::Interrupt => MessageLevel::Interrupt,
            },
        })? {
            Reply::Receipt { state, backend } => Ok(DeliveryReceipt {
                state: state.into(),
                backend_message_id: backend,
            }),
            _ => Err("unexpected Driver reply".into()),
        }
    }
    async fn events(
        &self,
        instance_id: &str,
        after_cursor: Option<&str>,
    ) -> Result<Vec<DriverEvent>, String> {
        match self.request(Request::Events {
            instance: instance_id.into(),
            after: after_cursor.map(str::to_owned),
        })? {
            Reply::Events(events) => Ok(events.into_iter().map(event_native).collect()),
            _ => Err("unexpected Driver reply".into()),
        }
    }
}
pub struct ContractFixture {
    backend: std::rc::Rc<Backend>,
    child: Child,
    proxy: Proxy,
    _replacement: Option<std::rc::Rc<Backend>>,
}
impl ContractFixture {
    fn boot(backend: &std::rc::Rc<Backend>) -> Self {
        if backend.fresh_home_on_boot && !backend.pids.lock().unwrap().is_empty() {
            let replacement = Backend::new();
            let mut fixture = Self::boot(&replacement);
            backend.pids.lock().unwrap().insert(fixture.child.id());
            fixture.backend = backend.clone();
            fixture._replacement = Some(replacement);
            return fixture;
        }
        let home = &backend.fixture.home;
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(home.join("contract-process.log"))
            .unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "claude_contract::boot_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(HOME_ENV, home)
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        backend.pids.lock().unwrap().insert(child.id());
        let until = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            if let Ok(stream) = UnixStream::connect(home.join("contract.sock")) {
                break stream;
            }
            if child.try_wait().unwrap().is_some() || Instant::now() >= until {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "contract child failed: {}",
                    fs::read_to_string(home.join("contract-process.log")).unwrap()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let fixture = Self {
            backend: backend.clone(),
            child,
            proxy: Proxy {
                rpc: Mutex::new(BufReader::new(stream)),
                idle: backend.idle.clone(),
            },
            _replacement: None,
        };
        wait_empty(home, "hooks");
        wait_empty(home, "acks");
        Fixture::native_hook(home, "SessionStart", json!({"source":"resume"}));
        std::thread::sleep(Duration::from_millis(5300));
        if backend.actor.lock().unwrap().is_none() {
            *backend.actor.lock().unwrap() = Some(Actor::start(
                home.clone(),
                backend.idle.clone(),
                backend.idle_hint_after,
            ));
        }
        // Fresh native Poll establishes actual idle before the contract starts.
        let request = ClientRequest::Claude {
            data: ClaudeRequestData {
                request_id: id(),
                instance_id: "claude".into(),
                operation: ClaudeOperation::Poll {
                    session_id: SESSION.into(),
                },
            },
        };
        let response = agend_client::exchange_once(
            &home.join(DAEMON_SOCKET),
            Some("claude".into()),
            V1_5,
            &request,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        assert!(matches!(response, ClientResponse::Claude { data } if data.messages.is_empty()));
        backend.idle.store(true, Ordering::SeqCst);
        fixture
    }
}

#[test]
fn changing_home_between_drv_boots_fails_at_the_second_boot() {
    let backend = Backend::with_fresh_home(true);
    let case = contract::cases::<ContractFixture>()
        .into_iter()
        .find(|case| case.name == "every_boot_backfills_what_happened_while_down")
        .unwrap();
    let error = (case.check)(ContractFixture::boot(&backend))
        .expect_err("a replacement HOME must lose backfill");
    assert!(error.contains("boot 2"), "{error}");
    println!("negative new HOME check: {error}");
}

#[test]
fn native_idle_record_not_actor_timer_defines_the_contract_precondition() {
    let mut backend = Backend::new();
    // A deliberately early actor hint reproduces the scheduling gap: the
    // production five-second idle gate has not yet reported idle to Driver.
    std::rc::Rc::get_mut(&mut backend).unwrap().idle_hint_after = Duration::from_millis(100);
    let case = contract::cases::<ContractFixture>()
        .into_iter()
        .find(|case| case.name == "cursors_are_unique")
        .unwrap();
    let result = (case.check)(ContractFixture::boot(&backend));
    assert!(
        result.is_ok(),
        "early actor hint must not start an idle contract while busy: {result:?}"
    );
    assert_eq!(backend.pids.lock().unwrap().len(), 1);
    assert_eq!(
        fs::read_to_string(backend.fixture.home.join("received.log"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}
impl contract::DriverFixture for ContractFixture {
    type Driver = Proxy;
    type Error = String;
    type Persisted = std::rc::Rc<Backend>;
    fn driver(&self) -> &Proxy {
        &self.proxy
    }
    fn instance_id(&self) -> &str {
        "claude"
    }
    fn persisted(&self) -> std::rc::Rc<Backend> {
        self.backend.clone()
    }
    fn boot(persisted: &std::rc::Rc<Backend>) -> Self {
        Self::boot(persisted)
    }
    fn turn_timeout(&self) -> Duration {
        Duration::from_secs(2)
    }
    fn emit_while_down(persisted: &std::rc::Rc<Backend>) {
        assert!(!persisted.fixture.home.join(DAEMON_SOCKET).exists());
        assert_eq!(
            Fixture::native_hook(
                &persisted.fixture.home,
                "Stop",
                json!({"stop_hook_active":false})
            ),
            json!({})
        );
        assert!(pending(&persisted.fixture.home, "hooks") > 0);
    }
}
impl Drop for ContractFixture {
    fn drop(&mut self) {
        let _ = self.proxy.request(Request::Stop);
        let until = Instant::now() + Duration::from_secs(10);
        while self.child.try_wait().unwrap().is_none() {
            if Instant::now() >= until {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn actual_claude_driver_passes_all_drv_cases_with_distinct_process_boots() {
    for case in contract::cases::<ContractFixture>() {
        let backend = Backend::new();
        let result = (case.check)(ContractFixture::boot(&backend));
        let pids = backend.pids.lock().unwrap().clone();
        assert!(result.is_ok(), "{} {}: {result:?}", case.rule, case.name);
        let expected = if matches!(
            case.name,
            "every_boot_backfills_what_happened_while_down"
                | "same_message_id_makes_one_turn_across_restarts"
        ) {
            // The shared contract discards its seed before the four boots.
            1 + agend_testkit::contract::LIFECYCLE.len()
        } else {
            1
        };
        assert_eq!(
            pids.len(),
            expected,
            "{} process identities: {pids:?}",
            case.name
        );
        println!("{} {} passed: {pids:?}", case.rule, case.name);
        let received =
            fs::read_to_string(backend.fixture.home.join("received.log")).unwrap_or_default();
        let ids: Vec<_> = received.lines().collect();
        assert_eq!(
            ids.iter().collect::<BTreeSet<_>>().len(),
            ids.len(),
            "replayed content: {received}"
        );
    }
}
