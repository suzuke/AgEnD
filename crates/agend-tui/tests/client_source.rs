//! `ClientSource` (the real `Source`, through `agend-client`) against the
//! testkit fake daemon's socket (gate 11 B P1–P7): the fleet view on the
//! screens, needs-you items that leave only on the daemon's event,
//! disconnects and reconnects, the live terminal, and typing. The fake runs
//! in-process in its own temp dir; no other process is started.

mod common;
#[path = "../examples/support/demo_daemon.rs"]
mod demo_daemon;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use agend_core::protocol::client::{
    AgentState, AttentionAction, AttentionRequiredData, ClientRequest, InstanceView,
    ResolveAttentionData, TaskView,
};
use agend_testkit::contract::client::proxy::{Direction, Options, Proxy, Transform};
use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};
use agend_testkit::tempdir::TempDir;
use agend_tui::App;
use agend_tui::app::{Connection, TermMode};
use agend_tui::i18n::Language;
use agend_tui::source::client::ClientSource;
use common::*;
use ratatui::crossterm::event::KeyCode::{Down, Enter, Esc, Left, Right};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

struct Lab {
    dir: TempDir,
}

impl Lab {
    fn new() -> Lab {
        Lab {
            dir: TempDir::new("g11-tui").unwrap(),
        }
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    fn daemon(&self) -> FakeDaemon {
        FakeDaemon::start_at(&self.socket()).unwrap()
    }

    /// The app on `socket` and its reader-thread count.
    fn app_on(
        &self,
        socket: PathBuf,
        caller: Option<&str>,
        lang: Language,
    ) -> (App, Arc<AtomicUsize>) {
        let source = ClientSource::new(&socket, caller.map(Into::into));
        let threads = source.threads();
        (App::new(Box::new(source), lang), threads)
    }

    fn app(&self, caller: Option<&str>) -> (App, Arc<AtomicUsize>) {
        self.app_on(self.socket(), caller, Language::En)
    }
}

fn instance(id: &str, backend: &str, state: AgentState) -> InstanceView {
    InstanceView {
        instance_id: id.into(),
        team_id: "general".into(),
        backend: backend.into(),
        state,
        working_directory: None,
    }
}

fn failed_item(id: &str) -> AttentionRequiredData {
    AttentionRequiredData {
        reason: format!("{id} failed: it still died"),
        task_id: None,
        ask: None,
        recap: None,
        attention_id: Some(format!("instance-failed:{id}")),
        unblocks: Some(0),
        waiting_since_unix_ms: Some(1),
        if_ignored: Some(format!("{id} stays stopped")),
        actions: vec![AttentionAction::Retry],
        instance_id: Some(id.into()),
    }
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

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, &[ch(c)]);
    }
}

/// `/` `<id>` Enter: the agent's terminal.
fn open_terminal(app: &mut App, id: &str) {
    press(app, &[ch('/')]);
    type_str(app, id);
    press(app, &[Enter]);
}

fn ctrl(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

#[test]
fn client_source_draws_the_fleet_view() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-1", "claude", AgentState::Unknown));
    daemon.set_instance(instance("g11-2", "claude", AgentState::Failed));
    daemon.add_attention(failed_item("g11-2"));
    let (mut app, _) = lab.app_on(lab.socket(), None, Language::ZhTw);
    let home = wait_until(&mut app, |t| t.contains("需要你 · 1"));
    assert!(
        line_with(&home, "g11-2 failed").ends_with("新  general · —"),
        "{home}"
    );
    // g11-2 needs you through the item's instance (P3); g11-1's state is unknown.
    assert!(
        line_with(&home, "┏ general").ends_with("! 1 需要你  ? 1 狀態不明"),
        "{home}"
    );
    assert!(home.contains("沒有進行中的目標"));
}

#[test]
fn the_demo_fleet_reads_the_same_needs_you_list_as_the_scripted_source() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    demo_daemon::seed(&daemon);
    let (mut app, _) = lab.app(None);
    let text = wait_until(&mut app, |t| t.contains("Needs you · 3"));
    let (mut scripted, _) = demo(Language::En);
    let scripted = render(&mut scripted);
    let needs = |t: &str| -> Vec<String> {
        t.lines()
            .filter(|l| l.starts_with('▌'))
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(needs(&text), needs(&scripted));
    for team in ["archfix", "research", "general"] {
        assert!(text.contains(&format!("┏ {team} ")), "{text}");
    }
    // From the fleet view, not a replay: the recent changes before connecting
    // are not events (gate 8 P4).
    assert!(!text.contains("T-37 merged"), "{text}");
}

