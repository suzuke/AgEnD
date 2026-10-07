//! Owned, zero-model free-text reply lab; network deletion is explicit.
use agend_core::{
    model::Backend,
    protocol::{
        ask::{AnswerSource, AskEntry, AskReply, AskThread},
        client::*,
    },
    runtime_records::{AskRow, Instance, InstanceStatus},
    telegram::{TelegramDelivery, TelegramStore},
};
use agend_daemon::{
    notifier::{
        config::Token,
        http::{Api, Method},
    },
    store::SqliteStore,
};
use serde_json::json;
use std::{io::Write, path::Path};
const ID: &str = "reply-probe";
const ASK: &str = "ask-reply-probe";
const TEXT: &str = "第一行：Telegram 回覆\n第二行：é 🙂";
const MARKER: &str = "telegram-reply-owned";
fn owned(home: &Path) -> Result<(), String> {
    if std::fs::read_to_string(home.join(MARKER)).ok().as_deref() != Some("AgEnD reply probe\n") {
        return Err("not an owned reply lab".into());
    }
    Ok(())
}
fn seed(home: &Path) -> Result<(), String> {
    if home.join("agend.db").exists() {
        return Err("existing database refused".into());
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(home.join(MARKER))
        .and_then(|mut f| f.write_all(b"AgEnD reply probe\n"))
        .map_err(|e| e.to_string())?;
    let store = SqliteStore::open(home, 0).map_err(|e| e.to_string())?;
    agend_testkit::block_on(async {
        store.add_instance(&Instance {
            id: ID.into(), backend: Backend::Claude, program: "/bin/bash".into(),
            args: vec!["-c".into(), "exec /bin/sleep 600".into()],
            working_directory: home.display().to_string(), session_id: None,
            status: InstanceStatus::New, session_started: false, agent_pid: None,
            legacy_no_thread: false, delivery: "inbox".into(),
        }).await?;
        store.save_ask(&AskRow { instance: ID.into(), created: 1, thread: AskThread {
            ask_id: ASK.into(), task_id: None, entries: vec![AskEntry::Question {
                from: ID.into(), text: format!("AgEnD 12D free-text reply test. Reply to this message with exactly:\n{TEXT}"), options: vec![],
            }],
        }}).await
    }).map_err(|e|e.to_string())?;
    println!(
        "{}",
        json!({"seeded":true,"model_processes":0,"expected_text":TEXT})
    );
    Ok(())
}
fn live(home: &Path, mode: &str) -> Result<(), String> {
    owned(home)?;
    let mut client = agend_client::Client::connect_once(&home.join("run/daemon.sock"), None)
        .map_err(|e| e.to_string())?;
    match mode {
        "observe" => {
            let fleet = client.get_fleet().map_err(|e| e.to_string())?;
            println!(
                "{}",
                json!({"open":fleet.attention.iter().any(|a|a.attention_id.as_deref()==Some(ASK)),"instances":fleet.instances.len(),"unexpected_attention":fleet.attention.iter().any(|a|a.attention_id.as_deref()!=Some(ASK))})
            );
        }
        "local-answer" => {
            if home.join("config.toml").exists() {
                return Err("local mode refuses Telegram config".into());
            }
            client
                .answer_ask(ASK, AnswerSource::Cli, AskReply::Text { text: TEXT.into() })
                .map_err(|e| e.to_string())?;
            println!("{}", json!({"answered":true}));
        }
        "remove" => {
            let reply = client
                .request(
                    &ClientRequest::Operator {
                        data: OperatorData {
                            request_id: "remove-owned-reply".into(),
                            command: OperatorCommand::InstanceRemove {
                                instance_id: ID.into(),
                            },
                        },
                    },
                    agend_client::Redo::Never,
                )
                .map_err(|e| e.to_string())?;
            if !matches!(reply,ClientResponse::CommandResult{data} if data.result==CommandResult::Accepted)
            {
                return Err("owned instance removal refused".into());
            }
            println!("{}", json!({"removed":true}));
        }
        _ => return Err("unknown live operation".into()),
    }
    Ok(())
}
fn inspect(home: &Path, delete: bool) -> Result<(), String> {
    owned(home)?;
    let store = SqliteStore::open(home, 0).map_err(|e| e.to_string())?;
    let asks = agend_testkit::block_on(store.asks()).map_err(|e| e.to_string())?;
    let expected =
        asks.len() == 1 && asks[0].thread.ask_id == ASK && asks[0].thread.entries.len() == 2;
    let answer = asks.first().and_then(|a| a.thread.entries.last());
    let exact =
        matches!(answer,Some(AskEntry::Answer{reply:AskReply::Text{text},..}) if text==TEXT);
    if !delete {
        println!(
            "{}",
            json!({"exact_answer":expected&&exact,"answer":answer})
        );
        return Ok(());
    }
    // Read only our private database, holding its exclusive store lock.
    // The runner supplies exact confirmed receipt rows collected after shutdown.
    let rows: Vec<TelegramDelivery> = serde_json::from_slice(
        &std::fs::read(home.join("owned-receipts.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let chat: i64 = std::env::var("TELEGRAM_CHAT_ID")
        .map_err(|_| "chat missing")?
        .parse()
        .map_err(|_| "invalid chat")?;
    if rows.len() > 3
        || rows.iter().any(|r| {
            r.destination.chat_id != chat
                || r.destination.topic_id.is_some()
                || !r.complete()
                || r.message_ids.len() != 1
        })
    {
        return Err("unexpected owned receipts; no deletion".into());
    }
    for row in &rows {
        if agend_testkit::block_on(store.telegram_delivery(&row.id))
            .map_err(|e| e.to_string())?
            .as_ref()
            != Some(row)
        {
            return Err("receipt changed; no deletion".into());
        }
    }
    let api = Api::new(Token::parse(
        std::env::var("TELEGRAM_BOT_TOKEN").map_err(|_| "token missing")?,
    )?);
    let me = api
        .call(Method::GetMe, &json!({}))
        .map_err(|e| e.to_string())?;
    if me["is_bot"] != true
        || rows
            .iter()
            .any(|r| me["id"].as_u64() != Some(r.destination.bot_id))
    {
        return Err("wrong bot; no deletion".into());
    }
    for row in &rows {
        if api
            .call(
                Method::DeleteMessage,
                &json!({"chat_id":chat,"message_id":row.message_ids[0]}),
            )
            .map_err(|e| e.to_string())?
            != true
        {
            return Err("deletion unconfirmed; do not retry".into());
        }
    }
    println!("{}", json!({"deleted":rows.len()}));
    Ok(())
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err(
            "usage: telegram_reply_probe seed|observe|local-answer|remove|inspect|delete HOME"
                .into(),
        );
    }
    let home = Path::new(&args[2]);
    match args[1].as_str() {
        "seed" => seed(home),
        "inspect" => inspect(home, false),
        "holders" => {
            owned(home)?;
            let holders =
                agend_daemon::runtime::files::running_holders(home).map_err(|e| e.to_string())?;
            println!("{}", json!({"empty":holders.is_empty(),"holders":holders}));
            Ok(())
        }
        "delete" => inspect(home, true),
        mode => live(home, mode),
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
