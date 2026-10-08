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
    bot: Arc<Mutex<Value>>,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    receipts: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Native {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let bot = Arc::new(Mutex::new(
            serde_json::from_str::<Value>(include_str!(
                "../../tests/fixtures/telegram/get-me.json"
            ))
            .unwrap()["result"]
                .clone(),
        ));
        let identity = bot.clone();
        let updates = Arc::new(Mutex::new(Vec::<Value>::new()));
        let queue = updates.clone();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let observed = calls.clone();
        let receipts = Arc::new(Mutex::new(Vec::<Value>::new()));
        let produced = receipts.clone();
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
                    "getMe" => identity.lock().unwrap().clone(),
                    "sendMessage" => {
                        let mut receipt = serde_json::from_str::<Value>(include_str!(
                            "../../tests/fixtures/telegram/message.json"
                        ))
                        .unwrap()["result"]
                            .clone();
                        let mut produced = produced.lock().unwrap();
                        receipt["message_id"] =
                            json!(receipt["message_id"].as_i64().unwrap() + produced.len() as i64);
                        receipt["text"] = request["text"].clone();
                        receipt["reply_markup"] = request["reply_markup"].clone();
                        produced.push(receipt.clone());
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
            bot,
            calls,
            receipts,
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

#[tokio::test]
async fn mobile_read_is_shared_without_consuming_the_operator_action() {
    use agend_core::attention_read::AttentionReadStore;
    let lab = Lab::new().await;
    let native = Native::new();
    let item = lab.failed_task().await;
    use agend_core::traits::Store;
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
    super::delivery::TelegramNotifier::new(
        native.api.clone(),
        lab.ctx.store.clone(),
        lab.destination.clone(),
    )
    .resume(&row.id)
    .await
    .unwrap();
    let markup = inbound::keyboard(&row).unwrap();
    let buttons = markup["inline_keyboard"].as_array().unwrap();
    let receipt =
        serde_json::from_str::<Value>(include_str!("../../tests/fixtures/telegram/message.json"))
            .unwrap()["result"]
            .clone();
    let mut update = json!({"update_id":1,"callback_query":{"id":"read","from":{"id":7,"is_bot":false},"message":receipt,"data":buttons.last().unwrap()[0]["callback_data"]}});
    let before = lab.ctx.fleet.view().as_of_event_id;
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    for id in [1, 2] {
        update["update_id"] = json!(id);
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
        assert_eq!(
            lab.ctx.fleet.attention(item.attention_id.as_ref().unwrap()),
            Some(item.clone())
        );
        assert!(
            !lab.ctx
                .store
                .progress("t-1")
                .await
                .unwrap()
                .unwrap()
                .acknowledged
        );
    }
    let key = item.read_key().unwrap();
    assert_eq!(lab.ctx.fleet.view().read_keys, vec![key.clone()]);
    assert_eq!(lab.ctx.fleet.subscribe(Some(before)).unwrap().backlog.iter().filter(|event| matches!(&event.event, agend_core::protocol::client::DaemonEvent::AttentionRead { data } if data.read_key == key)).count(), 1);
    assert_eq!(
        lab.ctx.store.attention_read_keys().await.unwrap(),
        vec![key]
    );
    update["update_id"] = json!(3);
    update["callback_query"]["data"] = markup["inline_keyboard"][0][0]["callback_data"].clone();
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
    assert!(
        lab.ctx
            .store
            .progress("t-1")
            .await
            .unwrap()
            .unwrap()
            .acknowledged
    );
    assert_eq!(
        native
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, v)| m == "answerCallbackQuery"
                && v["text"] == "Marked read; item remains open")
            .count(),
        2
    );
}

