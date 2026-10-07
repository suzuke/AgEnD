//! Unknown delivery outcomes survive daemon restarts and pipeline refresh.
//! Only an authenticated operator's explicit Abandon action terminates one.
use super::{Context, error};
use crate::{log::now_unix_ms, store::StoreError};
use agend_core::protocol::client::*;

pub(crate) const PREFIX: &str = "opencode-delivery:";
// Allow bounded HTTP IO and REST reconciliation before asking the operator.
const WRITE_GRACE_MS: u64 = 10_000;

pub(crate) async fn refresh(ctx: &Context, after: i64) -> Result<i64, StoreError> {
    for item in ctx.fleet.view().attention {
        if let Some(id) = item.attention_id
            && let Some(message) = id.strip_prefix(PREFIX)
            && !ctx.store.opencode_outcome_unknown(message).await?
        {
            ctx.fleet.dismiss(&id);
        }
    }
    let Some(before) = now_unix_ms().checked_sub(WRITE_GRACE_MS) else {
        return Ok(0);
    };
    let rows = ctx.store.unknown_opencode_after(after, before).await?;
    let next = rows.last().map_or(0, |m| m.seq);
    for m in rows {
        let attention_id = format!("{PREFIX}{}", m.id);
        ctx.fleet.upsert_attention(AttentionRequiredData {
            reason: format!("Delivery outcome unknown for message {}; content will not be resent automatically. Abandon permanently ends this delivery without confirming receipt.", m.id),
            task_id: m.task_id.clone(),
            ask: None,
            recap: None,
            attention_id: Some(attention_id.clone()),
            unblocks: Some(0),
            waiting_since_unix_ms: m.attempted_at_unix_ms,
            if_ignored: Some("Message and attribution stay retained while waiting for a valid receipt or your explicit abandonment".into()),
            actions: vec![AttentionAction::Abandon],
            instance_id: Some(m.to_instance),
        });
        // A receipt may have committed while publishing the snapshot.
        if !ctx.store.opencode_outcome_unknown(&m.id).await? {
            ctx.fleet.dismiss(&attention_id);
        }
    }
    Ok(next)
}

pub(crate) async fn resolve(ctx: &Context, data: ResolveAttentionData) -> ClientResponse {
    let Some(message) = data.attention_id.strip_prefix(PREFIX) else {
        return error(
            Some(data.request_id),
            error_code::UNKNOWN_ATTENTION,
            "unknown delivery attention",
        );
    };
    if data.action != AttentionAction::Abandon
        || !ctx
            .fleet
            .attention(&data.attention_id)
            .is_some_and(|a| a.actions.contains(&data.action))
    {
        return error(
            Some(data.request_id),
            error_code::UNKNOWN_ATTENTION,
            "abandon is the only available action for this unknown delivery",
        );
    }
    let reason = data
        .note
        .as_deref()
        .unwrap_or("operator explicitly abandoned the unknown delivery through resolve_attention");
    match ctx
        .store
        .abandon_unknown_opencode(message, reason, now_unix_ms())
        .await
    {
        Ok(()) => {
            ctx.fleet.resolve(&data.attention_id, data.action);
            ClientResponse::CommandResult {
                data: ClientCommandResultData {
                    request_id: data.request_id,
                    result: CommandResult::Accepted,
                },
            }
        }
        Err(e) => error(
            Some(data.request_id),
            error_code::INVALID_REQUEST,
            e.to_string(),
        ),
    }
}