#[test]
fn attention_resolved_from_the_daemon_removes_the_item() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-2", "claude", AgentState::Failed));
    daemon.add_attention(failed_item("g11-2"));
    let (mut app, _) = lab.app(None);
    wait_until(&mut app, |t| t.contains("Needs you · 1"));
    let (mut other, _) = ProbeClient::hello(&lab.socket(), None).unwrap();
    other
        .request(&ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: "r-1".into(),
                attention_id: "instance-failed:g11-2".into(),
                action: AttentionAction::Retry,
            },
        })
        .unwrap();
    wait_until(&mut app, |t| t.contains("Needs you · 0"));
}

#[test]
fn retry_leaves_only_when_the_event_arrives() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-2", "claude", AgentState::Failed));
    daemon.add_attention(failed_item("g11-2"));
    daemon.hold_resolved_events(true);
    let (mut app, _) = lab.app(None);
    wait_until(&mut app, |t| t.contains("Needs you · 1"));
    press(&mut app, &[Enter]);
    let open = render(&mut app);
    assert!(
        open.contains("┃    If you leave it: g11-2 stays stopped"),
        "{open}"
    );
    assert!(open.contains("┃    [1] Retry"), "{open}");
    press(&mut app, &[ch('1')]);
    assert!(render(&mut app).contains("Sent: Retry g11-2"));
    for _ in 0..20 {
        app.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        render(&mut app).contains("Needs you · 1 open"),
        "accepted is not resolved"
    );
    assert!(
        daemon.fleet().attention.is_empty(),
        "the daemon took it off"
    );
    daemon.release_resolved_events();
    wait_until(&mut app, |t| t.contains("Needs you · 0 open"));
}

#[test]
fn an_item_without_actions_says_so_and_errors_keep_it() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-2", "codex", AgentState::Failed));
    daemon.add_attention(AttentionRequiredData {
        actions: Vec::new(),
        if_ignored: Some("delete and re-add the instance (gate 9)".into()),
        ..failed_item("g11-2")
    });
    daemon.set_instance(instance("g11-3", "claude", AgentState::Failed));
    daemon.add_attention(AttentionRequiredData {
        waiting_since_unix_ms: Some(2),
        ..failed_item("g11-3")
    });
    let (mut app, _) = lab.app(Some("g11-1"));
    wait_until(&mut app, |t| t.contains("Needs you · 2"));
    press(&mut app, &[Enter]);
    let open = render(&mut app);
    assert!(open.contains("┃    No actions available"), "{open}");
    // As an agent, retry is refused by the daemon and the item stays.
    press(&mut app, &[Down, Down, Down, ch('1')]);
    let refused = render(&mut app);
    assert!(
        refused.contains("forbidden: only the operator can resolve needs-you items"),
        "{refused}"
    );
    assert!(refused.contains("Needs you · 2 open"));
}

/// A proxy that turns the task_changed "lag" into the daemon's
/// `event_gap` line (what a subscriber that fell behind gets).
fn gap_proxy(upstream: PathBuf) -> Proxy {
    let transform: Transform = Arc::new(|_, direction, line| {
        if direction == Direction::ToClient && line.contains("\"summary\":\"lag\"") {
            return vec![
                r#"{"type":"error","data":{"request_id":null,"code":"event_gap","message":"this client fell 1500 events behind"}}"#.into(),
            ];
        }
        vec![line]
    });
    Proxy::start(upstream, Options::rewrite(transform)).unwrap()
}

#[test]
fn event_gap_shows_disconnected_then_reconnects_with_a_new_fleet_view() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    demo_daemon::seed(&daemon);
    let proxy = gap_proxy(lab.socket());
    let (mut app, _) = lab.app_on(proxy.socket().to_path_buf(), None, Language::En);
    wait_until(&mut app, |t| t.contains("Needs you · 3"));
    press(&mut app, &[Down, Down, Down, Down, Right]);
    assert_eq!(first_line(&render(&mut app)), "AgEnD › archfix");
    daemon.set_task(TaskView {
        status: "lag".into(),
        ..demo_daemon::task_view(&agend_tui::source::scripted::demo_catalog().tasks[0])
    });
    let text = wait_until(&mut app, |t| t.contains("Daemon disconnected"));
    assert!(
        text.contains("event_gap: this client fell 1500 events behind"),
        "{text}"
    );
    let text = wait_until(&mut app, |t| t.contains("Reconnected to the daemon."));
    assert_eq!(first_line(&text), "AgEnD › archfix");
}

