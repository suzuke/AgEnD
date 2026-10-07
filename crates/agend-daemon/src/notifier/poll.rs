//! Bounded Telegram update polling with durable intent before operator effects.
use super::{
    delivery::TelegramNotifier,
    http::{Api, Method},
    inbound,
};
use crate::handlers::Context;
use agend_core::{
    config::TelegramConfig,
    telegram::{TelegramDestination, TelegramInboundStore},
    traits::{Notification, NotificationSeverity},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::watch;

pub async fn once(
    ctx: &Context,
    config: &TelegramConfig,
    api: Arc<Api>,
    destination: &TelegramDestination,
    stop: &watch::Receiver<bool>,
) -> Result<(), String> {
    if *stop.borrow() {
        return Ok(());
    }
    let bot = destination.bot_id;
    let offset = ctx
        .store
        .telegram_offset(bot)
        .await
        .map_err(|e| e.to_string())?;
    let transport = api.clone();
    let updates=tokio::task::spawn_blocking(move ||transport.call(Method::GetUpdates,&json!({"offset":offset,"limit":100,"timeout":0,"allowed_updates":["message","callback_query"]}))).await.map_err(|_|"Telegram poll worker stopped")?.map_err(|e|e.to_string())?;
    let updates = validated_batch(&updates, offset)?;
    for (id, update) in updates {
        if *stop.borrow() {
            break;
        }
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(update).map_err(|_| "invalid update")?)
        );
        if !ctx
            .store
            .claim_telegram_update(bot, id, &hash)
            .await
            .map_err(|e| e.to_string())?
        {
            continue;
        }
        // Only a durably claimed update may pass into the operator handlers.
        let admitted = inbound::admit(ctx.store.as_ref(), config, bot, update).await;
        let callback_id = inbound::callback_feedback(config, bot, update);
        let authorized = admitted.is_ok();
        let needs_reason = admitted.as_ref().is_ok_and(inbound::needs_reason);
        let marking_read = admitted.as_ref().is_ok_and(|action| {
            matches!(
                &action.request,
                agend_core::protocol::client::ClientRequest::MarkAttentionRead { .. }
            )
        });
        let outcome = match admitted {
            Ok(_) if needs_reason => Ok(()),
            Ok(action) if marking_read => inbound::dispatch(ctx, action).await,
            Ok(action) => {
                if ctx
                    .store
                    .claim_telegram_action(bot, id, &action.delivery_id)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    inbound::dispatch(ctx, action).await
                } else {
                    Err("notification was already used or changed".into())
                }
            }
            Err(error) => Err(error),
        };
        let status = match &outcome {
            Ok(()) if needs_reason => "reason_required",
            Ok(()) if marking_read => "read",
            Ok(()) => "accepted",
            Err(_) => "refused",
        };
        if !ctx
            .store
            .finish_telegram_update(bot, id, status)
            .await
            .map_err(|e| e.to_string())?
        {
            return Err("Telegram action result could not be committed; outcome unknown".into());
        }
        if *stop.borrow() {
            break;
        }
        if let Some(query) = callback_id {
            let transport = api.clone();
            let text = if needs_reason {
                "Reply to the notification with the requested changes. Nothing applied yet."
            } else if marking_read && outcome.is_ok() {
                "Marked read; item remains open"
            } else if outcome.is_ok() {
                "Accepted"
            } else {
                "Not applied; open the current needs-you item"
            };
            // Acknowledging a callback is cosmetic and still attempted once.
            let _ = tokio::task::spawn_blocking(move || {
                transport.call(
                    Method::AnswerCallbackQuery,
                    &json!({"callback_query_id":query,"text":text,"show_alert":outcome.is_err()}),
                )
            })
            .await;
        } else if authorized {
            let notifier =
                TelegramNotifier::new(api.clone(), ctx.store.clone(), destination.clone());
            let note = Notification {
                severity: NotificationSeverity::Info,
                title: "Action result".into(),
                body: if outcome.is_ok() {
                    "Accepted".into()
                } else {
                    "Not applied; open the current needs-you item".into()
                },
                task_id: None,
            };
            notifier
                .notify_with_id(
                    &format!("telegram-result:{bot}:{id}"),
                    &note,
                    crate::log::now_unix_ms(),
                )
                .await?;
        }
    }
    Ok(())
}

/// Validate the whole response before advancing any durable cursor. Ordered
/// processing means a crash never acknowledges an unclaimed later update.
pub(super) fn validated_batch(value: &Value, offset: i64) -> Result<Vec<(i64, &Value)>, String> {
    let list = value
        .as_array()
        .filter(|v| v.len() <= 100)
        .ok_or("invalid Telegram update batch")?;
    let mut previous = offset.checked_sub(1).ok_or("invalid Telegram offset")?;
    let mut out = Vec::new();
    for update in list {
        let id = update["update_id"]
            .as_i64()
            .filter(|id| *id >= 0 && *id < i64::MAX && *id > previous)
            .ok_or("unordered or invalid Telegram update identity")?;
        previous = id;
        out.push((id, update));
    }
    Ok(out)
}
