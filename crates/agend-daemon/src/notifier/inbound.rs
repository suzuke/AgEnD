//! Admit mobile actions only against a confirmed, current notification receipt.
use agend_core::{
    config::TelegramConfig,
    protocol::{
        ask::{AnswerSource, AskEntry, AskReply},
        client::{AnswerAskData, AttentionRequiredData, ClientRequest, ResolveAttentionData},
    },
    telegram::{TelegramDelivery, TelegramInboundStore},
};
use serde_json::{Value, json};

pub struct Admitted {
    pub delivery_id: String,
    pub task_version: Option<(u64, u64)>,
    pub request: ClientRequest,
    pub expected: AttentionRequiredData,
    pub callback_id: Option<String>,
}

/// Callback payloads select an action from the stored snapshot; they never
/// carry an authoritative attention id, arbitrary command or expected head.
pub fn keyboard(row: &TelegramDelivery) -> Option<Value> {
    let item = row.attention.as_ref()?;
    let mut buttons = Vec::new();
    if let Some(ask) = &item.ask {
        if let Some(AskEntry::Question { options, .. } | AskEntry::FollowUp { options, .. }) =
            ask.entries.last()
        {
            for (index, option) in options.iter().enumerate() {
                buttons.push(vec![
                    json!({"text":option,"callback_data":format!("c:{}:{index}",row.id)}),
                ]);
            }
        }
    } else {
        for (index, action) in item.actions.iter().enumerate() {
            buttons.push(vec![
                json!({"text":action.as_str(),"callback_data":format!("a:{}:{index}",row.id)}),
            ]);
        }
    }
    if buttons.is_empty() {
        None
    } else {
        Some(json!({"inline_keyboard":buttons}))
    }
}

