//! Pairing discovery uses the notifier transport, never the CLI or message actions.
//! Callers serialize and persist returned records before advancing the HTTP cursor.
use super::{
    http::{Api, Method},
    poll::validated_batch,
};
use agend_core::{
    config::SecretRef,
    telegram::pairing::{PairingCandidate, TelegramPairing},
    traits::Clock,
};
use serde_json::{Value, json};
use std::sync::Arc;

pub async fn begin(
    api: Arc<Api>,
    id: String,
    token: SecretRef,
    clock: &impl Clock,
) -> Result<TelegramPairing, String> {
    let me = tokio::task::spawn_blocking(move || api.call(Method::GetMe, &json!({})))
        .await
        .map_err(|_| "Telegram identity worker stopped")?
        .map_err(|e| e.to_string())?;
    if me["is_bot"].as_bool() != Some(true) {
        return Err("Telegram identity is not a bot".into());
    }
    TelegramPairing::new(
        id,
        token,
        me["id"].as_u64().ok_or("missing Telegram bot id")?,
        me["username"]
            .as_str()
            .ok_or("missing Telegram bot username")?
            .into(),
        clock,
    )
    .map_err(str::to_owned)
}

pub async fn poll(
    api: Arc<Api>,
    pending: &TelegramPairing,
    clock: &impl Clock,
) -> Result<TelegramPairing, String> {
    pending.check_time(clock)?;
    if pending.candidate.is_some() {
        return Ok(pending.clone());
    }
    verify_bot(api.clone(), pending).await?;
    pending.check_time(clock)?;
    let offset = pending.offset;
    let updates = tokio::task::spawn_blocking(move || {
        api.call(
            Method::GetUpdates,
            &json!({"offset":offset,"limit":100,"timeout":0,"allowed_updates":["message"]}),
        )
    })
    .await
    .map_err(|_| "Telegram pairing worker stopped")?
    .map_err(|e| e.to_string())?;
    pending.check_time(clock)?;
    let batch = validated_batch(&updates, offset)?;
    let mut next = pending.clone();
    for (id, update) in batch {
        if let Some((candidate, text, date)) = message(update) {
            next.observe(candidate, text, date, clock)?;
        }
        next.offset = id.checked_add(1).ok_or("Telegram update id exhausted")?;
    }
    Ok(next)
}

fn message(update: &Value) -> Option<(PairingCandidate, &str, u64)> {
    let m = update.get("message")?;
    if m["from"]["is_bot"].as_bool() != Some(false)
        || [
            "sender_chat",
            "forward_origin",
            "forward_from",
            "forward_from_chat",
            "edit_date",
        ]
        .iter()
        .any(|key| m.get(key).is_some())
        || m["is_automatic_forward"].as_bool().unwrap_or(false)
        || !matches!(
            m["chat"]["type"].as_str(),
            Some("private" | "group" | "supergroup")
        )
        || m["message_id"].as_i64().is_none_or(|id| id <= 0)
    {
        return None;
    }
    let topic_id = match m.get("message_thread_id") {
        Some(v) if m["chat"]["type"] == "supergroup" && m["is_topic_message"] == true => {
            Some(v.as_i64().filter(|id| *id > 0)?)
        }
        Some(_) => return None,
        None if m["is_topic_message"].as_bool().unwrap_or(false) => return None,
        None => None,
    };
    Some((
        PairingCandidate {
            chat_id: m["chat"]["id"].as_i64()?,
            user_id: m["from"]["id"].as_u64()?,
            topic_id,
        },
        m["text"].as_str()?,
        m["date"].as_u64()?,
    ))
}

async fn verify_bot(api: Arc<Api>, pending: &TelegramPairing) -> Result<(), String> {
    let me = tokio::task::spawn_blocking(move || api.call(Method::GetMe, &json!({})))
        .await
        .map_err(|_| "Telegram identity worker stopped")?
        .map_err(|e| e.to_string())?;
    if me["is_bot"].as_bool() != Some(true)
        || me["id"].as_u64() != Some(pending.bot_id)
        || me["username"].as_str() != Some(pending.bot_username.as_str())
    {
        return Err("Telegram pairing bot identity changed; begin again".into());
    }
    Ok(())
}

/// Re-resolve identity before the operator's exact confirmation becomes config.
pub async fn confirm(
    api: Arc<Api>,
    pending: &TelegramPairing,
    expected: &PairingCandidate,
    clock: &impl Clock,
) -> Result<agend_core::config::TelegramConfig, String> {
    pending.confirm(expected, clock)?;
    verify_bot(api, pending).await?;
    pending.confirm(expected, clock).map_err(str::to_owned)
}