#[test]
fn a_daemon_restart_refetches_the_fleet_view_and_returns_to_the_screen() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    demo_daemon::seed(&daemon);
    let (mut app, threads) = lab.app(None);
    wait_until(&mut app, |t| t.contains("Needs you · 3"));
    press(&mut app, &[Down, Down, Down, Down, Right]);
    drop(daemon);
    let text = wait_until(&mut app, |t| t.contains("Reconnect attempt"));
    assert!(text.contains("the daemon closed the connection"), "{text}");
    assert!(text.contains("cannot reach the AgEnD daemon at"), "{text}");
    assert!(!text.contains("Restructure"), "no stale data");
    assert_eq!(threads.load(Ordering::SeqCst), 0, "no reader left");
    let fresh = lab.daemon();
    demo_daemon::seed(&fresh);
    fresh.set_task(TaskView {
        task_id: "T-99".into(),
        title: "Added while the TUI was away".into(),
        team_id: "archfix".into(),
        status: "open".into(),
        assignee: None,
        stages: Vec::new(),
        current_stage: None,
    });
    let text = wait_until(&mut app, |t| t.contains("Reconnected to the daemon."));
    assert_eq!(first_line(&text), "AgEnD › archfix");
    assert!(text.contains("● Added while the TUI was away"), "{text}");
    assert!(
        text.contains("open  T-99"),
        "no progress bar without stages: {text}"
    );
}

#[test]
fn a_version_mismatch_is_shown_and_not_retried() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_supported_versions(&[agend_core::protocol::client::V1_1]);
    let (mut app, _) = lab.app(None);
    let text = render(&mut app);
    assert!(
        text.contains("the daemon speaks client protocol 1.1; this agend needs 1.2"),
        "{text}"
    );
    assert!(text.contains("Not retrying: the versions do not match."));
    let before = daemon.requests().len();
    for _ in 0..10 {
        app.tick();
    }
    assert_eq!(daemon.requests().len(), before, "no automatic retry");
    press(&mut app, &[ch('r')]);
    assert_eq!(daemon.requests().len(), before + 1, "r tries once");
}

#[test]
fn the_terminal_is_live_and_throttled_and_draws_the_last_output() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-1", "claude", AgentState::Unknown));
    daemon.set_screen("g11-1", "counter=1\n");
    let (mut app, _) = lab.app(None);
    wait_until(&mut app, |t| t.contains("general ─"));
    open_terminal(&mut app, "g11-1");
    assert!(render(&mut app).contains("━━ Terminal of g11-1 · live"));
    let fetches = || {
        daemon
            .requests()
            .iter()
            .filter(|r| matches!(r, ClientRequest::SubscribeTerminal { .. }))
            .count()
    };
    let started = Instant::now();
    daemon.push_terminal_bytes("g11-1", b"counter=2\r\n");
    wait_until(&mut app, |t| t.contains("│ counter=2"));
    // 200 ms throttle + a 10 ms tick here + the round trip.
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
    let before = fetches();
    let burst = Instant::now();
    // The fake's screen grows with every line; stay within the 26 rows.
    for n in 3..=22 {
        daemon.push_terminal_bytes("g11-1", format!("counter={n}\r\n").as_bytes());
        std::thread::sleep(Duration::from_millis(50));
        app.tick();
    }
    let per_second = (fetches() - before) as f64 / burst.elapsed().as_secs_f64();
    assert!(per_second <= 5.5, "{per_second} fetches a second");
    // The last output arrives after a fetch and nothing follows it: drawn
    // anyway by the stale mark.
    daemon.push_terminal_bytes("g11-1", b"$ ");
    let text = wait_until(&mut app, |t| t.contains("│ $"));
    assert!(text.contains("│ counter=22"));
    // Idle: no output, no fetching. Output that arrived just before the
    // last fetch may still mark the screen stale once (P5): let that one
    // refetch happen (the refresh interval, plus a tick), then count.
    for _ in 0..40 {
        app.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
    let idle = fetches();
    for _ in 0..50 {
        app.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fetches(), idle);
}

