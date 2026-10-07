//! Authorized, bounded text-fidelity probe: one send and one owned deletion.
use agend_daemon::notifier::{
    config::Token,
    http::{Api, Method},
};
use serde_json::json;
fn run() -> Result<(), String> {
    let token =
        Token::parse(std::env::var("TELEGRAM_BOT_TOKEN").map_err(|_| "token env missing")?)?;
    let chat: i64 = std::env::var("TELEGRAM_CHAT_ID")
        .map_err(|_| "chat env missing")?
        .parse()
        .map_err(|_| "invalid chat")?;
    let path = std::env::args()
        .nth(1)
        .ok_or("supply evidence output path")?;
    let api = Api::new(token);
    let raw = "\n  AgEnD 12D 測試：空白與繁中 é ✅\n最後一行  \n\n";
    let text = if std::env::args().nth(2).as_deref() == Some("framed") {
        format!("AgEnD\n{raw}\n——")
    } else {
        raw.to_owned()
    };
    let result = api
        .call(
            Method::SendMessage,
            &json!({"chat_id":chat,"text":text,"disable_notification":true}),
        )
        .map_err(|e| e.to_string())?;
    let id = result["message_id"]
        .as_i64()
        .ok_or("missing message id; inspect test chat")?;
    if result["chat"]["id"].as_i64() != Some(chat) {
        return Err("unexpected response chat; no deletion issued".into());
    }
    // Capture only our own authored text; no account/profile data is retained.
    let received = result["text"].as_str();
    let mut receipt = result.clone();
    for key in ["chat", "from"] {
        if let Some(object) = receipt[key].as_object_mut() {
            for name in ["first_name", "last_name", "username", "title"] {
                if object.contains_key(name) {
                    object.insert(name.into(), "Fixture".into());
                }
            }
            object.insert(
                "id".into(),
                if key == "chat" {
                    42.into()
                } else {
                    123456789.into()
                },
            );
        }
    }
    let deleted = api
        .call(
            Method::DeleteMessage,
            &json!({"chat_id":chat,"message_id":id}),
        )
        .map_err(|e| e.to_string());
    let evidence = json!({"sanitized_receipt":receipt,"method":"sendMessage","sent_text":text,"returned_text":received,"exact_match":received==Some(text.as_str()),"messages_sent":1,"owned_message_id":id,"delete_confirmed":deleted.as_ref().ok().and_then(|v|v.as_bool())==Some(true),"delete_error":deleted.as_ref().err()});
    std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap())
        .map_err(|_| "cannot write evidence")?;
    println!(
        "text_exact_match={} owned_message_deleted={}",
        evidence["exact_match"], evidence["delete_confirmed"]
    );
    deleted.map(|_| ())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
