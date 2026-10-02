//! U17 foundation: real holder/wrapper/raw fake TUI/app-server/driver/SQLite.
//! Foundation cases and a full diagnostic App/client/daemon path are distinct.
//! The normal daemon's Codex policy remains denied; no real LLM is run.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/codex_process.rs"]
mod codex;
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::model::DeliveryState;
use agend_core::policy::busy::BusyLevel;
use agend_core::protocol::terminal::{
    TerminalControlOperation as Op, TerminalFrame, TerminalSize, TerminalViewport,
};
use agend_core::traits::{AgentMessage, Driver, DriverEventKind, HolderLaunch};
use agend_daemon::driver::codex::{CodexDriver, history, launch};
use agend_daemon::runtime::{HolderRuntime, files};
use agend_daemon::store::{Instance, SqliteStore};
use agend_testkit::fake_agent::codex::Probe;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
const BIN: &str = env!("CARGO_BIN_EXE_agend");
const ID: &str = "g11-codex";
fn agend_bin() -> std::path::PathBuf {
    Path::new(BIN).to_path_buf()
}
#[path = "common/codex_u17_fixture.rs"]
mod fixture_support;
use fixture_support::{fixture, probe, text};
struct Boot {
    runtime: HolderRuntime,
    driver: CodexDriver,
    store: Arc<SqliteStore>,
}
impl Boot {
    async fn start(home: &Path, instance: &Instance, previous: Option<u32>) -> Self {
        Self::with_policy(home, instance, previous, Default::default()).await
    }
    async fn with_policy(
        home: &Path,
        instance: &Instance,
        previous: Option<u32>,
        policy: agend_core::policy::codex_input::CodexInputPolicy,
    ) -> Self {
        let store = Arc::new(SqliteStore::open(home, 0).unwrap());
        let runtime = HolderRuntime::new(
            home,
            Path::new(BIN),
            std::env::vars().collect(),
            Arc::new(|_| {}),
        );
        let spawn = HolderLaunch {
            instance_id: ID.into(),
            backend: instance.backend,
            executable: launch::SHELL.into(),
            args: launch::wrapper_args(home, instance).unwrap(),
            working_directory: instance.working_directory.clone(),
        };
        let started = match previous {
            Some(pid) => runtime.attach(&spawn, pid).await.unwrap(),
            None => runtime.start(&spawn).await.unwrap(),
        };
        let driver =
            CodexDriver::with_input_policy(home, Arc::clone(&store), Arc::new(|_| {}), policy);
        driver.connect(ID, started.generation).await.unwrap();
        let boot = Self {
            runtime,
            driver,
            store,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if text(&boot.frame().await).contains("FAKE-MANUAL-READY") {
                break;
            }
            assert!(Instant::now() < deadline, "fake manual TUI did not connect");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        boot
    }
    async fn frame(&self) -> TerminalFrame {
        self.runtime
            .terminal_connection(ID)
            .unwrap()
            .frame(TerminalViewport {
                top: None,
                rows: 20,
            })
            .await
            .unwrap()
    }
    async fn acquire(&self) -> String {
        let frame = self.frame().await;
        assert!(frame.modes.bracketed_paste);
        let attach = "manual-fixture".to_string();
        let ack = self
            .runtime
            .terminal_connection(ID)
            .unwrap()
            .control(
                frame.generation,
                Op::Acquire {
                    attach_id: attach.clone(),
                    size: TerminalSize {
                        rows: 20,
                        columns: 80,
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(ack.attach_id.as_deref(), Some(attach.as_str()));
        attach
    }
    async fn manual(&self, attach: &str, message: &str) {
        let frame = self.frame().await;
        let input = format!("\x1b[200~{message}\x1b[201~\r");
        self.runtime
            .terminal_connection(ID)
            .unwrap()
            .control(
                frame.generation,
                Op::Input {
                    attach_id: attach.into(),
                    bytes_base64: STANDARD.encode(input),
                },
            )
            .await
            .unwrap();
    }
    async fn thread(&self) -> String {
        self.store
            .instance(ID)
            .await
            .unwrap()
            .unwrap()
            .session_id
            .unwrap()
    }
    async fn busy(&self, expected: bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while self.driver.busy(ID) != Some(expected) {
            assert!(
                Instant::now() < deadline,
                "driver busy state did not become {expected}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
fn message(id: &str, body: &str) -> AgentMessage {
    AgentMessage {
        id: id.into(),
        from: "operator".into(),
        task_id: None,
        body: body.into(),
    }
}
fn turns(probe: &mut Probe, thread: &str) -> Vec<Value> {
    probe
        .call("thread/turns/list", json!({"threadId":thread}))
        .unwrap()["data"]
        .as_array()
        .unwrap()
        .clone()
}
async fn user_visible(probe: &mut Probe, thread: &str, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if history::user_items(&turns(probe, thread))
            .iter()
            .any(|(_, item)| item.text == expected)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "manual user message missing from same thread"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
fn run(test: impl AsyncFnOnce()) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(test());
}
#[test]
fn native_manual_turn_is_observed_and_queue_delivery_waits_for_its_own_user_item() {
    let native = lab::Lab::with_prefix(Path::new(BIN), "g11u17");
    let home = native.home(1);
    let instance = fixture(&native, &home);
    let _socket = codex::BoundSocket::of(&home, ID);
    run(async || {
        let boot = Boot::start(&home, &instance, None).await;
        let thread = boot.thread().await;
        let mut observer = probe(&home, &thread);
        let attach = boot.acquire().await;
        boot.manual(&attach, "manual 繁中\nsecond line").await;
        boot.busy(true).await;
        user_visible(&mut observer, &thread, "manual 繁中\nsecond line").await;
        assert!(
            boot.store.messages_to(ID).await.unwrap().is_empty(),
            "manual input invented a daemon message"
        );
        let receipt = boot
            .driver
            .deliver(ID, &message("u17-queued", "AUTOMATED"), BusyLevel::Queue)
            .await
            .unwrap();
        assert_eq!(receipt.state, DeliveryState::Sent);
        let queue = observer
            .call("thread/queue/list", json!({"threadId":thread}))
            .unwrap();
        assert_eq!(queue["data"].as_array().unwrap().len(), 1);
        assert_eq!(queue["data"][0]["clientUserMessageId"], "u17-queued");
        let row = boot.store.message("u17-queued").await.unwrap().unwrap();
        assert_eq!(row.state, DeliveryState::Sent);
        assert!(
            row.turn_id.is_none(),
            "queued delivery was falsely attributed to the manual turn"
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let row = boot.store.message("u17-queued").await.unwrap().unwrap();
            if row.state == DeliveryState::Confirmed {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "queued message never reached its own turn"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        boot.busy(false).await;
        let all = turns(&mut observer, &thread);
        let items = history::user_items(&all);
        assert_eq!(all.len(), 2);
        assert_eq!(items.len(), 2);
        let row = boot.store.message("u17-queued").await.unwrap().unwrap();
        assert_eq!(
            row.turn_id.as_deref(),
            items
                .iter()
                .find(|(_, item)| item.client_id.as_deref() == Some("u17-queued"))
                .map(|(_, item)| item.turn_id.as_str()),
            "confirmed queued message lost its actual turn"
        );
        assert_eq!(
            items
                .iter()
                .filter(|(_, item)| item.client_id.as_deref() == Some("u17-queued"))
                .count(),
            1
        );
        let events = boot.driver.events(ID, None).await.unwrap();
        assert_eq!(events.iter().filter(|event|matches!(&event.kind,DriverEventKind::MessageConfirmed{message_id} if message_id=="u17-queued")).count(),1);
        assert_eq!(boot.store.messages_to(ID).await.unwrap().len(), 1);
        println!(
            "same thread {thread}; manual busy/idle observed; one daemon row and one receipt; queue has its own turn"
        );
    });
}
#[test]
fn native_manual_text_cannot_confirm_an_unattempted_message_after_component_restart() {
    let native = lab::Lab::with_prefix(Path::new(BIN), "g11u17");
    let home = native.home(1);
    let instance = fixture(&native, &home);
    let _socket = codex::BoundSocket::of(&home, ID);
    run(async || {
        let boot = Boot::start(&home, &instance, None).await;
        let thread = boot.thread().await;
        let holder = files::running(&home, ID).unwrap().unwrap();
        let generation = boot.frame().await.generation;
        let attach = boot.acquire().await;
        boot.driver.disconnect(ID);
        let sent = message("u17-not-attempted", "identical manual text");
        let receipt = boot
            .driver
            .deliver(ID, &sent, BusyLevel::Queue)
            .await
            .unwrap();
        assert_eq!(receipt.state, DeliveryState::Queued);
        assert!(
            boot.store
                .message(&sent.id)
                .await
                .unwrap()
                .unwrap()
                .attempted_at_unix_ms
                .is_none()
        );
        let rendered = agend_daemon::delivery::render("operator", None, &sent.body);
        boot.manual(&attach, &rendered).await;
        let mut observer = probe(&home, &thread);
        user_visible(&mut observer, &thread, &rendered).await;
        drop(boot);
        let boot = Boot::start(&home, &instance, Some(holder)).await;
        assert_eq!(boot.thread().await, thread);
        assert_eq!(files::running(&home, ID).unwrap(), Some(holder));
        assert_eq!(boot.frame().await.generation, generation);
        let row = boot.store.message(&sent.id).await.unwrap().unwrap();
        assert_eq!(
            row.state,
            DeliveryState::Sent,
            "manual text forged a daemon receipt for a never-attempted queued row"
        );
        assert!(
            row.turn_id.is_none(),
            "manual turn stole the queued row's attribution"
        );
        let error = boot
            .runtime
            .terminal_connection(ID)
            .unwrap()
            .control(
                generation,
                Op::Input {
                    attach_id: attach,
                    bytes_base64: STANDARD.encode(b"OLD\r"),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "control_lost");
        let queue = observer
            .call("thread/queue/list", json!({"threadId":thread}))
            .unwrap();
        assert_eq!(queue["data"][0]["clientUserMessageId"], sent.id);
        println!(
            "same holder {holder}, generation and thread {thread}; unattempted text is not a receipt; old attach refused"
        );
    });
}

#[path = "common/codex_u17_app.rs"]
mod app_path;
#[test]
fn native_u17_daemon() {
    app_path::daemon();
}
#[test]
fn full_u17_app_client_daemon_path_keeps_manual_and_daemon_turns_distinct() {
    app_path::full_path();
}
#[test]
fn normal_daemon_does_not_enable_codex_input_from_probe_environment() {
    app_path::default_denied();
}

#[test]
fn manual_text_cannot_confirm_an_attempted_row_in_the_u17_input_scope() {
    let native = lab::Lab::with_prefix(Path::new(BIN), "g11u17id");
    let home = native.home(1);
    let instance = fixture(&native, &home);
    let _socket = codex::BoundSocket::of(&home, ID);
    run(async || {
        let policy =
            || agend_core::policy::codex_input::CodexInputPolicy::for_u17_verification(ID.into());
        let boot = Boot::with_policy(&home, &instance, None, policy()).await;
        let thread = boot.thread().await;
        let holder = files::running(&home, ID).unwrap().unwrap();
        let attach = boot.acquire().await;
        boot.driver.disconnect(ID);
        let sent = message("u17-attempted", "identical attempted text");
        assert_eq!(
            boot.driver
                .deliver(ID, &sent, BusyLevel::Queue)
                .await
                .unwrap()
                .state,
            DeliveryState::Queued
        );
        // The durable crash window between recording an attempt and its RPC.
        boot.store
            .mark_message_attempted(&sent.id, 1)
            .await
            .unwrap();
        let rendered = agend_daemon::delivery::render("operator", None, &sent.body);
        boot.manual(&attach, &rendered).await;
        let mut observer = probe(&home, &thread);
        user_visible(&mut observer, &thread, &rendered).await;
        drop(boot);
        let boot = Boot::with_policy(&home, &instance, Some(holder), policy()).await;
        let row = boot.store.message(&sent.id).await.unwrap().unwrap();
        assert_ne!(
            row.state,
            DeliveryState::Confirmed,
            "attempted row stole the human turn receipt"
        );
        assert!(
            row.turn_id.is_none(),
            "human turn was assigned to the daemon row"
        );
        let events = boot.driver.events(ID, None).await.unwrap();
        assert!(!events.iter().any(|e| matches!(&e.kind,DriverEventKind::MessageConfirmed { message_id } if message_id==&sent.id)),
            "history events invented a receipt for manual text");
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let row = boot.store.message(&sent.id).await.unwrap().unwrap();
            if row.state == DeliveryState::Confirmed {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "identified retry never confirmed after human turn ended"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        boot.busy(false).await;
        let items = history::user_items(&turns(&mut observer, &thread));
        assert_eq!(items.len(), 2);
        let own: Vec<_> = items
            .iter()
            .filter(|(_, i)| i.client_id.as_deref() == Some(sent.id.as_str()))
            .collect();
        assert_eq!(own.len(), 1);
        assert_eq!(
            boot.store
                .message(&sent.id)
                .await
                .unwrap()
                .unwrap()
                .turn_id
                .as_deref(),
            Some(own[0].1.turn_id.as_str())
        );
        println!(
            "U17 attempted crash row: manual matching text has no receipt; own identified retry confirms once"
        );
    });
}
