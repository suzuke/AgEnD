//! Persist a complete notification, then claim and confirm each text part once.
use super::http::{Api, Method};
use agend_core::{
    telegram::{TelegramDelivery, TelegramDestination, TelegramStore},
    traits::{Notification, Notifier},
};
use serde_json::json;
use std::sync::Arc;

pub struct TelegramNotifier<S> {
    api: Arc<Api>,
    store: Arc<S>,
    destination: TelegramDestination,
    serial: tokio::sync::Mutex<()>,
    bot_id: tokio::sync::OnceCell<u64>,
}
impl<S: TelegramStore + Send + Sync + 'static> TelegramNotifier<S>
where
    S::Error: std::fmt::Display,
{
    pub fn new(api: Arc<Api>, store: Arc<S>, destination: TelegramDestination) -> Self {
        Self {
            api,
            store,
            destination,
            serial: tokio::sync::Mutex::new(()),
            bot_id: tokio::sync::OnceCell::new(),
        }
    }
    /// Stable ids let the worker re-observe attention without publishing again.
    pub async fn notify_with_id(
        &self,
        id: &str,
        note: &Notification,
        now: u64,
    ) -> Result<(), String> {
        let _serial = self.serial.lock().await;
        let initial = TelegramDelivery::new(id.into(), self.destination.clone(), note.clone(), now);
        let row = self
            .store
            .enqueue_telegram(&initial)
            .await
            .map_err(|e| e.to_string())?;
        self.send_pending(row, false, None).await
    }
    /// Resume confirmed-prefix deliveries. An in-flight part is unknown, not pending.
    pub async fn resume(&self, id: &str) -> Result<(), String> {
        let _serial = self.serial.lock().await;
        let row = self
            .store
            .telegram_delivery(id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("unknown Telegram delivery")?;
        self.send_pending(row, false, None).await
    }
    pub(super) async fn resume_one(
        &self,
        id: &str,
        stop: &tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), String> {
        let _serial = self.serial.lock().await;
        let row = self
            .store
            .telegram_delivery(id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("unknown Telegram delivery")?;
        self.send_pending(row, true, Some(stop)).await
    }
    async fn send_pending(
        &self,
        row: TelegramDelivery,
        one: bool,
        stop: Option<&tokio::sync::watch::Receiver<bool>>,
    ) -> Result<(), String> {
        let id = row.id.clone();
        let mut attempted = false;
        let result = self
            .send_pending_inner(row, one, stop, &mut attempted)
            .await;
        if result.is_err() && attempted {
            self.store
                .mark_telegram_unknown(&id)
                .await
                .map_err(|e| e.to_string())?;
        }
        result
    }
    async fn send_pending_inner(
        &self,
        mut row: TelegramDelivery,
        one: bool,
        stop: Option<&tokio::sync::watch::Receiver<bool>>,
        attempted: &mut bool,
    ) -> Result<(), String> {
        if row.destination != self.destination {
            return Err("Telegram destination or bot identity changed; no send permitted".into());
        }
        if row.abandoned {
            return Err("Telegram delivery was abandoned".into());
        }
        if row.in_flight {
            return Err("Telegram send outcome unknown; no automatic replay permitted".into());
        }
        if row.complete() || stop.is_some_and(|s| *s.borrow()) {
            return Ok(());
        }
        // Pin this immutable API/token to the intended bot before any mutation.
        let actual = self
            .bot_id
            .get_or_try_init(|| async {
                let api = self.api.clone();
                let me = tokio::task::spawn_blocking(move || api.call(Method::GetMe, &json!({})))
                    .await
                    .map_err(|_| "Telegram identity worker stopped".to_owned())?
                    .map_err(|e| e.to_string())?;
                if me["is_bot"].as_bool() != Some(true) {
                    return Err("Telegram identity is not a bot".to_owned());
                }
                me["id"]
                    .as_u64()
                    .filter(|id| *id > 0 && *id < (1 << 52))
                    .ok_or_else(|| "Telegram identity lacks a valid bot id".to_owned())
            })
            .await?;
        if *actual != row.destination.bot_id {
            return Err("Telegram token belongs to a different bot; nothing sent".into());
        }
        while !row.complete() {
            if stop.is_some_and(|s| *s.borrow()) {
                return Ok(());
            }
            let part = row.next_part;
            if !self
                .store
                .claim_telegram_part(&row.id, part)
                .await
                .map_err(|e| e.to_string())?
            {
                return Err("Telegram delivery claim changed; nothing sent".into());
            }
            *attempted = true;
            let text = row.parts[part].clone();
            let mut request = json!({"chat_id":row.destination.chat_id,"text":text,"link_preview_options":{"is_disabled":true}});
            if let Some(topic) = row.destination.topic_id {
                request["message_thread_id"] = topic.into();
            }
            if part + 1 == row.parts.len()
                && let Some(markup) = super::inbound::keyboard(&row)
            {
                request["reply_markup"] = markup;
            }
            let api = self.api.clone();
            let reply =
                tokio::task::spawn_blocking(move || api.call(Method::SendMessage, &request))
                    .await
                    .map_err(|_| "Telegram send worker stopped; outcome unknown")?
                    .map_err(|e| e.to_string())?;
            let id = reply["message_id"]
                .as_i64()
                .filter(|id| *id > 0)
                .ok_or("Telegram receipt lacks message identity")?;
            if reply["chat"]["id"].as_i64() != Some(row.destination.chat_id)
                || reply["from"]["id"].as_u64() != Some(row.destination.bot_id)
                || reply["from"]["is_bot"].as_bool() != Some(true)
                || reply["text"].as_str() != Some(&text)
                || reply["message_thread_id"].as_i64() != row.destination.topic_id
            {
                return Err(
                    "Telegram receipt differs from intended bot, destination or complete text"
                        .into(),
                );
            }
            if !self
                .store
                .confirm_telegram_part(&row.id, part, id)
                .await
                .map_err(|e| e.to_string())?
            {
                return Err("Telegram receipt could not be committed; outcome unknown".into());
            }
            *attempted = false;
            row.next_part += 1;
            row.message_ids.push(id);
            if one {
                break;
            }
        }
        Ok(())
    }
}
impl<S: TelegramStore + Send + Sync + 'static> Notifier for TelegramNotifier<S>
where
    S::Error: std::fmt::Display,
{
    type Error = String;
    async fn notify(&self, notification: &Notification) -> Result<(), String> {
        let id = crate::store::instances::new_session_id().map_err(|e| e.to_string())?;
        self.notify_with_id(&id, notification, crate::log::now_unix_ms())
            .await
    }
}