#[test]
fn only_the_terminal_reconnects_when_its_connection_ends() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-1", "claude", AgentState::Unknown));
    daemon.set_screen("g11-1", "$ ");
    let (mut app, _) = lab.app(None);
    wait_until(&mut app, |t| t.contains("general ─"));
    open_terminal(&mut app, "g11-1");
    press(&mut app, &[ch('i')]);
    assert!(app.term.as_ref().unwrap().typing);
    daemon.drop_terminal_subscribers();
    let text = wait_until(&mut app, |t| t.contains("· ended, retrying"));
    assert!(app.is_connected(), "no disconnected screen: {text}");
    assert!(!app.term.as_ref().unwrap().typing, "typing stopped");
    daemon.push_terminal_bytes("g11-1", b"back\r\n");
    let text = wait_until(&mut app, |t| t.contains("━━ Terminal of g11-1 · live"));
    assert!(text.contains("│ $ back"), "{text}");
    assert!(
        !app.term.as_ref().unwrap().typing,
        "typing does not come back"
    );
}

#[test]
fn twenty_terminal_opens_leave_no_thread_or_connection_behind() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-1", "claude", AgentState::Unknown));
    daemon.set_instance(instance("g11-2", "claude", AgentState::Failed));
    daemon.set_screen("g11-1", "$ ");
    daemon.set_screen("g11-2", "died\n");
    let (mut app, threads) = lab.app(None);
    wait_until(&mut app, |t| t.contains("general ─"));
    for n in 0..20 {
        open_terminal(&mut app, if n % 2 == 0 { "g11-1" } else { "g11-2" });
        assert!(render(&mut app).contains("━━ Terminal of"));
        assert_eq!(threads.load(Ordering::SeqCst), 2, "events + terminal");
        press(&mut app, &[Left]);
    }
    assert_eq!(threads.load(Ordering::SeqCst), 1, "only the event reader");
    let deadline = Instant::now() + Duration::from_secs(5);
    while daemon.open_connections() != 2 {
        assert!(
            Instant::now() < deadline,
            "{} connections left open (expected events + requests)",
            daemon.open_connections()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(daemon.terminal_subscribers(), 0);
}

#[test]
fn a_stopped_agent_shows_its_last_screen_refuses_i_and_goes_live_when_it_runs() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-2", "claude", AgentState::Failed));
    daemon.set_screen("g11-2", "died\n");
    let (mut app, _) = lab.app_on(lab.socket(), None, Language::ZhTw);
    wait_until(&mut app, |t| t.contains("general ─"));
    open_terminal(&mut app, "g11-2");
    let text = render(&mut app);
    assert!(
        text.contains("g11-2 的終端 · 最後的畫面（已停止）"),
        "{text}"
    );
    press(&mut app, &[ch('i')]);
    let text = render(&mut app);
    assert!(text.contains("這個 agent 已停止，不能輸入"), "{text}");
    assert!(!app.term.as_ref().unwrap().typing);
    // Retried: running again, so the terminal is subscribed again, live.
    daemon.set_screen("g11-2", "started again\n");
    daemon.set_instance(instance("g11-2", "claude", AgentState::Unknown));
    let text = wait_until(&mut app, |t| t.contains("g11-2 的終端 · 即時"));
    assert!(text.contains("│ started again"), "{text}");
    assert_eq!(app.term.as_ref().unwrap().mode, TermMode::Live);
}

#[test]
fn an_empty_screen_is_a_message_and_the_view_stays() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    demo_daemon::seed(&daemon);
    daemon.set_screen("qa-1", "");
    let (mut app, threads) = lab.app(None);
    wait_until(&mut app, |t| t.contains("Needs you · 3"));
    press(
        &mut app,
        &[Down, Down, Down, Down, Right, ch('2'), Down, Down, ch('t')],
    );
    let text = render(&mut app);
    assert!(text.contains("No terminal output for qa-1."), "{text}");
    assert_eq!(first_line(&text), "AgEnD › archfix");
    assert_eq!(threads.load(Ordering::SeqCst), 1, "no terminal reader left");
}

