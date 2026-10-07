//! Bounded live-mobile lab helpers. Seed/observe are local; delete is explicit.
use agend_core::{
    pipeline::{
        state::PipelineState,
        task::{Task, TaskStatus},
        workflow::Workflow,
    },
    telegram::{TelegramDelivery, TelegramStore},
};
use agend_daemon::{
    notifier::{
        config::Token,
        http::{Api, Method},
    },
    store::SqliteStore,
};
use agend_tui::{App, i18n::Language, source::client::ClientSource};
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
const TASK: &str = "t-mobile";
const ITEM: &str = "task-failed:t-mobile";
const MARKER: &str = "telegram-mobile-owned";
fn owned(home: &Path) -> Result<(), String> {
    if !std::fs::read_to_string(home.join(MARKER)).is_ok_and(|s| s == "AgEnD 12D mobile probe\n") {
        return Err("not an owned mobile lab".into());
    }
    Ok(())
}
fn seed(home: &Path) -> Result<(), String> {
    if home.join("agend.db").exists() {
        return Err("refusing existing database".into());
    }
    let mut marker = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(home.join(MARKER))
        .map_err(|_| "cannot create ownership marker")?;
    marker
        .write_all(b"AgEnD 12D mobile probe\n")
        .map_err(|_| "cannot save ownership marker")?;
    let store = SqliteStore::open(home, 0).map_err(|e| e.to_string())?;
    let mut task = Task::new(
        TASK,
        "AgEnD 12D mobile callback test",
        "general",
        "research",
        1,
    );
    task.status = TaskStatus::Failed;
    let state = PipelineState::new(
        TASK,
        agend_daemon::pipeline::validate(Workflow::builtin_research())
            .map_err(|e| e.to_string())?,
    );
    agend_testkit::block_on(store.create_pipeline_task(
        &task,
        &serde_json::to_string(&state.snapshot()).unwrap(),
        1,
    ))
    .map_err(|e| e.to_string())?;
    println!("{}", json!({"seeded":true,"task":TASK,"model_processes":0}));
    Ok(())
}
fn observe(home: &Path) -> Result<(), String> {
    owned(home)?;
    let socket = home.join("run/daemon.sock");
    let mut client =
        agend_client::Client::connect_once(&socket, None).map_err(|e| e.to_string())?;
    let fleet = client.get_fleet().map_err(|e| e.to_string())?;
    let app = App::new(Box::new(ClientSource::new(&socket, None)), Language::En);
    let key = format!("{ITEM}#0");
    println!(
        "{}",
        json!({"open":fleet.attention.iter().any(|i|i.attention_id.as_deref()==Some(ITEM)),"read":fleet.read_keys.contains(&key),"tui_read":app.read.contains(&key),"tasks":fleet.tasks.len()})
    );
    Ok(())
}
fn local_action(home: &Path, read: bool) -> Result<(), String> {
    owned(home)?;
    if home.join("config.toml").exists() {
        return Err("local exercise refuses configured Telegram".into());
    }
    let mut client = agend_client::Client::connect_once(&home.join("run/daemon.sock"), None)
        .map_err(|e| e.to_string())?;
    if read {
        client.mark_attention_read(ITEM, &format!("{ITEM}#0"))
    } else {
        client.resolve_attention(
            ITEM,
            agend_core::protocol::client::AttentionAction::Acknowledge,
        )
    }
    .map_err(|e| e.to_string())?;
    println!(
        "{}",
        json!({"local_action":if read {"read"}else{"acknowledge"}})
    );
    Ok(())
}
fn delete(home: &Path) -> Result<(), String> {
    owned(home)?;
    // Inspect only after daemon exit, then acquire ownership and recheck each row.
    let db = rusqlite::Connection::open_with_flags(
        home.join("agend.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| "cannot read owned receipts")?;
    let mut q = db
        .prepare("SELECT delivery FROM telegram_outbox")
        .map_err(|_| "cannot read owned outbox")?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|_| "cannot read owned rows")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "cannot collect owned rows")?;
    drop(q);
    drop(db);
    let rows = rows
        .into_iter()
        .map(|s| serde_json::from_str::<TelegramDelivery>(&s).map_err(|_| "invalid owned receipt"))
        .collect::<Result<Vec<_>, _>>()?;
    let store = SqliteStore::open(home, 0).map_err(|e| e.to_string())?;
    for row in &rows {
        if agend_testkit::block_on(store.telegram_delivery(&row.id))
            .map_err(|e| e.to_string())?
            .as_ref()
            != Some(row)
        {
            return Err("owned receipt changed; no deletion".into());
        }
    }
    let chat: i64 = std::env::var("TELEGRAM_CHAT_ID")
        .map_err(|_| "chat env missing")?
        .parse()
        .map_err(|_| "invalid chat")?;
    if rows.len() > 1
        || rows.iter().any(|r| {
            r.notification.task_id.as_deref() != Some(TASK)
                || r.destination.chat_id != chat
                || !r.complete()
                || r.message_ids.len() != 1
        })
    {
        return Err(
            "owned receipt is not the single expected complete notification; no deletion".into(),
        );
    }
    if rows.is_empty() {
        println!("{}", json!({"deleted":0}));
        return Ok(());
    }
    let api = Api::new(Token::parse(
        std::env::var("TELEGRAM_BOT_TOKEN").map_err(|_| "token env missing")?,
    )?);
    let me = api
        .call(Method::GetMe, &json!({}))
        .map_err(|e| e.to_string())?;
    if me["id"].as_u64() != Some(rows[0].destination.bot_id) || me["is_bot"].as_bool() != Some(true)
    {
        return Err("wrong bot; no deletion".into());
    }
    let result = api
        .call(
            Method::DeleteMessage,
            &json!({"chat_id":chat,"message_id":rows[0].message_ids[0]}),
        )
        .map_err(|e| e.to_string())?;
    if result.as_bool() != Some(true) {
        return Err("deletion unconfirmed; do not retry automatically".into());
    }
    drop(store);
    println!(
        "{}",
        json!({"deleted":1,"owned_message_id":rows[0].message_ids[0]})
    );
    Ok(())
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: telegram_mobile_probe seed|observe|delete OWNED_HOME".into());
    }
    let home = PathBuf::from(&args[2]);
    match args[1].as_str() {
        "seed" => seed(&home),
        "observe" => observe(&home),
        "delete" => delete(&home),
        "local-read" => local_action(&home, true),
        "local-ack" => local_action(&home, false),
        _ => Err("unknown operation".into()),
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
