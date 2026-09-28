//! Gate 11 acceptance demo, fake-daemon half (`cargo xtask accept tui`
//! runs it, then `crates/agend/examples/tui_real.rs` against the real
//! daemon). The TUI reads the testkit fake daemon through `ClientSource`,
//! i.e. `agend-client`, the product's path (gate 11 B P1): the screens in
//! both languages, a scripted key sequence, answering, the disconnected
//! state (A), then `retry`, the live terminal and typing (B). Each section
//! checks what it prints; the first failed check exits non-zero. No real
//! daemon, no agents: the fake runs in this process in its own temp dir.

#[path = "support/demo_daemon.rs"]
mod demo_daemon;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use agend_core::protocol::ask::AskReply;
use agend_core::protocol::client::{
    AgentState, AttentionAction, AttentionRequiredData, ClientRequest, InstanceView,
};
use agend_testkit::fake_daemon::FakeDaemon;
use agend_testkit::tempdir::TempDir;
use agend_tui::i18n::Language;
use agend_tui::source::client::ClientSource;
use agend_tui::{App, render_to_string};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;

type Check = Result<(), String>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!(
                "tui demo: screens, navigation, resolve, disconnect, retry, terminal and input checks passed"
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("tui demo: FAILED: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Check {
    screens()?;
    navigate()?;
    resolve()?;
    disconnect()?;
    retry()?;
    terminal()?;
    input()
}

/// A fake daemon on a fixed socket (so it can be started again there).
struct Lab {
    dir: TempDir,
}

impl Lab {
    fn new() -> Result<Lab, String> {
        TempDir::new("g11-demo")
            .map(|dir| Lab { dir })
            .map_err(|e| format!("temp dir: {e}"))
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    fn daemon(&self) -> Result<FakeDaemon, String> {
        FakeDaemon::start_at(&self.socket()).map_err(|e| format!("fake daemon: {e}"))
    }

    fn app(&self, lang: Language, caller: Option<&str>) -> App {
        let source = ClientSource::new(&self.socket(), caller.map(Into::into));
        App::new(Box::new(source), lang)
    }
}

fn start(lang: Language) -> Result<(Lab, FakeDaemon, App), String> {
    let lab = Lab::new()?;
    let daemon = lab.daemon()?;
    demo_daemon::seed(&daemon);
    let mut app = lab.app(lang, None);
    let needs = lang.fmt(agend_tui::i18n::Text::NeedsYouN, &["3"]);
    wait(&mut app, |t| t.contains(&needs))?;
    Ok((lab, daemon, app))
}

/// Ticks until the screen satisfies `ok` (events arrive on a reader thread).
fn wait(app: &mut App, ok: impl Fn(&str) -> bool) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.tick();
        let text = render_to_string(app, WIDTH, HEIGHT);
        if ok(&text) {
            return Ok(text);
        }
        if Instant::now() > deadline {
            return Err(format!("timed out waiting; screen was:\n{text}"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn keys(app: &mut App, keys: &[KeyCode]) {
    for key in keys {
        app.key(KeyEvent::from(*key));
    }
}

fn ctrl(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

fn check(ok: bool, what: &str) -> Check {
    if ok {
        println!("   ok: {what}");
        Ok(())
    } else {
        Err(what.to_owned())
    }
}

/// The screen with runs of blank lines collapsed, indented for reading.
fn show(text: &str) {
    let mut blank = false;
    for line in text.lines() {
        if line.is_empty() && blank {
            continue;
        }
        blank = line.is_empty();
        println!("   | {line}");
    }
}

fn selected_line(text: &str) -> String {
    text.lines()
        .skip(1)
        .find(|l| l.contains('›'))
        .unwrap_or("")
        .trim_end()
        .to_owned()
}

fn line_with<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines().find(|l| l.contains(needle)).unwrap_or("")
}

fn screens() -> Check {
    println!("== screens (fake daemon through agend-client, {WIDTH}x{HEIGHT})");
    for lang in [Language::En, Language::ZhTw] {
        let (_lab, _daemon, mut app) = start(lang)?;
        let tag = lang.as_str();
        use KeyCode::{Char, Down, Left, Right};
        // (name, keys from the previous view, breadcrumb, a line it must show)
        let views: [(&str, &[KeyCode], &str, &str); 4] = [
            ("home", &[], "AgEnD", "┏ archfix"),
            (
                "team archfix",
                &[Down, Down, Down, Down, Right],
                "AgEnD › archfix",
                "[1 ",
            ),
            (
                "task T-45",
                &[Right],
                "AgEnD › archfix › T-45",
                "Restructure state boundary",
            ),
            (
                "agent dev-2",
                &[Left, Char('2'), Down, Right],
                "AgEnD › archfix › dev-2",
                "team archfix · backend codex",
            ),
        ];
        for (name, path, crumb, expect) in views {
            keys(&mut app, path);
            let text = render_to_string(&mut app, WIDTH, HEIGHT);
            println!("-- {name} ({tag})");
            show(&text);
            let first = text.lines().next();
            check(
                first == Some(crumb),
                &format!("{name} ({tag}): breadcrumb is {crumb:?}"),
            )?;
            check(
                text.contains(expect),
                &format!("{name} ({tag}): shows {expect:?}"),
            )?;
        }
    }
    Ok(())
}

fn navigate() -> Check {
    println!(
        "== navigate (scripted keys; each line: keys -> breadcrumb | selected row or first line)"
    );
    let (_lab, _daemon, mut app) = start(Language::En)?;
    use KeyCode::{Char, Down, Enter, Left, Right};
    let steps: [(&str, Vec<KeyCode>, &str, &str); 11] = [
        ("(start)", vec![], "AgEnD", "▌›! Regression suite"),
        (
            "↓ ↓ ↓ ↓",
            vec![Down, Down, Down, Down],
            "AgEnD",
            "┏›archfix",
        ),
        (
            "→",
            vec![Right],
            "AgEnD › archfix",
            "›! Restructure state boundary",
        ),
        ("2 ↓", vec![Char('2'), Down], "AgEnD › archfix", "dev-2"),
        (
            "Enter",
            vec![Enter],
            "AgEnD › archfix › dev-2",
            "›Current task: T-45",
        ),
        (
            "t",
            vec![Char('t')],
            "AgEnD › archfix › dev-2 › dev-2 Terminal",
            "│ Running regression suite",
        ),
        ("← ← ←", vec![Left, Left, Left], "AgEnD", "┏›archfix"),
        (
            "/ rev Enter",
            vec![Char('/'), Char('r'), Char('e'), Char('v'), Enter],
            "AgEnD › reviewer-1 Terminal",
            "│ Reviewing simulator report",
        ),
        ("←", vec![Left], "AgEnD", "┏›archfix"),
        ("L", vec![Char('L')], "AgEnD", "┏›archfix"),
        ("L", vec![Char('L')], "AgEnD", "┏›archfix"),
    ];
    for (label, path, crumb, row) in steps {
        keys(&mut app, &path);
        let text = render_to_string(&mut app, WIDTH, HEIGHT);
        let first = text.lines().next().unwrap_or("");
        let selected = selected_line(&text);
        let shown = if selected.is_empty() {
            text.lines().nth(3).unwrap_or("").to_owned()
        } else {
            selected
        };
        println!(
            "   {label:<12} -> {first} | {}",
            shown.chars().take(60).collect::<String>()
        );
        check(first == crumb, &format!("{label}: breadcrumb is {crumb:?}"))?;
        check(shown.contains(row), &format!("{label}: shows {row:?}"))?;
        if label == "L" && app.lang == Language::ZhTw {
            check(
                text.contains("需要你 · 3") && text.contains("L English"),
                "L switches to 繁中 in place",
            )?;
        }
    }
    Ok(())
}

fn resolve() -> Check {
    println!("== resolve (read is not resolved; answering removes the item)");
    let (_lab, daemon, mut app) = start(Language::En)?;
    keys(&mut app, &[KeyCode::Enter]);
    let viewed = render_to_string(&mut app, WIDTH, HEIGHT);
    show(&viewed);
    check(
        viewed.contains("Needs you · 3 open"),
        "viewing A-1 keeps 3 open (read, not resolved)",
    )?;
    keys(&mut app, &[KeyCode::Char('h')]);
    let home = render_to_string(&mut app, WIDTH, HEIGHT);
    let row = line_with(&home, "Regression suite");
    println!("   home row after viewing: {row}");
    check(!row.contains("new"), "A-1 lost its new marker (read)")?;
    keys(&mut app, &[KeyCode::Enter, KeyCode::Char('1')]);
    let after = wait(&mut app, |t| t.contains("Needs you · 2 open"))?;
    show(&after);
    check(
        after.contains("Answer sent to A-1: fixed seed 42"),
        "answer sent",
    )?;
    check(!after.contains("Regression suite"), "A-1 left needs-you")?;
    let sent = daemon.requests().into_iter().find_map(|r| match r {
        ClientRequest::AnswerAsk { data } => Some(data),
        _ => None,
    });
    let Some(sent) = sent else {
        return Err("the fake daemon received no answer_ask".into());
    };
    println!(
        "   daemon received: answer_ask ask_id={} source={:?} reply={:?}",
        sent.ask_id, sent.source, sent.reply
    );
    check(
        sent.ask_id == "A-1"
            && sent.reply
                == AskReply::Choice {
                    option: "fixed seed 42".into(),
                },
        "the daemon got answer_ask A-1 = fixed seed 42",
    )
}

fn disconnect() -> Check {
    println!("== disconnect (stop the fake daemon while the TUI is open)");
    let (lab, daemon, mut app) = start(Language::En)?;
    keys(
        &mut app,
        &[
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Down,
            KeyCode::Right,
        ],
    );
    drop(daemon);
    println!("   fake daemon stopped (its socket is closed and removed)");
    let text = wait(&mut app, |t| t.contains("Reconnect attempt"))?;
    show(&text);
    check(
        text.contains("━━ Daemon disconnected"),
        "disconnected state shown, no crash",
    )?;
    check(
        text.contains("the daemon closed the connection"),
        "the reason is shown",
    )?;
    check(
        !text.contains("Restructure"),
        "no stale data while disconnected",
    )?;

    let fresh = lab.daemon()?;
    demo_daemon::seed(&fresh);
    println!("   a new fake daemon started on the same socket; the TUI keeps retrying");
    let text = wait(&mut app, |t| t.contains("Reconnected to the daemon."))?;
    let first = text.lines().next().unwrap_or("");
    println!(
        "   back: {first} | {}",
        text.lines().rev().nth(1).unwrap_or("")
    );
    check(first == "AgEnD › archfix", "reconnected to the same screen")
}

fn instance(id: &str, state: AgentState) -> InstanceView {
    InstanceView {
        instance_id: id.into(),
        team_id: "general".into(),
        backend: "claude".into(),
        state,
        working_directory: None,
    }
}

fn failed_item(id: &str) -> AttentionRequiredData {
    AttentionRequiredData {
        reason: format!("{id} failed: restarted 3 times in 10m and it still died"),
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

fn retry() -> Check {
    println!("== retry (fake daemon holds attention_resolved: the item leaves only on the event)");
    let lab = Lab::new()?;
    let daemon = lab.daemon()?;
    daemon.set_instance(instance("g11-2", AgentState::Failed));
    daemon.add_attention(failed_item("g11-2"));
    daemon.hold_resolved_events(true);
    let mut app = lab.app(Language::ZhTw, None);
    wait(&mut app, |t| t.contains("需要你 · 1"))?;
    keys(&mut app, &[KeyCode::Enter]);
    let open = render_to_string(&mut app, WIDTH, HEIGHT);
    show(&open);
    check(
        open.contains("不處理的話：g11-2 stays stopped") && open.contains("[1] 重試"),
        "expanded: if ignored, and the action row [1] 重試",
    )?;
    keys(&mut app, &[KeyCode::Char('1')]);
    let sent = render_to_string(&mut app, WIDTH, HEIGHT);
    println!("   after 1: {}", line_with(&sent, "已送出"));
    check(
        sent.contains("已送出：重試 g11-2"),
        "message 已送出：重試 g11-2",
    )?;
    std::thread::sleep(Duration::from_millis(300));
    app.tick();
    let held = render_to_string(&mut app, WIDTH, HEIGHT);
    check(
        held.contains("需要你 · 1 項待處理"),
        "accepted, but no attention_resolved yet: the item stays",
    )?;
    daemon.release_resolved_events();
    wait(&mut app, |t| t.contains("需要你 · 0 項待處理"))?;
    println!("   attention_resolved released: 需要你 · 0");
    check(true, "the item left on attention_resolved")
}

fn terminal() -> Check {
    println!(
        "== terminal (live: output marks the screen stale; the holder's screen is fetched again)"
    );
    let lab = Lab::new()?;
    let daemon = lab.daemon()?;
    daemon.set_instance(instance("g11-1", AgentState::Unknown));
    daemon.set_screen("g11-1", "counter=1\n");
    let mut app = lab.app(Language::En, None);
    wait(&mut app, |t| t.contains("general ─"))?;
    for c in "/g11-1".chars() {
        keys(&mut app, &[KeyCode::Char(c)]);
    }
    keys(&mut app, &[KeyCode::Enter]);
    let text = render_to_string(&mut app, WIDTH, HEIGHT);
    println!("   {}", line_with(&text, "Terminal of"));
    check(
        text.contains("━━ Terminal of g11-1 · live"),
        "the title says live",
    )?;
    let started = Instant::now();
    daemon.push_terminal_bytes("g11-1", b"counter=2\r\n");
    wait(&mut app, |t| t.contains("│ counter=2"))?;
    let took = started.elapsed();
    println!(
        "   counter=2 printed -> on screen after {} ms",
        took.as_millis()
    );
    check(
        took < Duration::from_millis(500),
        "drawn within 300 ms plus a round trip",
    )?;
    let before = subscriptions(&daemon);
    let burst = Instant::now();
    for n in 3..=22 {
        daemon.push_terminal_bytes("g11-1", format!("counter={n}\r\n").as_bytes());
        std::thread::sleep(Duration::from_millis(50));
        app.tick();
    }
    daemon.push_terminal_bytes("g11-1", b"$ ");
    let text = wait(&mut app, |t| t.contains("│ $"))?;
    let fetched = subscriptions(&daemon) - before;
    let seconds = burst.elapsed().as_secs_f64();
    println!(
        "   21 chunks in {seconds:.1} s -> {fetched} screen fetches; the last output (a prompt) drawn"
    );
    check(
        fetched as f64 <= seconds * 5.0 + 1.0,
        "at most 5 fetches a second",
    )?;
    check(text.contains("│ counter=22"), "the last counter line drawn")
}

fn subscriptions(daemon: &FakeDaemon) -> usize {
    daemon
        .requests()
        .iter()
        .filter(|r| matches!(r, ClientRequest::SubscribeTerminal { .. }))
        .count()
}

fn input() -> Check {
    println!("== input (i types; every key goes to the agent except Ctrl-])");
    let lab = Lab::new()?;
    let daemon = lab.daemon()?;
    daemon.set_instance(instance("g11-1", AgentState::Unknown));
    daemon.set_screen("g11-1", "$ ");
    let mut app = lab.app(Language::ZhTw, None);
    wait(&mut app, |t| t.contains("general ─"))?;
    for c in "/g11-1".chars() {
        keys(&mut app, &[KeyCode::Char(c)]);
    }
    keys(&mut app, &[KeyCode::Enter, KeyCode::Char('i')]);
    let typing = render_to_string(&mut app, WIDTH, HEIGHT);
    println!("   {}", line_with(&typing, "的終端"));
    check(
        typing.contains("g11-1 的終端 · 輸入中（Ctrl-] 離開）"),
        "i: typing",
    )?;
    for c in "hello".chars() {
        keys(&mut app, &[KeyCode::Char(c)]);
    }
    keys(&mut app, &[KeyCode::Char('q'), KeyCode::Esc, KeyCode::Left]);
    ctrl(&mut app, 'c');
    ctrl(&mut app, ']');
    let typed: Vec<u8> = inputs(&daemon);
    println!("   the daemon got: {:?}", String::from_utf8_lossy(&typed));
    check(
        typed == b"helloq\x1b\x1b[D\x03",
        "hello, q, Esc, ← and Ctrl-C reached the agent as bytes",
    )?;
    let back = render_to_string(&mut app, WIDTH, HEIGHT);
    check(
        back.contains("g11-1 的終端 · 即時") && !app.quit,
        "Ctrl-] left typing; q and Ctrl-C did not quit",
    )?;

    let mut agent = lab.app(Language::ZhTw, Some("g11-1"));
    wait(&mut agent, |t| t.contains("general ─"))?;
    for c in "/g11-1".chars() {
        keys(&mut agent, &[KeyCode::Char(c)]);
    }
    keys(
        &mut agent,
        &[KeyCode::Enter, KeyCode::Char('i'), KeyCode::Char('x')],
    );
    let refused = wait(&mut agent, |t| t.contains("forbidden"))?;
    println!("   as agent g11-1: {}", line_with(&refused, "forbidden"));
    check(
        refused.contains("forbidden: only the operator can type into an agent's terminal")
            && refused.contains("g11-1 的終端 · 即時"),
        "an agent's typing is refused by the daemon and the view is read-only again",
    )?;
    check(
        inputs(&daemon).len() == typed.len(),
        "nothing the agent typed reached the terminal",
    )
}

/// The daemon's recorded input, once what was sent has arrived.
fn inputs(daemon: &FakeDaemon) -> Vec<u8> {
    std::thread::sleep(Duration::from_millis(300));
    daemon
        .terminal_inputs()
        .into_iter()
        .flat_map(|(_, bytes)| bytes)
        .collect()
}