#[test]
fn typing_sends_every_key_but_ctrl_bracket_and_a_disconnect_stops_it() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-1", "claude", AgentState::Unknown));
    daemon.set_screen("g11-1", "$ ");
    let (mut app, _) = lab.app(None);
    wait_until(&mut app, |t| t.contains("general ─"));
    open_terminal(&mut app, "g11-1");
    press(&mut app, &[ch('i')]);
    type_str(&mut app, "hi");
    press(
        &mut app,
        &[
            ch('q'),
            ch('L'),
            ch('h'),
            Esc,
            Left,
            Enter,
            KeyCode::Backspace,
        ],
    );
    ctrl(&mut app, 'c');
    ctrl(&mut app, '5'); // how some terminals report Ctrl-]
    assert!(!app.term.as_ref().unwrap().typing);
    assert!(!app.quit);
    std::thread::sleep(Duration::from_millis(300));
    let typed: Vec<u8> = daemon
        .terminal_inputs()
        .into_iter()
        .flat_map(|(_, b)| b)
        .collect();
    assert_eq!(typed, b"hiqLh\x1b\x1b[D\r\x7f\x03");
    // A disconnect ends typing; after reconnecting it is read-only.
    press(&mut app, &[ch('i')]);
    drop(daemon);
    let text = wait_until(&mut app, |t| t.contains("Daemon disconnected"));
    assert!(app.term.is_none(), "{text}");
    let fresh = lab.daemon();
    fresh.set_instance(instance("g11-1", "claude", AgentState::Unknown));
    fresh.set_screen("g11-1", "$ again");
    let text = wait_until(&mut app, |t| t.contains("Reconnected to the daemon."));
    assert!(text.contains("━━ Terminal of g11-1 · live"), "{text}");
    assert!(text.contains("│ $ again"), "{text}");
    assert!(!app.term.as_ref().unwrap().typing);
    let _ = fresh;
}

#[test]
fn terminal_errors_stay_on_the_terminal_and_do_not_answer_a_retry() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-x", "codex", AgentState::Unknown));
    daemon.set_instance(instance("g11-2", "claude", AgentState::Failed));
    daemon.add_attention(failed_item("g11-2"));
    let (mut app, _) = lab.app(None);
    wait_until(&mut app, |t| t.contains("Needs you · 1"));
    open_terminal(&mut app, "g11-x");
    press(&mut app, &[ch('i'), ch('x')]);
    // The id-less not_supported is in flight on the terminal connection
    // while the retry waits for its reply on the request connection.
    ctrl(&mut app, ']');
    press(&mut app, &[ch('h'), Enter, ch('1')]);
    let text = render(&mut app);
    assert!(text.contains("Sent: Retry g11-2"), "{text}");
    assert!(
        daemon.fleet().attention.is_empty(),
        "the retry reached the daemon"
    );
    assert!(daemon.terminal_inputs().is_empty());
    // Back in the terminal the error is shown there.
    open_terminal(&mut app, "g11-x");
    press(&mut app, &[ch('i'), ch('x')]);
    let text = wait_until(&mut app, |t| t.contains("not_supported"));
    assert!(
        text.contains("not_supported: typing into a codex terminal waits until U17 is verified"),
        "{text}"
    );
    assert!(!app.term.as_ref().unwrap().typing, "back to read-only");
    assert!(text.contains("━━ Terminal of g11-x · live"), "{text}");
}

#[test]
fn an_agent_cannot_type_the_daemon_says_so() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-1", "claude", AgentState::Unknown));
    let (mut app, _) = lab.app(Some("g11-1"));
    wait_until(&mut app, |t| t.contains("general ─"));
    open_terminal(&mut app, "g11-1");
    press(&mut app, &[ch('i'), ch('x')]);
    let text = wait_until(&mut app, |t| t.contains("forbidden"));
    assert!(
        text.contains("forbidden: only the operator can type into an agent's terminal"),
        "{text}"
    );
    assert!(!app.term.as_ref().unwrap().typing);
    assert!(daemon.terminal_inputs().is_empty());
    assert!(matches!(app.connection, Connection::Connected));
}

