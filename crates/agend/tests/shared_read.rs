//! Native daemon socket + two TUI clients + SQLite reopen; no model or live bot.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::pipeline::{
    state::PipelineState,
    task::{Task, TaskStatus},
    workflow::Workflow,
};
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;
use agend_tui::{App, i18n::Language, source::client::ClientSource};
use std::{
    path::Path,
    time::{Duration, Instant},
};
fn app(socket: &Path) -> App {
    App::new(Box::new(ClientSource::new(socket, None)), Language::En)
}
#[test]
fn two_tuis_share_read_state_and_restart_keeps_the_item_open() {
    let lab = lab::Lab::with_prefix(Path::new(env!("CARGO_BIN_EXE_agend")), "g12d-read");
    let home = lab.home(0);
    let store = SqliteStore::open(&home, 0).unwrap();
    let mut task = Task::new("t-1", "shared read", "general", "research", 1);
    task.status = TaskStatus::Failed;
    let state = PipelineState::new(
        "t-1",
        agend_daemon::pipeline::validate(Workflow::builtin_research()).unwrap(),
    );
    block_on(store.create_pipeline_task(
        &task,
        &serde_json::to_string(&state.snapshot()).unwrap(),
        1,
    ))
    .unwrap();
    drop(store);
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    let socket = home.join("run/daemon.sock");
    let mut a = app(&socket);
    let mut b = app(&socket);
    let mut client = agend_client::Client::connect(&socket, None).unwrap();
    let item = client
        .get_fleet()
        .unwrap()
        .attention
        .into_iter()
        .find(|i| i.attention_id.as_deref() == Some("task-failed:t-1"))
        .unwrap();
    let key = item.read_key().unwrap();
    assert!(!a.read.contains(&key));
    assert!(!b.read.contains(&key));
    a.event(ratatui::crossterm::event::Event::Key(
        ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Enter,
            ratatui::crossterm::event::KeyModifiers::NONE,
        ),
    ));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !b.read.contains(&key) && Instant::now() < deadline {
        b.tick();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(a.read.contains(&key) && b.read.contains(&key));
    assert!(
        client
            .mark_attention_read("task-failed:t-1", "task-failed:t-1#99")
            .is_err()
    );
    assert!(
        client
            .get_fleet()
            .unwrap()
            .attention
            .iter()
            .any(|i| i.attention_id == item.attention_id)
    );
    drop((a, b, client));
    daemon.interrupt().unwrap();
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    let restored = app(&socket);
    assert!(restored.read.contains(&key));
    let mut client = agend_client::Client::connect(&socket, None).unwrap();
    assert_eq!(client.get_fleet().unwrap().read_keys, vec![key]);
    assert!(
        client
            .get_fleet()
            .unwrap()
            .attention
            .iter()
            .any(|i| i.attention_id == item.attention_id)
    );
    drop((restored, client));
    daemon.interrupt().unwrap();
    assert!(lab.running_holders().is_empty());
}