#[tokio::test]
async fn a_followup_remains_unread_and_an_old_read_key_cannot_mark_it() {
    use agend_core::protocol::ask::{AskEntry, AskThread};
    let lab = Lab::new().await;
    let mut item = lab.failed_task().await;
    item.attention_id = Some("ask-1".into());
    item.ask = Some(AskThread {
        ask_id: "ask-1".into(),
        task_id: None,
        entries: vec![AskEntry::Question {
            from: "agent".into(),
            text: "First?".into(),
            options: vec![],
        }],
    });
    lab.ctx.fleet.upsert_attention(item.clone());
    let key = item.read_key().unwrap();
    crate::handlers::mark_attention_read(&lab.ctx, "ask-1", &key)
        .await
        .unwrap();
    assert!(lab.ctx.fleet.view().read_keys.contains(&key));
    item.ask.as_mut().unwrap().entries.push(AskEntry::FollowUp {
        from: "agent".into(),
        text: "Next?".into(),
        options: vec![],
    });
    lab.ctx.fleet.upsert_attention(item.clone());
    assert!(
        !lab.ctx
            .fleet
            .view()
            .read_keys
            .contains(&item.read_key().unwrap())
    );
    assert!(
        crate::handlers::mark_attention_read(&lab.ctx, "ask-1", &key)
            .await
            .is_err()
    );
    assert_eq!(lab.ctx.fleet.attention("ask-1"), Some(item.clone()));
    crate::handlers::mark_attention_read(&lab.ctx, "ask-1", &item.read_key().unwrap())
        .await
        .unwrap();
    assert!(
        lab.ctx
            .fleet
            .view()
            .read_keys
            .contains(&item.read_key().unwrap())
    );
}

