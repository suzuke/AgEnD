//! Gate 11 acceptance demo (`cargo xtask accept tui`): the screen layer
//! against the testkit fake daemon over its real socket (client protocol
//! v1). Prints the screens in both languages, a scripted key sequence,
//! resolving a needs-you item, and the disconnected state, checking each;
//! exits non-zero on the first failed check. No real daemon, no agents: the
//! fake daemon runs in this process in its own temp dir.

#[path = "support/daemon_source.rs"]
mod daemon_source;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use agend_core::protocol::ask::AskReply;
use agend_core::protocol::client::ClientRequest;
use agend_testkit::fake_daemon::FakeDaemon;
use agend_tui::i18n::Language;
use agend_tui::source::scripted::demo_catalog;
use agend_tui::{App, render_to_string};
use daemon_source::{Address, DaemonSource, seed_demo};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;

type Check = Result<(), String>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("tui demo: screens, navigation, resolve and disconnect checks passed");
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
    disconnect()
}

fn start(lang: Language) -> Result<(FakeDaemon, App, Address), String> {
    let daemon = FakeDaemon::start().map_err(|e| format!("fake daemon: {e}"))?;
    seed_demo(&daemon);
    let (source, address) = DaemonSource::new(daemon.socket_path().to_path_buf(), demo_catalog());
    let mut app = App::new(Box::new(source), lang);
    let needs = lang.fmt(agend_tui::i18n::Text::NeedsYouN, &["3"]);
    wait(&mut app, |t| t.contains(&needs))?;
    Ok((daemon, app, address))
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

fn screens() -> Check {
    println!("== screens (fake daemon over its socket, {WIDTH}x{HEIGHT})");
    for lang in [Language::En, Language::ZhTw] {
        let (_daemon, mut app, _) = start(lang)?;
        let tag = lang.as_str();
        use KeyCode::{Char, Down, Left, Right};
        // (name, keys from the previous view, breadcrumb, a line it must show)
        let views: [(&str, &[KeyCode], &str, &str); 4] = [
            ("home", &[], "AgEnD", "┏ archfix"),
            (
                "team archfix",
                &[Down, Down, Down, Right],
                "AgEnD › archfix",
                "[1 ",
            ),
            (
                "task T-45",
                &[Right],
                "AgEnD › archfix › T-45",
                "repo agend-terminal · T-45",
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
    let (_daemon, mut app, _) = start(Language::En)?;
    use KeyCode::{Char, Down, Enter, Left, Right};
    let steps: [(&str, Vec<KeyCode>, &str, &str); 11] = [
        ("(start)", vec![], "AgEnD", "▌›! Regression suite"),
        ("↓ ↓ ↓", vec![Down, Down, Down], "AgEnD", "┏›archfix"),
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
            "│ fake screen of dev-2",
        ),
        ("← ← ←", vec![Left, Left, Left], "AgEnD", "┏›archfix"),
        (
            "/ rev Enter",
            vec![Char('/'), Char('r'), Char('e'), Char('v'), Enter],
            "AgEnD › reviewer-1 Terminal",
            "│ fake screen of reviewer-1",
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
    let (daemon, mut app, _) = start(Language::En)?;
    keys(&mut app, &[KeyCode::Enter]);
    let viewed = render_to_string(&mut app, WIDTH, HEIGHT);
    show(&viewed);
    check(
        viewed.contains("Needs you · 3 open"),
        "viewing A-1 keeps 3 open (read, not resolved)",
    )?;
    keys(&mut app, &[KeyCode::Char('h')]);
    let home = render_to_string(&mut app, WIDTH, HEIGHT);
    let row = home
        .lines()
        .find(|l| l.contains("Regression suite"))
        .unwrap_or("");
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
    let (daemon, mut app, address) = start(Language::En)?;
    keys(
        &mut app,
        &[KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Right],
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

    let fresh = FakeDaemon::start().map_err(|e| format!("fake daemon: {e}"))?;
    seed_demo(&fresh);
    *address.lock().unwrap_or_else(|e| e.into_inner()) = fresh.socket_path().to_path_buf();
    println!("   a new fake daemon started; the TUI keeps retrying");
    let text = wait(&mut app, |t| t.contains("Reconnected to the daemon."))?;
    let first = text.lines().next().unwrap_or("");
    println!(
        "   back: {first} | {}",
        text.lines().rev().nth(1).unwrap_or("")
    );
    check(first == "AgEnD › archfix", "reconnected to the same screen")
}