#[test]
fn a_tall_terminal_shows_its_newest_rows_until_scrolled_up() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-1", "claude", AgentState::Unknown));
    let tall: String = (1..=50).map(|n| format!("line {n}\n")).collect();
    daemon.set_screen("g11-1", &tall);
    let (mut app, _) = lab.app(None);
    wait_until(&mut app, |t| t.contains("general ─"));
    open_terminal(&mut app, "g11-1");
    let text = render(&mut app);
    assert!(
        text.contains("│ line 50") && !text.contains("│ line 1\n"),
        "{text}"
    );
    // The title stays while the lines follow the end.
    assert!(text.contains("━━ Terminal of g11-1 · live"), "{text}");
    press(&mut app, &[KeyCode::Up, KeyCode::Up]);
    let before = render(&mut app);
    daemon.push_terminal_bytes("g11-1", b"line 51\r\n");
    for _ in 0..50 {
        app.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
    let text = render(&mut app);
    let body = |t: &str| t.lines().take(28).collect::<Vec<_>>().join("\n");
    assert_eq!(
        body(&text),
        body(&before),
        "scrolled up: the view stays put"
    );
    assert!(text.contains("↓ 3 more lines below"), "{text}");
    press(&mut app, &[Down, Down, Down]);
    let text = render(&mut app);
    assert!(
        text.contains("│ line 51"),
        "back at the end: follows\n{text}"
    );
}

/// P7/T6: while disconnected, one reconnect attempt every 500 ms, even
/// though the loop ticks every 100 ms.
#[test]
fn reconnect_attempts_are_every_500_ms() {
    let lab = Lab::new();
    let (mut app, _) = lab.app(None);
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(2000) {
        app.tick();
        std::thread::sleep(agend_tui::TICK);
    }
    let Connection::Disconnected { attempts, .. } = app.connection else {
        panic!("{:?}", app.connection);
    };
    assert!((3..=5).contains(&attempts), "{attempts} attempts in 2 s");
    // `r` still tries at once.
    press(&mut app, &[ch('r')]);
    let Connection::Disconnected {
        attempts: after, ..
    } = app.connection
    else {
        panic!();
    };
    assert_eq!(after, attempts + 1);
}

/// P7: after a reconnect, a selection that is gone goes to the first row
/// of that screen, not a neighbour.
#[test]
fn a_selection_gone_after_reconnect_goes_to_the_first_row() {
    use agend_tui::app::Target;
    let lab = Lab::new();
    let seed = |daemon: &FakeDaemon, ids: &[&str]| {
        for (n, id) in ids.iter().enumerate() {
            daemon.set_instance(instance(id, "claude", AgentState::Failed));
            daemon.add_attention(AttentionRequiredData {
                waiting_since_unix_ms: Some(n as u64 + 1),
                ..failed_item(id)
            });
        }
    };
    let daemon = lab.daemon();
    seed(&daemon, &["g-a", "g-b", "g-c", "g-d"]);
    let (mut app, _) = lab.app(None);
    wait_until(&mut app, |t| t.contains("Needs you · 4"));
    press(&mut app, &[Down, Enter]);
    assert_eq!(
        app.view().selected,
        Some(Target::Item("instance-failed:g-b".into()))
    );
    drop(daemon);
    wait_until(&mut app, |t| t.contains("Daemon disconnected"));
    let fresh = lab.daemon();
    seed(&fresh, &["g-a", "g-c", "g-d"]);
    wait_until(&mut app, |t| t.contains("Reconnected to the daemon."));
    assert_eq!(
        app.view().selected,
        Some(Target::Item("instance-failed:g-a".into()))
    );
}

/// Typing stops when the instance turns failed (its terminal is then the
/// last screen), without waiting for a new screen.
#[test]
fn typing_stops_when_the_instance_fails() {
    let lab = Lab::new();
    let daemon = lab.daemon();
    daemon.set_instance(instance("g11-1", "claude", AgentState::Unknown));
    daemon.set_screen("g11-1", "$ ");
    let (mut app, _) = lab.app(None);
    wait_until(&mut app, |t| t.contains("general ─"));
    open_terminal(&mut app, "g11-1");
    press(&mut app, &[ch('i')]);
    assert!(app.term.as_ref().unwrap().typing);
    daemon.set_instance(instance("g11-1", "claude", AgentState::Failed));
    let text = wait_until(&mut app, |t| t.contains("· last screen (stopped)"));
    assert!(!app.term.as_ref().unwrap().typing, "{text}");
    press(&mut app, &[ch('x')]);
    std::thread::sleep(Duration::from_millis(200));
    assert!(daemon.terminal_inputs().is_empty(), "x was not typed");
}