#[tokio::test]
async fn native_mobile_choice_and_free_reply_reach_the_asking_agent_once() {
    use agend_core::{
        model::Backend,
        protocol::{
            ask::{AnswerSource, AskEntry, AskReply},
            client::{AgentCommand, CommandResult},
        },
        runtime_records::{Instance, InstanceStatus},
    };
    let lab = Lab::new().await;
    let native = Native::new();
    lab.ctx
        .store
        .add_instance(&Instance {
            id: "asker".into(),
            backend: Backend::Codex,
            program: "unused".into(),
            args: vec![],
            working_directory: lab._dir.path().to_string_lossy().into_owned(),
            session_id: None,
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "inbox".into(),
        })
        .await
        .unwrap();
    let CommandResult::AskCreated { data } = lab
        .ctx
        .pipeline
        .agent(
            Some("asker".into()),
            AgentCommand::Ask {
                question: "Which approach?".into(),
                options: vec!["First".into(), "Second".into()],
            },
        )
        .await
        .unwrap()
    else {
        panic!("ask not created")
    };
    let ask_id = data.ask_id;
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    let mut previous = None;
    for round in 0..2 {
        let item = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(item) = lab.ctx.fleet.attention(&ask_id) {
                    let entries = &item.ask.as_ref().unwrap().entries;
                    if (round == 0 && entries.len() == 1) || (round == 1 && entries.len() == 3) {
                        break item;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let row = lab
            .ctx
            .store
            .observe_telegram(
                &[worker::notice(&item).unwrap()],
                &lab.destination,
                1 + round,
            )
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
        let receipt = native.receipts.lock().unwrap().last().unwrap().clone();
        let update = if round == 0 {
            let markup = receipt["reply_markup"].clone();
            json!({"update_id":1,"callback_query":{"id":"choice","from":{"id":7,"is_bot":false},"message":receipt,"data":markup["inline_keyboard"][1][0]["callback_data"]}})
        } else {
            let mut old: Value = previous.clone().unwrap();
            old["update_id"] = json!(2);
            native.updates.lock().unwrap().push(old);
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
                lab.ctx.store.asks().await.unwrap()[0].thread.entries.len(),
                3,
                "old choice cannot answer follow-up"
            );
            let stale_reply = json!({"update_id":3,"message":{"from":{"id":7,"is_bot":false},"chat":{"id":42},"reply_to_message":previous.as_ref().unwrap()["callback_query"]["message"],"text":"stale free text"}});
            native.updates.lock().unwrap().push(stale_reply);
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
                lab.ctx.store.asks().await.unwrap()[0].thread.entries.len(),
                3,
                "old free reply cannot answer follow-up"
            );
            json!({"update_id":4,"message":{"from":{"id":7,"is_bot":false},"chat":{"id":42},"reply_to_message":receipt,"text":"完整回答\nwith details"}})
        };
        previous = Some(update.clone());
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
        // Re-poll the same producer batch; the persistent offset must skip it.
        poll::once(
            &lab.ctx,
            &lab.config,
            native.api.clone(),
            &lab.destination,
            &stopped,
        )
        .await
        .unwrap();
        let asks = lab.ctx.store.asks().await.unwrap();
        let expected = if round == 0 {
            AskReply::Choice {
                option: "Second".into(),
            }
        } else {
            AskReply::Text {
                text: "完整回答\nwith details".into(),
            }
        };
        assert!(
            matches!(asks[0].thread.entries.last(), Some(AskEntry::Answer { source: AnswerSource::Telegram, reply, .. }) if *reply == expected)
        );
        let messages = lab.ctx.store.messages_to("asker").await.unwrap();
        assert_eq!(messages.len(), (round + 1) as usize);
        for (index, message) in messages.iter().enumerate() {
            assert_eq!(message.id, format!("ask:{ask_id}/{}", 2 * (index + 1)));
            assert_eq!(message.from_instance, "daemon");
            assert_eq!(message.to_instance, "asker");
            assert_eq!(
                serde_json::from_str::<AskEntry>(&message.body).unwrap(),
                asks[0].thread.entries[2 * index + 1]
            );
        }
        if round == 0 {
            lab.ctx
                .pipeline
                .agent(
                    Some("asker".into()),
                    AgentCommand::AskFollowUp {
                        ask_id: ask_id.clone(),
                        question: "Explain?".into(),
                        options: vec![],
                    },
                )
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn native_mobile_approval_and_changes_require_current_receipt_and_explicit_reason() {
    use agend_core::{
        pipeline::{
            state::{PipelineEvent, WorkProduct, step},
            workflow::{Approver, Stage},
        },
        traits::Store,
    };
    for changes in [false, true] {
        let lab = Lab::new().await;
        let native = Native::new();
        let mut workflow = Workflow::builtin_research();
        workflow.id = "human-mobile".into();
        workflow.stages[1].stage = Stage::Approval {
            by: Approver::Human,
            count: 1,
            bind_head: false,
        };
        lab.ctx.store.save_workflow(&workflow).await.unwrap();
        let mut state = PipelineState::new("t-human", crate::pipeline::validate(workflow).unwrap());
        state = step(&state, PipelineEvent::Start).unwrap().0;
        state = step(
            &state,
            PipelineEvent::WorkCompleted {
                stage_id: "work".into(),
                attempt: 1,
                product: WorkProduct::Result {
                    summary: "Prepared result".into(),
                    output: None,
                },
            },
        )
        .unwrap()
        .0;
        let mut task = Task::new("t-human", "mobile approval", "general", "human-mobile", 1);
        task.status = TaskStatus::Running;
        lab.ctx
            .store
            .create_pipeline_task(&task, &serde_json::to_string(&state.snapshot()).unwrap(), 1)
            .await
            .unwrap();
        lab.ctx.pipeline.wake();
        let item = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(item) = lab.ctx.fleet.attention("approval:t-human/review/1") {
                    break item;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut notice = worker::notice(&item).unwrap();
        notice.task_version = Some((
            lab.ctx
                .store
                .load_task("t-human")
                .await
                .unwrap()
                .unwrap()
                .version,
            lab.ctx
                .store
                .progress("t-human")
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
        super::delivery::TelegramNotifier::new(
            native.api.clone(),
            lab.ctx.store.clone(),
            lab.destination.clone(),
        )
        .resume(&row.id)
        .await
        .unwrap();
        let receipt = native.receipts.lock().unwrap().last().unwrap().clone();
        let button = if changes { 1 } else { 0 };
        let callback = json!({"update_id":1,"callback_query":{"id":"human","from":{"id":7,"is_bot":false},"message":receipt,"data":receipt["reply_markup"]["inline_keyboard"][button][0]["callback_data"]}});
        native.updates.lock().unwrap().push(callback.clone());
        let (_stop, stopped) = tokio::sync::watch::channel(false);
        poll::once(
            &lab.ctx,
            &lab.config,
            native.api.clone(),
            &lab.destination,
            &stopped,
        )
        .await
        .unwrap();
        if changes {
            assert_eq!(
                lab.ctx.fleet.attention("approval:t-human/review/1"),
                Some(item)
            );
            assert_eq!(
                lab.ctx
                    .store
                    .progress("t-human")
                    .await
                    .unwrap()
                    .unwrap()
                    .data
                    .pipeline,
                serde_json::to_string(&state.snapshot()).unwrap()
            );
            assert!(native.calls.lock().unwrap().iter().any(|(m,v)| m=="answerCallbackQuery" && v["text"]=="Reply to the notification with the requested changes. Nothing applied yet."));
            let empty = json!({"update_id":2,"message":{"from":{"id":7,"is_bot":false},"chat":{"id":42},"reply_to_message":receipt,"text":" \n "}});
            native.updates.lock().unwrap().push(empty);
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
                lab.ctx
                    .fleet
                    .attention("approval:t-human/review/1")
                    .is_some()
            );
            assert_eq!(
                lab.ctx
                    .store
                    .progress("t-human")
                    .await
                    .unwrap()
                    .unwrap()
                    .data
                    .pipeline,
                serde_json::to_string(&state.snapshot()).unwrap()
            );
            let reply = json!({"update_id":3,"message":{"from":{"id":7,"is_bot":false},"chat":{"id":42},"reply_to_message":receipt,"text":"Please preserve\n完整內容"}});
            native.updates.lock().unwrap().push(reply);
            poll::once(
                &lab.ctx,
                &lab.config,
                native.api.clone(),
                &lab.destination,
                &stopped,
            )
            .await
            .unwrap();
        }
        assert!(
            lab.ctx
                .fleet
                .attention("approval:t-human/review/1")
                .is_none()
        );
        let progress = lab.ctx.store.progress("t-human").await.unwrap().unwrap();
        let persisted: Value = serde_json::from_str(&progress.data.pipeline).unwrap();
        if changes {
            assert_eq!(persisted["stage_index"], 0);
            assert_eq!(
                persisted["pending_work_reason"],
                "changes requested by operator: Please preserve\n完整內容"
            );
        } else {
            assert_eq!(
                lab.ctx
                    .store
                    .load_task("t-human")
                    .await
                    .unwrap()
                    .unwrap()
                    .task
                    .status,
                TaskStatus::Done
            );
        }
        let mut repeated = callback;
        repeated["update_id"] = json!(4);
        native.updates.lock().unwrap().push(repeated);
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
            native
                .calls
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(method, _)| method == "answerCallbackQuery")
                .is_some_and(
                    |(_, value)| value["text"] == "Not applied; open the current needs-you item"
                )
        );
        assert_eq!(
            lab.ctx
                .store
                .progress("t-human")
                .await
                .unwrap()
                .unwrap()
                .data
                .pipeline,
            progress.data.pipeline
        );
    }
}

#[tokio::test]
async fn unknown_notification_is_local_operator_only_and_never_claims_receipt() {
    use crate::handlers::{self, Outcome};
    use agend_core::{
        protocol::client::*,
        telegram::TelegramDelivery,
        traits::{Notification, NotificationSeverity},
    };
    let lab = Lab::new().await;
    let row = TelegramDelivery::new(
        "unknown-local".into(),
        lab.destination.clone(),
        Notification {
            severity: NotificationSeverity::Error,
            title: "Uncertain".into(),
            body: "Full evidence".into(),
            task_id: None,
        },
        1,
    );
    lab.ctx.store.enqueue_telegram(&row).await.unwrap();
    lab.ctx.store.claim_telegram_part(&row.id, 0).await.unwrap();
    handlers::telegram_attention::refresh(&lab.ctx, "")
        .await
        .unwrap();
    assert!(
        lab.ctx.fleet.view().attention.is_empty(),
        "active send is not unknown"
    );
    lab.ctx.store.mark_telegram_unknown(&row.id).await.unwrap();
    handlers::telegram_attention::refresh(&lab.ctx, "")
        .await
        .unwrap();
    let id = format!("telegram-delivery:{}", row.id);
    assert_eq!(
        lab.ctx.fleet.attention(&id).unwrap().actions,
        vec![AttentionAction::Abandon]
    );
    let request = |action| ClientRequest::ResolveAttention {
        data: ResolveAttentionData {
            request_id: "dispose".into(),
            attention_id: id.clone(),
            action,
            note: Some("Operator ends uncertain notification".into()),
        },
    };
    assert!(
        matches!(handlers::handle(&lab.ctx, Some("agent"), request(AttentionAction::Abandon)).await, Outcome::Reply(ClientResponse::Error { data }) if data.code == error_code::FORBIDDEN)
    );
    assert!(matches!(
        handlers::handle(&lab.ctx, None, request(AttentionAction::Retry)).await,
        Outcome::Reply(ClientResponse::Error { .. })
    ));
    // The notification failure must not produce another Telegram failure loop.
    let native = Native::new();
    let w = worker::start_with_api(
        lab.config.clone(),
        native.api.clone(),
        lab.ctx.store.clone(),
        lab.ctx.fleet.clone(),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    w.stop().await;
    assert!(
        !native
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(m, _)| m == "sendMessage")
    );
    assert!(matches!(
        handlers::handle(&lab.ctx, None, request(AttentionAction::Abandon)).await,
        Outcome::Reply(ClientResponse::CommandResult { .. })
    ));
    assert!(lab.ctx.fleet.attention(&id).is_none());
    assert!(
        !lab.ctx
            .store
            .confirm_telegram_part(&row.id, 0, 500)
            .await
            .unwrap()
    );
    assert!(!lab.ctx.store.claim_telegram_part(&row.id, 0).await.unwrap());
    let saved = lab
        .ctx
        .store
        .telegram_delivery(&row.id)
        .await
        .unwrap()
        .unwrap();
    assert!(saved.outcome_unknown && saved.in_flight && saved.abandoned);
    assert!(saved.message_ids.is_empty());
    assert_eq!(
        saved.abandoned_by_operator.as_deref(),
        Some("Operator ends uncertain notification")
    );
    handlers::telegram_attention::refresh(&lab.ctx, "")
        .await
        .unwrap();
    assert!(lab.ctx.fleet.attention(&id).is_none());
    assert!(matches!(
        handlers::handle(&lab.ctx, None, request(AttentionAction::Abandon)).await,
        Outcome::Reply(ClientResponse::Error { .. })
    ));
}

fn pairing_message(command: &str, date: u64) -> Value {
    // Mutate the recorded native message producer into a human /start update.
    let mut message =
        serde_json::from_str::<Value>(include_str!("../../tests/fixtures/telegram/message.json"))
            .unwrap()["result"]
            .clone();
    message["from"]["id"] = json!(42);
    message["from"]["is_bot"] = json!(false);
    message["text"] = json!(command);
    message["date"] = json!(date);
    json!({"update_id":100,"message":message})
}

#[tokio::test]
async fn native_pairing_requires_fresh_challenge_and_exact_operator_confirmation() {
    use super::pairing;
    use agend_core::telegram::pairing::PAIRING_WINDOW_MS;
    use agend_testkit::fakes::FakeClock;
    let native = Native::new();
    let clock = FakeClock::new(1_791_367_350_123);
    let pending = pairing::begin(
        native.api.clone(),
        "11111111-1111-4111-8111-111111111111".into(),
        SecretRef::File("/private/test-token".into()),
        &clock,
    )
    .await
    .unwrap();
    assert!(pending.candidate.is_none());
    assert_eq!(pending.bot_id, 123456789);
    assert_eq!(pending.bot_username, "agend_fixture_bot");
    let update = pairing_message(&pending.command(), clock.peek() / 1000);
    *native.updates.lock().unwrap() = vec![update];
    let observed = pairing::poll(native.api.clone(), &pending, &clock)
        .await
        .unwrap();
    assert!(
        pending.candidate.is_none(),
        "caller persists a new snapshot, never a partial mutation"
    );
    assert_eq!(observed.offset, 101);
    let candidate = observed.candidate.as_ref().unwrap();
    assert_eq!(
        (candidate.chat_id, candidate.user_id, candidate.topic_id),
        (42, 42, None)
    );
    let mut wrong = candidate.clone();
    wrong.user_id = 43;
    assert!(observed.confirm(&wrong, &clock).is_err());
    assert!(pending.confirm(candidate, &clock).is_err());
    let config = pairing::confirm(native.api.clone(), &observed, candidate, &clock)
        .await
        .unwrap();
    assert!(config.allows(42, 42, false));
    assert!(!config.allows(42, 43, false));
    assert!(!config.allows(42, 42, true));
    native.bot.lock().unwrap()["id"] = json!(987654321);
    assert!(
        pairing::confirm(native.api.clone(), &observed, candidate, &clock)
            .await
            .is_err()
    );
    assert!(
        pairing::poll(native.api.clone(), &pending, &clock)
            .await
            .is_err()
    );
    native.bot.lock().unwrap()["id"] = json!(123456789);
    let calls = native.calls.lock().unwrap().len();
    assert_eq!(
        pairing::poll(native.api.clone(), &observed, &clock)
            .await
            .unwrap(),
        observed
    );
    assert_eq!(native.calls.lock().unwrap().len(), calls);
    clock.advance(PAIRING_WINDOW_MS);
    assert!(observed.confirm(candidate, &clock).is_err());
    assert!(
        pairing::poll(native.api.clone(), &pending, &clock)
            .await
            .is_err()
    );
    assert_eq!(
        native.calls.lock().unwrap().len(),
        calls,
        "expired pairing performs no HTTP"
    );
    assert!(
        native
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|(method, _)| matches!(method.as_str(), "getMe" | "getUpdates"))
    );
}

#[tokio::test]
async fn native_pairing_refuses_forwarded_stale_ambiguous_and_malformed_updates() {
    use super::pairing;
    use agend_testkit::fakes::FakeClock;
    let native = Native::new();
    let clock = FakeClock::new(1_791_367_350_123);
    let pending = pairing::begin(
        native.api.clone(),
        "11111111-1111-4111-8111-111111111111".into(),
        SecretRef::File("/private/test-token".into()),
        &clock,
    )
    .await
    .unwrap();
    let valid = pairing_message(&pending.command(), clock.peek() / 1000);
    for case in 0..10 {
        let mut update = valid.clone();
        match case {
            0 => update["message"]["text"] = json!("/start unrelated"),
            1 => update["message"]["date"] = json!(clock.peek() / 1000 - 1),
            2 => update["message"]["date"] = json!(clock.peek() / 1000 + 1),
            3 => update["message"]["from"]["is_bot"] = json!(true),
            4 => update["message"]["forward_origin"] = json!({"type":"user"}),
            5 => update["message"]["sender_chat"] = json!({"id":42}),
            6 => update["message"]["edit_date"] = json!(clock.peek() / 1000),
            7 => update["message"]["chat"]["type"] = json!("channel"),
            8 => update["message"]["message_thread_id"] = json!(7),
            9 => {
                update["message"]["text"] =
                    json!(pending.command().replace("/start", "/start@other_bot"))
            }
            _ => unreachable!(),
        }
        *native.updates.lock().unwrap() = vec![update];
        let result = pairing::poll(native.api.clone(), &pending, &clock)
            .await
            .unwrap();
        assert!(result.candidate.is_none(), "admitted case {case}");
    }
    *native.updates.lock().unwrap() = vec![valid.clone(), valid.clone()];
    assert!(
        pairing::poll(native.api.clone(), &pending, &clock)
            .await
            .is_err()
    );
    let mut second = valid.clone();
    second["update_id"] = json!(101);
    second["message"]["from"]["id"] = json!(43);
    *native.updates.lock().unwrap() = vec![valid.clone(), second];
    assert!(
        pairing::poll(native.api.clone(), &pending, &clock)
            .await
            .is_err()
    );
    assert!(pending.candidate.is_none());
    let mut forum = valid;
    forum["message"]["chat"]["id"] = json!(-10042);
    forum["message"]["chat"]["type"] = json!("supergroup");
    forum["message"]["is_topic_message"] = json!(true);
    forum["message"]["message_thread_id"] = json!(7);
    forum["message"]["text"] = json!(
        pending
            .command()
            .replace("/start", "/start@agend_fixture_bot")
    );
    *native.updates.lock().unwrap() = vec![forum];
    let result = pairing::poll(native.api.clone(), &pending, &clock)
        .await
        .unwrap();
    let config = result
        .confirm(result.candidate.as_ref().unwrap(), &clock)
        .unwrap();
    assert_eq!(config.needs_you_topic, Some(7));
    assert!(config.allows(-10042, 42, false));
}
