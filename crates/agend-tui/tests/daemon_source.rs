//! The screens against the testkit fake daemon over its real socket
//! (client protocol v1): same screens, a different source. The fake daemon
//! runs in-process in its own temp dir; no other process is started.

mod common;
#[path = "../examples/support/daemon_source.rs"]
mod daemon_source;

use agend_core::protocol::ask::AskReply;
use agend_core::protocol::client::{ClientRequest, DaemonEvent, TaskChangedData};
use agend_testkit::fake_daemon::FakeDaemon;
use agend_tui::App;
use agend_tui::app::Connection;
use agend_tui::i18n::Language;
use agend_tui::source::scripted::demo_catalog;
use common::*;
use daemon_source::{DaemonSource, seed_demo};
use ratatui::crossterm::event::KeyCode::Enter;
use std::time::{Duration, Instant};

fn connected_app(daemon: &FakeDaemon) -> (App, daemon_source::Address) {
    let (source, address) = DaemonSource::new(daemon.socket_path().to_path_buf(), demo_catalog());
    let mut app = App::new(Box::new(source), Language::En);
    assert!(app.is_connected(), "{:?}", app.connection);
    wait_until(&mut app, |text| text.contains("Needs you · 3"));
    (app, address)
}

/// Ticks until the rendered screen satisfies `ok` (events arrive on a thread).
fn wait_until(app: &mut App, ok: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.tick();
        let text = render(app);
        if ok(&text) {
            return text;
        }
        assert!(Instant::now() < deadline, "timed out; last screen:\n{text}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn home_from_the_fake_daemon_matches_the_scripted_source() {
    let daemon = FakeDaemon::start().unwrap();
    seed_demo(&daemon);
    let (mut app, _) = connected_app(&daemon);
    let (mut scripted, _) = demo(Language::En);
    assert_eq!(
        render(&mut app),
        render(&mut scripted),
        "screens cannot tell the sources apart"
    );
    // Live events after the backlog show up too.
    daemon.emit(DaemonEvent::TaskChanged {
        data: TaskChangedData {
            task_id: "T-90".into(),
            summary: "survey outline pushed".into(),
        },
    });
    wait_until(&mut app, |t| {
        t.contains("→ T-90 survey outline pushed — Survey fuzzers")
    });
}

#[test]
fn answering_goes_over_the_socket_and_removes_the_item() {
    let daemon = FakeDaemon::start().unwrap();
    seed_demo(&daemon);
    let (mut app, _) = connected_app(&daemon);
    press(&mut app, &[Enter, Enter, Enter]);
    let text = wait_until(&mut app, |t| t.contains("Needs you · 2 open"));
    assert!(text.contains("Answer sent to A-1: fixed seed 42"), "{text}");
    let answered = daemon.requests().into_iter().any(|r| {
        matches!(r, ClientRequest::AnswerAsk { data }
            if data.ask_id == "A-1" && data.reply == AskReply::Choice { option: "fixed seed 42".into() })
    });
    assert!(answered, "the daemon received the answer");
}

#[test]
fn terminal_snapshot_comes_from_the_daemon() {
    let daemon = FakeDaemon::start().unwrap();
    seed_demo(&daemon);
    let (mut app, _) = connected_app(&daemon);
    press(&mut app, &[ch('t')]);
    let text = render(&mut app);
    assert!(
        text.contains("━━ Terminal of dev-2 · read-only snapshot"),
        "{text}"
    );
    assert!(text.contains("│ fake screen of dev-2"));
}

#[test]
fn stopping_the_daemon_shows_disconnected_and_a_new_one_reconnects() {
    let daemon = FakeDaemon::start().unwrap();
    seed_demo(&daemon);
    let (mut app, address) = connected_app(&daemon);
    drop(daemon);
    let text = wait_until(&mut app, |t| t.contains("Daemon disconnected"));
    assert!(text.contains("the daemon closed the connection"), "{text}");
    let text = wait_until(&mut app, |t| t.contains("Reconnect attempt"));
    assert!(text.contains("cannot connect to"), "{text}");
    assert!(matches!(app.connection, Connection::Disconnected { .. }));

    let fresh = FakeDaemon::start().unwrap();
    seed_demo(&fresh);
    *address.lock().unwrap() = fresh.socket_path().to_path_buf();
    let text = wait_until(&mut app, |t| t.contains("Needs you · 3"));
    assert!(text.contains("Reconnected to the daemon."));
}
