//! Real HTTP transport, SQLite, Fleet and serialized operator path; no live bot.
use super::{config::Token, http::Api, inbound, poll, worker};
use crate::{fleet::Fleet, handlers::Context, store::SqliteStore};
use agend_core::{
    config::{SecretRef, TelegramConfig},
    pipeline::{
        state::PipelineState,
        task::{Task, TaskStatus},
        workflow::Workflow,
    },
    telegram::{TelegramDestination, TelegramInboundStore, TelegramStore},
};
use agend_testkit::tempdir::TempDir;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

struct Native {
    api: Arc<Api>,
    updates: Arc<Mutex<Vec<Value>>>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Native {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let updates = Arc::new(Mutex::new(Vec::<Value>::new()));
        let queue = updates.clone();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let observed = calls.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let ending = stop.clone();
        let thread = thread::spawn(move || {
            while !ending.load(Ordering::SeqCst) {
                let (stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
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
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse::<usize>().unwrap();
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                observed
                    .lock()
                    .unwrap()
                    .push((method.clone(), request.clone()));
                let result = match method.as_str() {
                    "getUpdates" => Value::Array(
                        queue
                            .lock()
                            .unwrap()
                            .iter()
                            .filter(|u| {
                                u["update_id"].as_i64().unwrap()
                                    >= request["offset"].as_i64().unwrap()
                            })
                            .cloned()
                            .collect(),
                    ),
                    "answerCallbackQuery" => json!(true),
                    "getMe" => serde_json::from_str::<Value>(include_str!(
                        "../../tests/fixtures/telegram/get-me.json"
                    ))
                    .unwrap()["result"]
                        .clone(),
                    "sendMessage" => {
                        let mut receipt = serde_json::from_str::<Value>(include_str!(
                            "../../tests/fixtures/telegram/message.json"
                        ))
                        .unwrap()["result"]
                            .clone();
                        receipt["text"] = request["text"].clone();
                        receipt["reply_markup"] = request["reply_markup"].clone();
                        receipt
                    }
                    _ => panic!("unexpected method {method}"),
                };
                let body = json!({"ok":true,"result":result}).to_string();
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        Self {
            api: Arc::new(Api::with_origin(
                Token::parse("123:abcdefghijklmnopqrstuvwxyz_123456789".into()).unwrap(),
                origin,
            )),
            updates,
            calls,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
struct Lab {
    ctx: Arc<Context>,
    pipeline: tokio::task::JoinHandle<()>,
    _dir: TempDir,
    config: TelegramConfig,
    destination: TelegramDestination,
}
impl Drop for Lab {
    fn drop(&mut self) {
        self.pipeline.abort();
    }
}
impl Lab {
    async fn new() -> Self {
        let dir = TempDir::new("telegram-native-poll").unwrap();
        let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
        let fleet = Arc::new(Fleet::new(0));
        let codex =
            crate::driver::codex::CodexDriver::new(dir.path(), store.clone(), Arc::new(|_| {}));
        let (pipeline, task) = crate::pipeline::start(
            dir.path(),
            Path::new("/nonexistent/agend"),
            store.clone(),
            fleet.clone(),
            codex.clone(),
        )
        .await
        .unwrap();
        let (supervisor, _) = tokio::sync::mpsc::unbounded_channel();
        let ctx = Arc::new(Context {
            pipeline,
            fleet,
            runtime: crate::runtime::HolderRuntime::new(
                dir.path(),
                Path::new("/nonexistent/agend"),
                vec![],
                Arc::new(|_| {}),
            ),
            supervisor,
            store,
            codex,
            exe: "/nonexistent/agend".into(),
            restarting: AtomicBool::new(false),
            codex_input: Default::default(),
        });
        Self {
            ctx,
            pipeline: task,
            _dir: dir,
            config: TelegramConfig {
                token: SecretRef::Env("UNUSED".into()),
                chat_id: 42,
                allow_user_ids: vec![7],
                needs_you_topic: None,
                team_topics: Default::default(),
            },
            destination: TelegramDestination {
                bot_id: 123456789,
                chat_id: 42,
                topic_id: None,
            },
        }
    }
    async fn failed_task(&self) -> agend_core::protocol::client::AttentionRequiredData {
        let workflow = Workflow::builtin_research();
        let mut task = Task::new("t-1", "native mobile", "general", "research", 1);
        task.status = TaskStatus::Failed;
        let state = PipelineState::new("t-1", crate::pipeline::validate(workflow).unwrap());
        self.ctx
            .store
            .create_pipeline_task(&task, &serde_json::to_string(&state.snapshot()).unwrap(), 1)
            .await
            .unwrap();
        self.ctx.pipeline.wake();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(item) = self.ctx.fleet.attention("task-failed:t-1") {
                    break item;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
}
#[tokio::test]
async fn native_poll_authorizes_once_and_applies_acknowledgment_inside_pipeline() {
    use agend_core::traits::Store;
    let lab = Lab::new().await;
    let native = Native::new();
    let item = lab.failed_task().await;
    let mut notice = worker::notice(&item).unwrap();
    notice.task_version = Some((
        lab.ctx
            .store
            .load_task("t-1")
            .await
            .unwrap()
            .unwrap()
            .version,
        lab.ctx
            .store
            .progress("t-1")
            .await
            .unwrap()
            .unwrap()
            .attention_revision,
    ));
    let row = lab
        .ctx
        .store
        .observe_telegram(&[notice], &lab.destination, 1)
        .await
        .unwrap()
        .remove(0);
    let notifier = super::delivery::TelegramNotifier::new(
        native.api.clone(),
        lab.ctx.store.clone(),
        lab.destination.clone(),
    );
    notifier.resume(&row.id).await.unwrap();
    let markup = inbound::keyboard(&row).unwrap();
    let receipt =
        serde_json::from_str::<Value>(include_str!("../../tests/fixtures/telegram/message.json"))
            .unwrap()["result"]
            .clone();
    let mut update = json!({"update_id":1,"callback_query":{"id":"native-callback","from":{"id":8,"is_bot":false},"message":receipt,"data":markup["inline_keyboard"][0][0]["callback_data"]}});
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    native.updates.lock().unwrap().push(update.clone());
    poll::once(
        &lab.ctx,
        &lab.config,
        native.api.clone(),
        &lab.destination,
        &stopped,
    )
    .await
    .unwrap();
    assert!(
        !lab.ctx
            .store
            .progress("t-1")
            .await
            .unwrap()
            .unwrap()
            .acknowledged
    );
    update["update_id"] = json!(2);
    update["callback_query"]["from"]["id"] = json!(7);
    native.updates.lock().unwrap().push(update.clone());
    poll::once(
        &lab.ctx,
        &lab.config,
        native.api.clone(),
        &lab.destination,
        &stopped,
    )
    .await
    .unwrap();
    let after = lab.ctx.store.progress("t-1").await.unwrap().unwrap();
    assert!(after.acknowledged);
    update["update_id"] = json!(3);
    native.updates.lock().unwrap().push(update);
    poll::once(
        &lab.ctx,
        &lab.config,
        native.api.clone(),
        &lab.destination,
        &stopped,
    )
    .await
    .unwrap();
    assert_eq!(
        lab.ctx
            .store
            .progress("t-1")
            .await
            .unwrap()
            .unwrap()
            .attention_revision,
        after.attention_revision
    );
    assert_eq!(lab.ctx.store.telegram_offset(123456789).await.unwrap(), 4);
    assert_eq!(
        native
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "answerCallbackQuery")
            .count(),
        2,
        "allowed stale buttons get feedback; unauthorized users do not"
    );
    assert_eq!(
        native
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, v)| m == "answerCallbackQuery" && v["text"] == "Accepted")
            .count(),
        1
    );
}

#[tokio::test]
async fn shutdown_cancels_a_queued_retry_without_removing_the_item_or_reporting_acceptance() {
    use agend_core::{
        model::Backend,
        runtime_records::{Instance, InstanceStatus},
    };
    let mut lab = Lab::new().await;
    let (sender, mut queue) = tokio::sync::mpsc::unbounded_channel();
    Arc::get_mut(&mut lab.ctx).unwrap().supervisor = sender;
    let instance = Instance {
        id: "failed-agent".into(),
        backend: Backend::Claude,
        program: "/nonexistent/claude".into(),
        args: vec![],
        working_directory: lab._dir.path().display().to_string(),
        session_id: Some("session".into()),
        status: InstanceStatus::Failed,
        session_started: true,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "push".into(),
    };
    let item = crate::supervisor::failed_item(&instance, "fixture failure", 1);
    lab.ctx.fleet.raise(item.clone());
    let native = Native::new();
    let row = lab
        .ctx
        .store
        .observe_telegram(&[worker::notice(&item).unwrap()], &lab.destination, 1)
        .await
        .unwrap()
        .remove(0);
    super::delivery::TelegramNotifier::new(
        native.api.clone(),
        lab.ctx.store.clone(),
        lab.destination.clone(),
    )
    .resume(&row.id)
    .await
    .unwrap();
    let markup = inbound::keyboard(&row).unwrap();
    let receipt =
        serde_json::from_str::<Value>(include_str!("../../tests/fixtures/telegram/message.json"))
            .unwrap()["result"]
            .clone();
    native.updates.lock().unwrap().push(json!({"update_id":1,"callback_query":{"id":"shutdown","from":{"id":7,"is_bot":false},"message":receipt,"data":markup["inline_keyboard"][0][0]["callback_data"]}}));
    let ctx = lab.ctx.clone();
    let config = lab.config.clone();
    let destination = lab.destination.clone();
    let api = native.api.clone();
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    let polling =
        tokio::spawn(async move { poll::once(&ctx, &config, api, &destination, &stopped).await });
    let event = tokio::time::timeout(Duration::from_secs(2), queue.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        crate::supervisor::Event::RetryConfirmed { .. }
    ));
    assert_eq!(
        lab.ctx.fleet.attention(item.attention_id.as_ref().unwrap()),
        Some(item.clone())
    );
    // Production shutdown drops the receiver before waiting for Telegram.
    drop(queue);
    drop(event);
    tokio::time::timeout(Duration::from_secs(2), polling)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        lab.ctx.fleet.attention(item.attention_id.as_ref().unwrap()),
        Some(item)
    );
    assert!(
        !native
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(m, v)| m == "answerCallbackQuery" && v["text"] == "Accepted")
    );
    assert_eq!(lab.ctx.store.telegram_offset(123456789).await.unwrap(), 2);
}
