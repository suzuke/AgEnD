//! The TUI against the real `agend daemon` (gate 11 B): `App` over
//! `ClientSource` (through `agend-client`) on a daemon in a temp home, with
//! a counting agent and one that dies at once. Shared by
//! `tests/tui_daemon.rs` and `examples/tui_real.rs` (`#[path]`), so the demo
//! prints what the test checks. Needs the gate 6 lab module as `crate::lab`
//! and gate 8's client module as `crate::clp`.
//!
//! Safety: homes under `/tmp/g11.t-<pid>-<n>` (the lab's); every daemon is
//! a child of this process (SIGINT / `Child::kill`); holders are stopped
//! with `Shutdown` by the lab. `agend app` runs are waited with a deadline.
//!
//! Must NOT: signal anything else, or run a real claude, codex or opencode.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use agend_core::model::Backend;
use agend_tui::App;
use agend_tui::i18n::Language;
use agend_tui::source::client::ClientSource;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::clp;
use crate::lab::{self, Daemon, Lab};

pub const WIDTH: u16 = 100;
pub const HEIGHT: u16 = 30;

/// A lab whose homes start with `/tmp/g11.` (gate 11 page: no leftovers).
pub fn lab(agend: &Path) -> Lab {
    Lab::with_prefix(agend, "g11.t")
}

fn ensure(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what()) }
}

pub fn app(socket: &Path, caller: Option<&str>, lang: Language) -> App {
    App::new(
        Box::new(ClientSource::new(socket, caller.map(Into::into))),
        lang,
    )
}

pub fn render(app: &mut App) -> String {
    agend_tui::render_to_string(app, WIDTH, HEIGHT)
}

