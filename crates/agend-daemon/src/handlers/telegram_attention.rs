//! Local operator disposition of an unknown Telegram notification; never replay.
use super::{Context, error};
use crate::store::StoreError;
use agend_core::{protocol::client::*, telegram::TelegramStore};
pub(crate) const PREFIX: &str = "telegram-delivery:";

pub(crate) async fn refresh(ctx: &Context, after: &str) -> Result<String, StoreError> {
    for item in ctx.fleet.view().attention {
        if let Some(id) = item.attention_id
            && let Some(delivery) = id.strip_prefix(PREFIX)
        {
            let current = ctx.store.telegram_delivery(delivery).await?;
            if current.is_none_or(|r| !r.outcome_unknown || r.abandoned_by_operator.is_some()) {
                ctx.fleet.dismiss(&id);
            }
        }
    }
    let rows = ctx.store.unknown_telegram_after(after).await?;
    let next = rows.last().map_or_else(String::new, |r| r.id.clone());
    for row in rows {
        let id = format!("{PREFIX}{}", row.id);
        ctx.fleet.upsert_attention(AttentionRequiredData {
            reason: format!("Telegram notification {} has no confirmed receipt for part {} of {}. Abandon ends this delivery without confirming receipt or sending again.", row.id, row.next_part + 1, row.parts.len()),
            task_id: row.notification.task_id,
            ask: None, recap: None, attention_id: Some(id.clone()), unblocks: Some(0),
            waiting_since_unix_ms: Some(row.created_at_ms),
            if_ignored: Some("Delivery evidence remains retained; unfinished parts are not sent automatically".into()),
            actions: vec![AttentionAction::Abandon], instance_id: None,
        });
        if ctx
            .store
            .telegram_delivery(&row.id)
            .await?
            .is_none_or(|r| !r.outcome_unknown || r.abandoned_by_operator.is_some())
        {
            ctx.fleet.dismiss(&id);
        }
    }
    Ok(next)
}
pub(crate) async fn resolve(ctx: &Context, data: ResolveAttentionData) -> ClientResponse {
    let Some(id) = data.attention_id.strip_prefix(PREFIX) else {
        return error(
            Some(data.request_id),
            error_code::UNKNOWN_ATTENTION,
            "unknown Telegram delivery",
        );
    };
    if data.action != AttentionAction::Abandon || ctx.fleet.attention(&data.attention_id).is_none()
    {
        return error(
            Some(data.request_id),
            error_code::UNKNOWN_ATTENTION,
            "only abandon is available for this unknown notification",
        );
    }
    let reason = data
        .note
        .as_deref()
        .unwrap_or("operator explicitly abandoned the unknown Telegram notification");
    match ctx.store.abandon_unknown_telegram(id, reason).await {
        Ok(true) => {
            ctx.fleet.resolve(&data.attention_id, data.action);
            ClientResponse::CommandResult {
                data: ClientCommandResultData {
                    request_id: data.request_id,
                    result: CommandResult::Accepted,
                },
            }
        }
        Ok(false) => error(
            Some(data.request_id),
            error_code::UNKNOWN_ATTENTION,
            "notification already changed or was abandoned",
        ),
        Err(e) => error(
            Some(data.request_id),
            error_code::INVALID_REQUEST,
            e.to_string(),
        ),
    }
}
