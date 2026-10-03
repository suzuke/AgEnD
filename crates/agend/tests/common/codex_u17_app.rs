//! Full App/client/daemon/holder path with the same fake remote Codex thread.
use super::{ID, agend_bin, codex, history, lab, probe};
use agend_client::{Client, Redo};
use agend_core::protocol::client::*;
use agend_core::protocol::terminal::TerminalSize;
use agend_daemon::runtime::files;
use agend_testkit::fake_agent::codex::Probe;
use agend_tui::{App, i18n::Language, source::client::ClientSource};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const QUEUED: &str = "00000000-0000-4000-8000-000000000017";
const IDLE: &str = "00000000-0000-4000-8000-000000000018";

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn wait(app: &mut App, mut check: impl FnMut(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.tick();
        if check(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "U17 App timeout: connected={} typing={:?} message={:?}",
            app.is_connected(),
            app.term.as_ref().map(|t| t.typing),
            app.message
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn ready(app: &App) -> bool {
    app.term
        .as_ref()
        .and_then(|t| t.full.as_ref())
        .is_some_and(|f| f.ready)
}
fn open(home: &Path, instance: &str, caller: Option<String>) -> (App, Arc<AtomicUsize>) {
    let source = ClientSource::new(&home.join(DAEMON_SOCKET), caller);
    let threads = source.threads();
    let mut app = App::new(Box::new(source), Language::En);
    app.resize(80, 24);
    key(&mut app, KeyCode::Char('/'));
    for value in instance.chars() {
        key(&mut app, KeyCode::Char(value));
    }
    key(&mut app, KeyCode::Enter);
    wait(&mut app, ready);
    (app, threads)
}
fn acquire(app: &mut App) {
    key(app, KeyCode::Char('i'));
    wait(app, |a| a.term.as_ref().is_some_and(|t| t.typing));
}
fn frame(app: &App) -> &agend_core::protocol::terminal::TerminalFrame {
    &app.term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .data
        .as_ref()
        .unwrap()
        .frame
}
fn all(probe: &mut Probe, thread: &str) -> Vec<Value> {
    probe
        .call("thread/turns/list", json!({"threadId":thread}))
        .unwrap()["data"]
        .as_array()
        .unwrap()
        .clone()
}
fn visible(app: &mut App, probe: &mut Probe, thread: &str, expected: &str) {
    wait(app, |_| {
        history::user_items(&all(probe, thread))
            .iter()
            .any(|(_, item)| item.text == expected)
    });
}
fn command(home: &Path, command: AgentCommand) -> CommandResult {
    let mut client = Client::connect_once(&home.join(DAEMON_SOCKET), Some(ID.into())).unwrap();
    let request = ClientRequest::Command {
        data: ClientCommandData {
            request_id: client.next_request_id(),
            command,
        },
    };
    match client.request(&request, Redo::Never).unwrap() {
        ClientResponse::CommandResult { data } => data.result,
        other => panic!("unexpected command response {other:?}"),
    }
}
fn count(home: &Path) -> usize {
    match command(
        home,
        AgentCommand::Inbox {
            after_message_id: None,
        },
    ) {
        CommandResult::Messages { data } => data.messages.len(),
        other => panic!("unexpected inbox response {other:?}"),
    }
}
fn send(home: &Path, id: &str, body: &str) {
    assert_eq!(
        command(
            home,
            AgentCommand::Send {
                to: ID.into(),
                message: body.into(),
                level: Some(MessageLevel::Queue),
                message_id: Some(id.into())
            }
        ),
        CommandResult::Accepted
    );
}
fn launch_version(home: &Path) -> String {
    agend_daemon::driver::codex::launch::launched_version(home, ID).unwrap()
}
fn stored(home: &Path) -> Vec<agend_daemon::store::Message> {
    // The daemon owns SQLite exclusively. Inspect durable rows only after
    // that child has exited; live checks use the actual client inbox.
    let store = agend_daemon::store::SqliteStore::open(home, 0).unwrap();
    agend_testkit::block_on(store.messages_to(ID)).unwrap()
}
fn diagnostic(lab: &mut lab::Lab) {
    let script = lab.root.join("u17-daemon");
    let executable = std::env::current_exe()
        .unwrap()
        .display()
        .to_string()
        .replace('\'', "'\\''");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nexec '{executable}' --exact native_u17_daemon --nocapture\n"),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    lab.agend = script;
}
pub fn daemon() {
    if std::env::var("AGEND_U17_TEST_DAEMON").as_deref() != Ok("1") {
        return;
    }
    let home = std::env::var_os("AGEND_HOME").unwrap().into();
    assert_eq!(
        agend_daemon::daemon::run_u17_probe(home, agend_bin(), ID.into()),
        std::process::ExitCode::SUCCESS
    );
}
pub fn full_path() {
    full_path_with_version(false);
}
pub fn approved_full_path() {
    full_path_with_version(true);
}
fn full_path_with_version(approved: bool) {
    let mut lab = lab::Lab::with_prefix(&agend_bin(), "g11u17app");
    let home = lab.home(1);
    let instance = if approved {
        super::fixture_support::fixture_version(&lab, &home, Some("codex-cli 0.159.3"))
    } else {
        super::fixture(&lab, &home)
    };
    // A second Codex instance uses the same real profile but is outside the
    // explicit verification scope.
    let other = if approved {
        codex::fake_codex().unwrap()
    } else {
        Path::new(&instance.program).to_path_buf()
    };
    codex::add(&home, "g11-codex-other", &other, 1500).unwrap();
    let _socket = codex::BoundSocket::of(&home, ID);
    let _other_socket = codex::BoundSocket::of(&home, "g11-codex-other");
    if !approved {
        diagnostic(&mut lab);
    }
    let flags: &[(&str, &str)] = if approved {
        &[]
    } else {
        &[("AGEND_U17_TEST_DAEMON", "1"), ("AGEND_U17_PROBE", "1")]
    };
    let mut daemon = lab::Daemon::start(&lab, &home, flags).unwrap();
    daemon.ready().unwrap();
    let line = daemon.expect(&format!("{ID}: go (resume ")).unwrap();
    let thread = line
        .split("go (resume ")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap()
        .to_owned();
    let holder = files::running(&home, ID).unwrap().unwrap();
    let mut observer = probe(&home, &thread);
    let (mut app, threads) = open(&home, ID, None);
    wait(&mut app, |a| frame(a).modes.bracketed_paste);
    let generation = frame(&app).generation.clone();
    acquire(&mut app);
    app.paste("human 繁中\nsecond line");
    key(&mut app, KeyCode::Enter);
    visible(&mut app, &mut observer, &thread, "human 繁中\nsecond line");
    let manual_turn = history::user_items(&all(&mut observer, &thread))
        .into_iter()
        .find(|(_, i)| i.text == "human 繁中\nsecond line")
        .unwrap()
        .1
        .turn_id;
    daemon
        .expect(&format!("{ID}: U17 turn {manual_turn} busy"))
        .unwrap();
    assert_eq!(count(&home), 0, "manual turn invented a daemon receipt");
    send(&home, QUEUED, "QUEUED-AUTOMATED");
    daemon
        .expect(&format!("{QUEUED} (queue) → thread/queue/add"))
        .unwrap();
    let queue = observer
        .call("thread/queue/list", json!({"threadId":thread}))
        .unwrap();
    assert_eq!(queue["data"].as_array().unwrap().len(), 1);
    assert_eq!(queue["data"][0]["clientUserMessageId"], QUEUED);
    daemon
        .expect(&format!("{QUEUED} confirmed (turn "))
        .unwrap();
    let first = history::user_items(&all(&mut observer, &thread));
    assert_eq!(first.len(), 2);
    assert_eq!(
        first
            .iter()
            .filter(|(_, i)| i.client_id.as_deref() == Some(QUEUED))
            .count(),
        1
    );
    assert_eq!(count(&home), 1);
    if approved {
        // A replaced executable must not change the version of the surviving holder.
        std::fs::write(lab.root.join("advertised-version"), "codex-cli 0.159.4\n").unwrap();
        assert_eq!(launch_version(&home).trim(), "codex-cli 0.159.3");
    }
    // A partially entered frontend prompt belongs to the holder, not daemon.
    app.paste("draft survives restart ");
    wait(&mut app, |a| {
        super::text(frame(a)).contains("FAKE-MANUAL-DRAFT draft survives restart ")
    });
    daemon.interrupt().unwrap();
    let rows = stored(&home);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, QUEUED);
    assert_eq!(rows[0].state, agend_core::model::DeliveryState::Confirmed);
    assert_eq!(
        rows[0].turn_id.as_deref(),
        first
            .iter()
            .find(|(_, i)| i.client_id.as_deref() == Some(QUEUED))
            .map(|(_, i)| i.turn_id.as_str())
    );
    wait(&mut app, |a| !a.is_connected());
    assert!(app.term.as_ref().is_none_or(|term| !term.typing));
    app.paste("DISCONNECTED-MUST-NOT-ARRIVE");
    assert_eq!(files::running(&home, ID).unwrap(), Some(holder));
    let mut daemon = lab::Daemon::start(&lab, &home, flags).unwrap();
    daemon.ready().unwrap();
    daemon
        .expect(&format!("{ID}: thread {thread} resumed"))
        .unwrap();
    wait(&mut app, |a| a.is_connected() && ready(a));
    assert_eq!(files::running(&home, ID).unwrap(), Some(holder));
    assert_eq!(frame(&app).generation, generation);
    assert!(
        !app.term.as_ref().unwrap().typing,
        "reconnect restored input without explicit i"
    );
    app.paste("READONLY-MUST-NOT-ARRIVE");
    app.resize(60, 16);
    acquire(&mut app);
    assert_eq!(
        frame(&app).size,
        TerminalSize {
            rows: 15,
            columns: 60
        }
    );
    wait(&mut app, |_| {
        all(&mut observer, &thread)
            .iter()
            .all(|turn| turn["status"] != "inProgress")
    });
    key(&mut app, KeyCode::Enter);
    visible(&mut app, &mut observer, &thread, "draft survives restart ");
    assert_eq!(count(&home), 1, "restarted human turn invented a receipt");
    wait(&mut app, |_| {
        all(&mut observer, &thread)
            .iter()
            .all(|turn| turn["status"] != "inProgress")
    });
    let manual_turn = history::user_items(&all(&mut observer, &thread))
        .into_iter()
        .find(|(_, i)| i.text == "draft survives restart ")
        .unwrap()
        .1
        .turn_id;
    // Backend history can turn idle before its status notification reaches
    // this daemon connection. Wait for the driver's actual observation.
    daemon
        .expect(&format!("{ID}: U17 turn {manual_turn} idle"))
        .unwrap();
    send(&home, IDLE, "IDLE-AUTOMATED");
    daemon
        .expect(&format!("{IDLE} (queue) → turn/start"))
        .unwrap();
    daemon.expect(&format!("{IDLE} confirmed (turn ")).unwrap();
    // The same id after reconnect is still one daemon row / one user item.
    send(&home, QUEUED, "QUEUED-AUTOMATED");
    let items = history::user_items(&all(&mut observer, &thread));
    assert_eq!(items.len(), 4);
    assert_eq!(
        items
            .iter()
            .filter(|(_, i)| i.client_id.as_deref() == Some(QUEUED))
            .count(),
        1
    );
    assert_eq!(
        items
            .iter()
            .filter(|(_, i)| i.client_id.as_deref() == Some(IDLE))
            .count(),
        1
    );
    assert_eq!(count(&home), 2);

    let (mut denied, denied_threads) = open(&home, "g11-codex-other", None);
    key(&mut denied, KeyCode::Char('i'));
    wait(&mut denied, |a| {
        a.message
            .as_ref()
            .is_some_and(|s| s.contains("not_supported"))
    });
    assert!(
        !denied.term.as_ref().unwrap().typing,
        "probe scope leaked into another instance"
    );
    denied.paste("FOREIGN-MUST-NOT-ARRIVE");
    drop(denied);
    assert_eq!(denied_threads.load(Ordering::SeqCst), 0);
    let (mut agent, agent_threads) = open(&home, ID, Some(ID.into()));
    key(&mut agent, KeyCode::Char('i'));
    wait(&mut agent, |a| {
        a.message
            .as_ref()
            .is_some_and(|s| s.contains("only the operator"))
    });
    assert!(
        !agent.term.as_ref().unwrap().typing,
        "agent caller obtained probe control"
    );
    drop(agent);
    assert_eq!(agent_threads.load(Ordering::SeqCst), 0);
    assert_eq!(count(&home), 2);
    drop(app);
    assert_eq!(threads.load(Ordering::SeqCst), 0);
    daemon.interrupt().unwrap();
    let rows = stored(&home);
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(row.state, agend_core::model::DeliveryState::Confirmed);
        assert_eq!(
            row.turn_id.as_deref(),
            items
                .iter()
                .find(|(_, i)| i.client_id.as_deref() == Some(row.id.as_str()))
                .map(|(_, i)| i.turn_id.as_str()),
            "manual turn stole the daemon receipt"
        );
    }
    lab.stop_all_holders();
    assert!(lab.running_holders().is_empty());
    println!(
        "U17 full App/client/daemon/holder: same thread {thread}, holder {holder}, generation; manual busy/idle drive queue/start; two rows/two receipts, no duplicate; draft/reconnect/scope/caller/cleanup passed"
    );
}
pub fn default_denied() {
    denied_version(None);
}
pub fn denied_version(version: Option<&str>) {
    let lab = lab::Lab::with_prefix(&agend_bin(), "g11u17deny");
    let home = lab.home(1);
    super::fixture_support::fixture_version(&lab, &home, version);
    let _socket = codex::BoundSocket::of(&home, ID);
    // Even an exported diagnostic variable cannot opt the normal binary in.
    let mut daemon = lab::Daemon::start(&lab, &home, &[("AGEND_U17_PROBE", "1")]).unwrap();
    daemon.ready().unwrap();
    let (mut app, threads) = open(&home, ID, None);
    let size = frame(&app).size;
    key(&mut app, KeyCode::Char('i'));
    wait(&mut app, |a| {
        a.message
            .as_ref()
            .is_some_and(|s| s.contains("not_supported"))
    });
    assert!(!app.term.as_ref().unwrap().typing);
    app.paste("DEFAULT-MUST-NOT-ARRIVE");
    assert_eq!(frame(&app).size, size);
    drop(app);
    assert_eq!(threads.load(Ordering::SeqCst), 0);
    daemon.interrupt().unwrap();
    let store = agend_daemon::store::SqliteStore::open(&home, 0).unwrap();
    assert!(
        agend_testkit::block_on(store.messages_to(ID))
            .unwrap()
            .is_empty()
    );
    println!("normal agend daemon still denies Codex input, including with AGEND_U17_PROBE=1");
}