/// Ticks every 10 ms until the screen satisfies `ok`.
pub fn wait(app: &mut App, within: Duration, ok: impl Fn(&str) -> bool) -> Result<String, String> {
    let deadline = Instant::now() + within;
    loop {
        app.tick();
        let text = render(app);
        if ok(&text) {
            return Ok(text);
        }
        if Instant::now() > deadline {
            return Err(format!("timed out after {within:?}; screen:\n{text}"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn keys(app: &mut App, codes: &[KeyCode]) {
    for code in codes {
        app.key(KeyEvent::from(*code));
    }
}

pub fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        keys(app, &[KeyCode::Char(c)]);
    }
}

fn ctrl(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

/// `/` `<id>` Enter: the agent's terminal.
pub fn open_terminal(app: &mut App, id: &str) {
    keys(app, &[KeyCode::Char('/')]);
    type_str(app, id);
    keys(app, &[KeyCode::Enter]);
}

/// The first line with `needle`, without the rule's trailing `━`/`─`.
pub fn line_with(text: &str, needle: &str) -> String {
    text.lines()
        .find(|l| l.contains(needle))
        .unwrap_or("")
        .trim_end_matches([' ', '━', '─'])
        .to_owned()
}

/// The largest `counter=N` on a terminal screen.
pub fn counter(text: &str) -> Option<u64> {
    text.lines()
        .filter_map(|l| l.split("counter=").nth(1))
        .filter_map(|n| n.trim().parse().ok())
        .max()
}

/// The real daemon with `<a>` (counts every second) and `<b>` (dies at
/// once, so it is `failed` after three restarts).
pub struct Fleet {
    pub home: PathBuf,
    pub socket: PathBuf,
    pub a: String,
    pub b: String,
    pub daemon: Daemon,
}

pub fn start(lab: &Lab) -> Result<Fleet, String> {
    let home = lab.home(1);
    let tag = clp::tag();
    let (a, b) = (format!("g11-{tag}a"), format!("g11-{tag}b"));
    clp::add(&home, &a, Backend::Claude, lab::COUNTER)?;
    clp::add(&home, &b, Backend::Claude, lab::DIES_AT_ONCE)?;
    let mut daemon = Daemon::start(lab, &home, &[])?;
    daemon.ready()?;
    Ok(Fleet {
        socket: clp::socket_of(&home),
        home,
        a,
        b,
        daemon,
    })
}

/// Every section, in order; each line to print (headers start with `==`).
pub fn scenario(lab: &Lab) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut fleet = start(lab)?;
    let result = sections(lab, &mut fleet, &mut out);
    let _ = fleet.daemon.interrupt();
    result.map_err(|e| format!("{e}\n(so far:\n{})", out.join("\n")))?;
    out.extend(agend_app(lab, &fleet.home)?);
    Ok(out)
}

fn sections(lab: &Lab, fleet: &mut Fleet, out: &mut Vec<String>) -> Result<(), String> {
    let (a, b) = (fleet.a.clone(), fleet.b.clone());
    let socket = fleet.socket.clone();

    out.push(format!(
        "== real daemon (agend daemon in a temp home; App through agend-client, {WIDTH}x{HEIGHT})"
    ));
    let mut op = app(&socket, None, Language::ZhTw);
    let text = wait(&mut op, Duration::from_secs(10), |t| {
        t.contains("沒有進行中的目標")
    })?;
    out.push(format!("   {}", line_with(&text, "general ─")));
    out.push(format!("   {}", line_with(&text, "沒有進行中的目標")));
    let started = Instant::now();
    let text = wait(&mut op, Duration::from_secs(40), |t| {
        t.contains("需要你 · 1")
    })?;
    let row = line_with(&text, &format!("{b} failed"));
    out.push(format!(
        "   after {:.0} s, no key pressed: {}",
        started.elapsed().as_secs_f64(),
        line_with(&text, "需要你 · 1")
    ));
    out.push(format!("   {row}"));
    ensure(row.contains("新") && row.contains("general"), || {
        format!("{b}'s row: {row}")
    })?;

    out.push("== retry (real daemon: the item leaves on attention_resolved)".into());
    keys(&mut op, &[KeyCode::Enter]);
    let open = render(&mut op);
    for needle in ["不處理的話", "[1] 重試"] {
        let line = line_with(&open, needle);
        ensure(!line.is_empty(), || format!("no {needle:?}:\n{open}"))?;
        out.push(format!("   {line}"));
    }
    keys(&mut op, &[KeyCode::Char('h')]);
    let home = render(&mut op);
    let row = line_with(&home, &format!("{b} failed"));
    ensure(
        !row.contains("新") && home.contains("需要你 · 1"),
        || format!("after viewing: {home}"),
    )?;
    out.push(format!("   viewed, still 需要你 · 1, no 新: {row}"));
    keys(&mut op, &[KeyCode::Enter, KeyCode::Char('1')]);
    let sent = render(&mut op);
    let message = line_with(&sent, "已送出");
    ensure(message.contains(&format!("已送出：重試 {b}")), || {
        format!("after 1:\n{sent}")
    })?;
    out.push(format!("   {message}"));
    wait(&mut op, Duration::from_secs(10), |t| {
        t.contains("需要你 · 0")
    })?;
    out.push("   需要你 · 0 項待處理 (after attention_resolved)".into());
    let log = fleet
        .daemon
        .expect(&format!("{b}: retry requested by the operator"))?;
    out.push(format!("   daemon: {}", lab::untimed(&log)));
    let start = fleet.daemon.expect(&format!("{b}: start --resume"))?;
    out.push(format!("   daemon: {}", lab::untimed(&start)));

    out.push("== terminal (real daemon: live, no key pressed)".into());
    keys(&mut op, &[KeyCode::Char('h')]);
    open_terminal(&mut op, &a);
    let text = render(&mut op);
    let title = line_with(&text, "的終端");
    ensure(title.contains(&format!("{a} 的終端 · 即時")), || {
        text.clone()
    })?;
    out.push(format!("   {title}"));
    let first = counter(&text).ok_or_else(|| format!("no counter:\n{text}"))?;
    let text = wait(&mut op, Duration::from_secs(5), |t| {
        counter(t).is_some_and(|n| n > first)
    })?;
    out.push(format!(
        "   counter={first} -> counter={} without a key",
        counter(&text).unwrap_or(0)
    ));
    keys(&mut op, &[KeyCode::Char('L')]);
    let english = render(&mut op);
    let title = line_with(&english, "Terminal of");
    ensure(title.contains(&format!("Terminal of {a} · live")), || {
        english.clone()
    })?;
    out.push(format!("   L: {title}"));
    keys(&mut op, &[KeyCode::Char('L')]);

    out.push("== input (real daemon: i types, Ctrl-] stops, only the operator)".into());
    keys(&mut op, &[KeyCode::Char('i')]);
    let typing = render(&mut op);
    let title = line_with(&typing, "的終端");
    ensure(title.contains("輸入中（Ctrl-] 離開）"), || {
        typing.clone()
    })?;
    out.push(format!("   i: {title}"));
    type_str(&mut op, "hello");
    let echoed = wait(&mut op, Duration::from_secs(5), |t| t.contains("hello"))?;
    out.push(format!(
        "   typed hello; the PTY echoes: {}",
        line_with(&echoed, "hello")
    ));
    ctrl(&mut op, ']');
    let back = render(&mut op);
    ensure(back.contains(&format!("{a} 的終端 · 即時")), || {
        back.clone()
    })?;
    out.push(format!("   Ctrl-]: {}", line_with(&back, "的終端")));
    keys(&mut op, &[KeyCode::Left]);

    let mut agent = app(&socket, Some(&a), Language::ZhTw);
    open_terminal(&mut agent, &a);
    keys(&mut agent, &[KeyCode::Char('i'), KeyCode::Char('x')]);
    let refused = wait(&mut agent, Duration::from_secs(5), |t| {
        t.contains("forbidden")
    })?;
    std::thread::sleep(Duration::from_millis(1500));
    let later = wait(&mut agent, Duration::from_secs(1), |_| true)?;
    ensure(
        refused.contains("forbidden: only the operator can type into an agent's terminal")
            && !later.contains("xcounter"),
        || format!("as agent {a}:\n{later}"),
    )?;
    out.push(format!(
        "   AGEND_INSTANCE={a}: i, x -> {}",
        line_with(&refused, "forbidden").trim()
    ));
    keys(&mut agent, &[KeyCode::Char('h')]);
    // b dies again after the retry and is back in "needs you".
    wait(&mut agent, Duration::from_secs(40), |t| {
        t.contains("需要你 · 1")
    })?;
    keys(&mut agent, &[KeyCode::Enter, KeyCode::Char('1')]);
    let refused = render(&mut agent);
    ensure(
        refused.contains("forbidden: only the operator can resolve needs-you items")
            && refused.contains("需要你 · 1"),
        || format!("retry as agent:\n{refused}"),
    )?;
    out.push(format!(
        "   AGEND_INSTANCE={a}: retry -> {}",
        line_with(&refused, "forbidden").trim()
    ));
    drop(agent);
    drop(op);

    out.push("== reconnect (real daemon restarted while the terminal is open)".into());
    let mut op = app(&socket, None, Language::En);
    open_terminal(&mut op, &a);
    let before = counter(&render(&mut op)).unwrap_or(0);
    fleet.daemon.interrupt()?;
    let down = wait(&mut op, Duration::from_secs(10), |t| {
        t.contains("Reconnect attempt")
    })?;
    for needle in [
        "━━ Daemon disconnected",
        "Lost the connection",
        "Reconnect attempt",
    ] {
        out.push(format!("   {}", line_with(&down, needle).trim()));
    }
    ensure(
        down.contains("Lost the connection to the daemon: the daemon closed the connection"),
        || down.clone(),
    )?;
    fleet.daemon = Daemon::start(lab, &fleet.home, &[])?;
    fleet.daemon.ready()?;
    let back = wait(&mut op, Duration::from_secs(10), |t| {
        t.contains("Reconnected to the daemon.") && t.contains("· live")
    })?;
    let after = wait(&mut op, Duration::from_secs(5), |t| {
        counter(t).is_some_and(|n| n > before)
    })?;
    out.push(format!(
        "   {} | {}",
        line_with(&back, "Terminal of").trim(),
        line_with(&back, "Reconnected")
    ));
    out.push(format!(
        "   counter={before} before the restart -> counter={} (the same holder)",
        counter(&after).unwrap_or(0)
    ));
    keys(&mut op, &[KeyCode::Left]);
    let home = wait(&mut op, Duration::from_secs(20), |t| {
        t.contains("Needs you · 1")
    })?;
    out.push(format!(
        "   ←: {} (from the new fleet view, not a replay)",
        line_with(&home, &format!("{b} failed")).trim()
    ));
    Ok(())
}

/// `agend app` refuses to start without a home or a terminal (P2).
pub fn agend_app(lab: &Lab, home: &Path) -> Result<Vec<String>, String> {
    let mut out = vec!["== agend app (the binary: exits before the full screen)".into()];
    for (what, home) in [("no AGEND_HOME", None), ("stdout is a pipe", Some(home))] {
        let mut cmd = Command::new(&lab.agend);
        cmd.arg("app")
            .env_remove("AGEND_HOME")
            .env_remove("AGEND_INSTANCE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(home) = home {
            cmd.env("AGEND_HOME", home);
        }
        let run = clp::wait_output(
            cmd.spawn().map_err(|e| e.to_string())?,
            Duration::from_secs(10),
        )?;
        let said = String::from_utf8_lossy(&run.stderr).trim().to_owned();
        ensure(
            run.status.code() == Some(2) && run.stdout.is_empty(),
            || format!("{what}: {:?} {said}", run.status),
        )?;
        out.push(format!("   {what}: exit 2: {said}"));
    }
    Ok(out)
}
