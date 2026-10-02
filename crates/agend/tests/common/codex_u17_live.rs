//! Real model smoke. No fake backend is accepted as live evidence.
use crate::lab::{Daemon, Lab};
use agend_client::{Client, Redo};
use agend_core::model::{Backend, DeliveryState};
use agend_core::protocol::client::*;
use agend_daemon::driver::codex::{history, launch};
use agend_daemon::runtime::files;
use agend_daemon::store::{Instance, InstanceStatus, SqliteStore};
use agend_testkit::{block_on, fake_agent::codex::Probe};
use agend_tui::{App, i18n::Language, source::client::ClientSource};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

pub const ID: &str = "g11-live";
const QUEUED: &str = "00000000-0000-4000-8000-000000000117";
const IDLE: &str = "00000000-0000-4000-8000-000000000118";
const WITHIN: Duration = Duration::from_secs(180);

fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn wait(
    app: &mut App,
    mut predicate: impl FnMut(&App) -> Result<bool, String>,
) -> Result<(), String> {
    let deadline = Instant::now() + WITHIN;
    loop {
        app.tick();
        if predicate(app)? {
            return Ok(());
        }
        check(Instant::now() < deadline, "U17 App condition timed out")?;
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn ready(app: &App) -> bool {
    app.term
        .as_ref()
        .and_then(|t| t.full.as_ref())
        .is_some_and(|f| f.ready)
}
fn frame(app: &App) -> Option<&agend_core::protocol::terminal::TerminalFrame> {
    app.term
        .as_ref()?
        .full
        .as_ref()?
        .data
        .as_ref()
        .map(|data| &data.frame)
}
fn acquire(app: &mut App) -> Result<(), String> {
    key(app, KeyCode::Char('i'));
    wait(app, |a| Ok(a.term.as_ref().is_some_and(|t| t.typing)))
}
fn turns(observer: &mut Probe, thread: &str) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let page = observer.call(
            "thread/turns/list",
            json!({"threadId":thread, "cursor":cursor, "limit":100}),
        )?;
        result.extend(
            page["data"]
                .as_array()
                .ok_or("missing turns")?
                .iter()
                .cloned(),
        );
        cursor = page["nextCursor"].clone();
        if cursor.is_null() {
            result.reverse();
            return Ok(result);
        }
    }
}
fn inbox(home: &Path) -> Result<Vec<InboxMessage>, String> {
    let mut client = Client::connect_once(&home.join(DAEMON_SOCKET), Some(ID.into()))
        .map_err(|e| e.to_string())?;
    let request = ClientRequest::Command {
        data: ClientCommandData {
            request_id: client.next_request_id(),
            command: AgentCommand::Inbox {
                after_message_id: None,
            },
        },
    };
    match client
        .request(&request, Redo::Never)
        .map_err(|e| e.to_string())?
    {
        ClientResponse::CommandResult { data } => match data.result {
            CommandResult::Messages { data } => Ok(data.messages),
            other => Err(format!("unexpected inbox result {other:?}")),
        },
        other => Err(format!("unexpected inbox response {other:?}")),
    }
}
fn send(home: &Path, id: &str, body: &str) -> Result<(), String> {
    let mut client = Client::connect_once(&home.join(DAEMON_SOCKET), Some(ID.into()))
        .map_err(|e| e.to_string())?;
    let request = ClientRequest::Command {
        data: ClientCommandData {
            request_id: client.next_request_id(),
            command: AgentCommand::Send {
                to: ID.into(),
                message: body.into(),
                level: Some(MessageLevel::Queue),
                message_id: Some(id.into()),
            },
        },
    };
    match client
        .request(&request, Redo::Never)
        .map_err(|e| e.to_string())?
    {
        ClientResponse::CommandResult { data } if data.result == CommandResult::Accepted => Ok(()),
        other => Err(format!("send refused: {other:?}")),
    }
}
fn manual(app: &mut App, observer: &mut Probe, thread: &str, text: &str) -> Result<String, String> {
    app.paste(text);
    key(app, KeyCode::Enter);
    let mut found = None;
    wait(app, |_| {
        found = history::user_items(&turns(observer, thread)?)
            .into_iter()
            .find(|(_, i)| i.text == text)
            .map(|(_, i)| i);
        Ok(found.is_some())
    })?;
    let item = found.ok_or("manual user item missing")?;
    check(
        item.client_id.as_deref() != Some(QUEUED) && item.client_id.as_deref() != Some(IDLE),
        "manual turn used a daemon client id",
    )?;
    println!("manual turn {} observed on thread {thread}", item.turn_id);
    Ok(item.turn_id)
}
fn ended(
    app: &mut App,
    observer: &mut Probe,
    thread: &str,
    turn: &str,
) -> Result<Vec<Value>, String> {
    let mut latest = Vec::new();
    wait(app, |_| {
        latest = turns(observer, thread)?;
        Ok(latest
            .iter()
            .any(|t| t["id"] == turn && t["status"] == "completed"))
    })?;
    Ok(latest)
}