pub async fn admit<S: TelegramInboundStore>(
    store: &S,
    config: &TelegramConfig,
    bot: u64,
    update: &Value,
) -> Result<Admitted, String>
where
    S::Error: std::fmt::Display,
{
    let update_id = update["update_id"]
        .as_i64()
        .filter(|id| *id >= 0)
        .ok_or("missing update identity")?;
    let (sender, message, callback, text) = if let Some(query) = update.get("callback_query") {
        (&query["from"], &query["message"], Some(query), None)
    } else {
        let message = update.get("message").ok_or("unsupported update")?;
        (
            &message["from"],
            &message["reply_to_message"],
            None,
            message["text"].as_str(),
        )
    };
    let chat = if callback.is_some() {
        message["chat"]["id"].as_i64()
    } else {
        update["message"]["chat"]["id"].as_i64()
    }
    .ok_or("missing chat identity")?;
    if !config.allows(
        chat,
        sender["id"].as_u64().unwrap_or(0),
        sender["is_bot"].as_bool().unwrap_or(true),
    ) {
        return Err("sender or chat is not allowed".into());
    }
    if message["from"]["id"].as_u64() != Some(bot)
        || message["from"]["is_bot"].as_bool() != Some(true)
    {
        return Err("reply does not name this bot's message".into());
    }
    let message_id = message["message_id"]
        .as_i64()
        .filter(|id| *id > 0)
        .ok_or("missing notification message")?;
    let row = store
        .telegram_message(bot, chat, message_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("notification is no longer current or has no confirmed receipt")?;
    if message["chat"]["id"].as_i64() != Some(chat)
        || message["message_thread_id"].as_i64() != row.destination.topic_id
    {
        return Err("notification destination changed".into());
    }
    if callback.is_none()
        && update["message"]["message_thread_id"].as_i64() != row.destination.topic_id
    {
        return Err("reply topic differs from notification".into());
    }
    if !row.complete() {
        return Err("notification has not been fully delivered".into());
    }
    if callback.is_some() && row.message_ids.last().copied() != Some(message_id) {
        return Err("action button is not on the final notification part".into());
    }
    let expected = row
        .attention
        .ok_or("notification has no actionable snapshot")?;
    let request_id = format!("telegram:{bot}:{update_id}");
    let callback_id = callback
        .map(|q| {
            q["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or("missing callback identity")
        })
        .transpose()?;
    let request = if let Some(query) = callback {
        let data = query["data"]
            .as_str()
            .filter(|s| s.len() <= 64)
            .ok_or("invalid callback data")?;
        let mut fields = data.split(':');
        let kind = fields.next().ok_or("invalid callback")?;
        if fields.next() != Some(row.id.as_str()) {
            return Err("callback belongs to another notification".into());
        }
        let index = fields
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or("invalid action index")?;
        if fields.next().is_some() {
            return Err("invalid callback fields".into());
        }
        match kind {
            "a" if expected.ask.is_none() => {
                let action = *expected.actions.get(index).ok_or("action not offered")?;
                ClientRequest::ResolveAttention {
                    data: ResolveAttentionData {
                        request_id,
                        attention_id: expected
                            .attention_id
                            .clone()
                            .ok_or("missing attention identity")?,
                        action,
                        note: None,
                    },
                }
            }
            "c" => {
                let ask = expected.ask.as_ref().ok_or("not a question")?;
                let options = match ask.entries.last() {
                    Some(
                        AskEntry::Question { options, .. } | AskEntry::FollowUp { options, .. },
                    ) => options,
                    _ => return Err("question is not waiting".into()),
                };
                let option = options.get(index).ok_or("option not offered")?.clone();
                ClientRequest::AnswerAsk {
                    data: AnswerAskData {
                        request_id,
                        ask_id: ask.ask_id.clone(),
                        source: AnswerSource::Telegram,
                        reply: AskReply::Choice { option },
                    },
                }
            }
            _ => return Err("unsupported action".into()),
        }
    } else {
        let text = text
            .filter(|s| !s.trim().is_empty())
            .ok_or("empty answer")?
            .to_owned();
        if let Some(ask) = &expected.ask {
            let reply = AskReply::Text { text };
            if !ask.accepts(&reply) {
                return Err("question is not waiting".into());
            }
            ClientRequest::AnswerAsk {
                data: AnswerAskData {
                    request_id,
                    ask_id: ask.ask_id.clone(),
                    source: AnswerSource::Telegram,
                    reply,
                },
            }
        } else if expected
            .actions
            .contains(&agend_core::protocol::client::AttentionAction::RequestChanges)
        {
            ClientRequest::ResolveAttention {
                data: ResolveAttentionData {
                    request_id,
                    attention_id: expected
                        .attention_id
                        .clone()
                        .ok_or("missing attention identity")?,
                    action: agend_core::protocol::client::AttentionAction::RequestChanges,
                    note: Some(text),
                },
            }
        } else {
            return Err("reply to a question or use its action buttons".into());
        }
    };
    Ok(Admitted {
        delivery_id: row.id,
        task_version: row.task_version,
        request,
        expected,
        callback_id,
    })
}

/// Requesting changes needs an explicit reason before any operator effect.
pub fn needs_reason(action: &Admitted) -> bool {
    matches!(&action.request, ClientRequest::ResolveAttention { data }
        if data.action == agend_core::protocol::client::AttentionAction::RequestChanges && data.note.is_none())
}

/// The serialized pipeline rechecks ordinary task/ask snapshots at execution,
/// not only when the HTTP update first arrives. Provider ids are immutable
/// permission/delivery ids and their existing handlers perform their own CAS.
pub async fn dispatch(ctx: &crate::handlers::Context, admitted: Admitted) -> Result<(), String> {
    use crate::handlers::{self, Outcome};
    use agend_core::protocol::client::ClientResponse;
    let id = admitted
        .expected
        .attention_id
        .as_deref()
        .ok_or("missing attention identity")?;
    let current = ctx
        .fleet
        .attention(id)
        .ok_or("notification has been resolved")?;
    if !agend_core::telegram::same_attention(&current, &admitted.expected) {
        return Err("notification is stale; open the current needs-you item".into());
    }
    if id.starts_with("instance-failed:") {
        let ClientRequest::ResolveAttention { data } = admitted.request else {
            return Err("instance requires an action".into());
        };
        if data.action != agend_core::protocol::client::AttentionAction::Retry {
            return Err("instance requires retry".into());
        }
        let (reply, completion) = tokio::sync::oneshot::channel();
        ctx.supervisor
            .send(crate::supervisor::Event::RetryConfirmed {
                expected: Box::new(admitted.expected),
                reply,
            })
            .map_err(|_| "daemon is stopping; retry was not queued")?;
        return completion
            .await
            .map_err(|_| "daemon stopped before confirming retry")?;
    }
    if id.starts_with(handlers::claude_attention::PREFIX)
        || id.starts_with(handlers::opencode_delivery_attention::PREFIX)
        || id.starts_with(handlers::opencode_attention::PREFIX)
    {
        return match handlers::handle(ctx, None, admitted.request).await {
            Outcome::Reply(ClientResponse::CommandResult { .. }) => Ok(()),
            Outcome::Reply(ClientResponse::Error { data }) => Err(data.message),
            _ => Err("operator action did not return a result".into()),
        };
    }
    ctx.pipeline
        .guarded(admitted.expected, admitted.task_version, admitted.request)
        .await
        .map(|_| ())
        .map_err(|(_, message)| message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SqliteStore;
    use agend_core::{
        config::SecretRef,
        protocol::client::AttentionAction,
        telegram::{TelegramDestination, TelegramStore},
    };
    use agend_testkit::tempdir::TempDir;
    #[tokio::test]
    async fn button_uses_saved_actions_and_rejects_foreign_stale_or_unconfirmed_messages() {
        let dir = TempDir::new("telegram-admit").unwrap();
        let store = SqliteStore::open(dir.path(), 0).unwrap();
        let config = TelegramConfig {
            token: SecretRef::Env("UNUSED".into()),
            chat_id: 42,
            allow_user_ids: vec![7],
            needs_you_topic: None,
            team_topics: Default::default(),
        };
        let destination = TelegramDestination {
            bot_id: 123456789,
            chat_id: 42,
            topic_id: None,
        };
        let item = AttentionRequiredData {
            reason: "Approve head abc".into(),
            task_id: Some("t-1".into()),
            ask: None,
            recap: None,
            attention_id: Some("approval:t-1/approve/1".into()),
            unblocks: None,
            waiting_since_unix_ms: Some(1),
            if_ignored: None,
            actions: vec![AttentionAction::Approve, AttentionAction::RequestChanges],
            instance_id: None,
        };
        let row = store
            .observe_telegram(
                &[super::super::worker::notice(&item).unwrap()],
                &destination,
                1,
            )
            .await
            .unwrap()
            .remove(0);
        // Message fields are the recorded Bot API producer receipt. Callback
        // envelope is a schema-level adversary fixture, not live-callback proof.
        let receipt: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/telegram/message.json"))
                .unwrap();
        let message = receipt["result"].clone();
        let id = message["message_id"].as_i64().unwrap();
        let markup = keyboard(&row).unwrap();
        let update = json!({"update_id":1,"callback_query":{"id":"fixture-query","from":{"id":7,"is_bot":false},"message":message,"data":markup["inline_keyboard"][0][0]["callback_data"]}});
        assert!(admit(&store, &config, 123456789, &update).await.is_err());
        assert!(store.claim_telegram_part(&row.id, 0).await.unwrap());
        assert!(store.confirm_telegram_part(&row.id, 0, id).await.unwrap());
        let admitted = admit(&store, &config, 123456789, &update).await.unwrap();
        assert!(
            matches!(admitted.request,ClientRequest::ResolveAttention{data} if data.action==AttentionAction::Approve)
        );
        for (path, value) in [
            ("/callback_query/from/id", json!(8)),
            ("/callback_query/from/is_bot", json!(true)),
            ("/callback_query/message/chat/id", json!(43)),
            ("/callback_query/message/from/id", json!(123)),
            ("/callback_query/message/message_id", json!(id + 1)),
            ("/callback_query/data", json!(format!("a:{}:2", row.id))),
            ("/callback_query/data", json!("a:foreign:0")),
        ] {
            let mut changed = update.clone();
            *changed.pointer_mut(path).unwrap() = value;
            assert!(
                admit(&store, &config, 123456789, &changed).await.is_err(),
                "{path}"
            );
        }
        let mut changes = update.clone();
        changes["callback_query"]["data"] =
            markup["inline_keyboard"][1][0]["callback_data"].clone();
        assert!(needs_reason(
            &admit(&store, &config, 123456789, &changes).await.unwrap()
        ));
        let reply = json!({"update_id":2,"message":{"from":{"id":7,"is_bot":false},
            "chat":{"id":42},"reply_to_message":update["callback_query"]["message"],
            "text":"Please add the missing check"}});
        let admitted = admit(&store, &config, 123456789, &reply).await.unwrap();
        assert!(!needs_reason(&admitted));
        assert!(
            matches!(admitted.request, ClientRequest::ResolveAttention { data }
            if data.action == AttentionAction::RequestChanges && data.note.as_deref() == Some("Please add the missing check"))
        );
        // A new update id cannot reuse a consumed button, even before polling
        // observes an identical failure recurrence or after a DB restart.
        assert!(
            store
                .claim_telegram_update(123456789, 3, &"a".repeat(64))
                .await
                .unwrap()
        );
        assert!(
            store
                .claim_telegram_action(123456789, 3, &row.id)
                .await
                .unwrap()
        );
        assert!(admit(&store, &config, 123456789, &update).await.is_err());
        drop(store);
        let store = SqliteStore::open(dir.path(), 0).unwrap();
        assert!(
            store
                .claim_telegram_update(123456789, 4, &"b".repeat(64))
                .await
                .unwrap()
        );
        assert!(
            !store
                .claim_telegram_action(123456789, 4, &row.id)
                .await
                .unwrap()
        );
        let unchanged = store
            .observe_telegram(
                &[super::super::worker::notice(&item).unwrap()],
                &destination,
                2,
            )
            .await
            .unwrap();
        assert_eq!(
            unchanged[0].id, row.id,
            "unknown effect must not generate another actionable copy"
        );
        assert!(
            store
                .finish_telegram_update(123456789, 3, "accepted")
                .await
                .unwrap()
        );
        let next = store
            .observe_telegram(
                &[super::super::worker::notice(&item).unwrap()],
                &destination,
                3,
            )
            .await
            .unwrap();
        assert_ne!(
            next[0].id, row.id,
            "identical recurrence gets a fresh delivery"
        );
        store.observe_telegram(&[], &destination, 2).await.unwrap();
        assert!(admit(&store, &config, 123456789, &update).await.is_err());
    }
    #[test]
    fn malformed_or_unordered_batch_never_advances_a_cursor() {
        use super::super::poll::validated_batch;
        assert!(validated_batch(&json!([{"update_id":4},{"update_id":3}]), 0).is_err());
        assert!(validated_batch(&json!([{"update_id":4},{"update_id":4}]), 0).is_err());
        assert!(validated_batch(&json!([{"update_id":4},{}]), 0).is_err());
        assert!(validated_batch(&json!([{"update_id":4}]), 5).is_err());
        assert_eq!(
            validated_batch(&json!([{"update_id":4},{"update_id":8}]), 4)
                .unwrap()
                .len(),
            2
        );
    }
}