pub fn run(agend: &Path) -> Result<(), String> {
    let codex = std::env::var_os("CODEX_BIN")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("PATH")?
                .to_str()?
                .split(':')
                .map(|p| Path::new(p).join("codex"))
                .find(|p| p.is_file())
        })
        .ok_or("codex not found; set CODEX_BIN")?;
    let version = Command::new(&codex)
        .arg("--version")
        .output()
        .map_err(|e| e.to_string())?;
    check(version.status.success(), "codex --version failed")?;
    let version = String::from_utf8(version.stdout).map_err(|e| e.to_string())?;
    println!("U17 live CLI: {} at {}", version.trim(), codex.display());
    check(
        version.trim().starts_with("codex-cli "),
        "CODEX_BIN is not the real Codex CLI",
    )?;
    let mut lab = Lab::with_prefix(agend, "g11live");
    let home = lab.home(1);
    let workdir = home.join("workspace").join(ID);
    std::fs::create_dir_all(&workdir).map_err(|e| e.to_string())?;
    {
        let store = SqliteStore::open(&home, 0).map_err(|e| e.to_string())?;
        block_on(store.add_instance(&Instance {
            id: ID.into(),
            backend: Backend::Codex,
            program: codex.display().to_string(),
            args: vec![
                "-c".into(),
                "model=\"gpt-6-luna\"".into(),
                "-c".into(),
                "model_reasoning_effort=\"low\"".into(),
            ],
            working_directory: workdir.display().to_string(),
            session_id: None,
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }))
        .map_err(|e| e.to_string())?;
    }
    // This example, not the normal binary, provides the scoped diagnostic entry.
    lab.agend = std::env::current_exe().map_err(|e| e.to_string())?;
    let actual = agend.display().to_string();
    let flags = [
        ("AGEND_REAL_CODEX", "1"),
        ("AGEND_U17_PROBE", "1"),
        ("AGEND_BIN", actual.as_str()),
    ];
    let mut daemon = Daemon::start(&lab, &home, &flags)?;
    daemon.ready()?;
    let go = daemon.expect_within(&format!("{ID}: go (resume "), WITHIN)?;
    let thread = go
        .split("go (resume ")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .ok_or("missing thread id")?
        .to_owned();
    let holder = files::running(&home, ID)
        .map_err(|e| e.to_string())?
        .ok_or("no holder")?;
    let mut observer = Probe::connect(&launch::socket_path(&home, ID))?;
    observer.call(
        "initialize",
        json!({"clientInfo":{"name":"agend-u17-live","version":"1"},
        "capabilities":{"experimentalApi":true}}),
    )?;
    observer.call(
        "thread/resume",
        json!({"threadId":thread,"excludeTurns":true}),
    )?;
    let source = ClientSource::new(&home.join(DAEMON_SOCKET), None);
    let threads = source.threads();
    let mut app = App::new(Box::new(source), Language::En);
    app.resize(100, 30);
    key(&mut app, KeyCode::Char('/'));
    for c in ID.chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    wait(&mut app, |a| {
        Ok(ready(a) && frame(a).is_some_and(|f| f.modes.bracketed_paste))
    })?;
    let generation = frame(&app).ok_or("no frame")?.generation.clone();
    acquire(&mut app)?;
    let secret = format!("G11U17_{}", std::process::id());
    let prompt = format!(
        "Remember the code word {secret}. Do not use tools.\nWrite the numbers from 1 to 300, separated by spaces, with no other text."
    );
    let human = manual(&mut app, &mut observer, &thread, &prompt)?;
    daemon.expect_within(&format!("{ID}: U17 turn {human} busy"), WITHIN)?;
    check(
        inbox(&home)?.is_empty(),
        "manual input created a daemon message",
    )?;
    let queued_body = "Do not use tools. Reply with exactly QUEUED-OK.";
    send(&home, QUEUED, queued_body)?;
    daemon.expect_within(&format!("{QUEUED} (queue) → thread/queue/add"), WITHIN)?;
    daemon.expect_within(&format!("{QUEUED} confirmed (turn "), WITHIN)?;
    let first = history::user_items(&turns(&mut observer, &thread)?);
    let own = first
        .iter()
        .find(|(_, i)| i.client_id.as_deref() == Some(QUEUED))
        .ok_or("queued message lacks its own client id")?
        .1
        .turn_id
        .clone();
    ended(&mut app, &mut observer, &thread, &own)?;
    daemon.expect_within(&format!("{ID}: U17 turn {own} idle"), WITHIN)?;
    check(
        inbox(&home)?.len() == 1,
        "manual input invented an inbox row",
    )?;
    daemon.interrupt()?;
    for line in &daemon.log {
        println!("boot 1: {line}");
    }
    wait(&mut app, |a| Ok(!a.is_connected()))?;
    app.paste("OFFLINE-MUST-NOT-ARRIVE");
    let mut daemon = Daemon::start(&lab, &home, &flags)?;
    daemon.ready()?;
    daemon.expect_within(&format!("{ID}: thread {thread} resumed"), WITHIN)?;
    wait(&mut app, |a| Ok(a.is_connected() && ready(a)))?;
    check(
        files::running(&home, ID).map_err(|e| e.to_string())? == Some(holder),
        "holder changed on daemon restart",
    )?;
    check(
        frame(&app).is_some_and(|f| f.generation == generation),
        "frame generation changed on daemon restart",
    )?;
    check(
        app.term.as_ref().is_some_and(|t| !t.typing),
        "reconnect restored input without i",
    )?;
    acquire(&mut app)?;
    let query = "What code word did I ask you to remember? Reply with it only. Do not use tools.";
    let recall = manual(&mut app, &mut observer, &thread, query)?;
    let completed = ended(&mut app, &mut observer, &thread, &recall)?;
    daemon.expect_within(&format!("{ID}: U17 turn {recall} idle"), WITHIN)?;
    let answer = completed
        .iter()
        .find(|t| t["id"] == recall)
        .and_then(|t| t["items"].as_array())
        .ok_or("no context reply")?
        .iter()
        .filter(|i| i["type"] == "agentMessage")
        .filter_map(|i| i["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    check(
        answer.contains(&secret),
        "Codex context did not survive daemon restart",
    )?;
    check(
        inbox(&home)?.len() == 1,
        "recall turn invented a daemon receipt",
    )?;
    let idle_body = "Do not use tools. Reply with exactly IDLE-OK.";
    send(&home, IDLE, idle_body)?;
    daemon.expect_within(&format!("{IDLE} (queue) → turn/start"), WITHIN)?;
    daemon.expect_within(&format!("{IDLE} confirmed (turn "), WITHIN)?;
    let items = history::user_items(&turns(&mut observer, &thread)?);
    check(
        items.len() == 4,
        "unexpected manual or daemon user item count",
    )?;
    for id in [QUEUED, IDLE] {
        check(
            items
                .iter()
                .filter(|(_, i)| i.client_id.as_deref() == Some(id))
                .count()
                == 1,
            "missing or duplicate identified daemon receipt",
        )?;
    }
    send(&home, QUEUED, queued_body)?;
    check(inbox(&home)?.len() == 2, "replayed id added an inbox row")?;
    drop(app);
    check(threads.load(Ordering::SeqCst) == 0, "App threads leaked")?;
    daemon.interrupt()?;
    {
        let store = SqliteStore::open(&home, 0).map_err(|e| e.to_string())?;
        let rows = block_on(store.messages_to(ID)).map_err(|e| e.to_string())?;
        check(rows.len() == 2, "unexpected durable message count")?;
        for row in rows {
            let turn = items
                .iter()
                .find(|(_, i)| i.client_id.as_deref() == Some(row.id.as_str()))
                .map(|(_, i)| i.turn_id.as_str());
            check(
                row.state == DeliveryState::Confirmed && row.turn_id.as_deref() == turn,
                "durable receipt belongs to the wrong turn",
            )?;
            println!(
                "receipt {} confirmed in turn {}",
                row.id,
                row.turn_id.unwrap_or_default()
            );
        }
    }
    for line in &daemon.log {
        println!("{line}");
    }
    lab.stop_all_holders();
    check(
        lab.running_holders().is_empty(),
        "owned holder cleanup failed",
    )?;
    println!(
        "same thread {thread}; holder {holder}; context preserved; two manual turns and two independent daemon receipts; cleanup complete"
    );
    Ok(())
}
