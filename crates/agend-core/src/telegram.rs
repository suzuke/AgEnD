//! Durable Telegram delivery boundary; no network, clock or secret values.
use crate::protocol::client::AttentionRequiredData;
use crate::traits::{Notification, NotificationSeverity};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::future::Future;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramDestination {
    pub bot_id: u64,
    pub chat_id: i64,
    pub topic_id: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramDelivery {
    pub id: String,
    pub destination: TelegramDestination,
    pub notification: Notification,
    pub parts: Vec<String>,
    pub next_part: usize,
    pub in_flight: bool,
    /// Transport finished without a confirmed receipt, or was interrupted by restart.
    #[serde(default)]
    pub outcome_unknown: bool,
    /// Explicit operator disposition; automatic source replacement does not set this.
    #[serde(default)]
    pub abandoned_by_operator: Option<String>,
    pub message_ids: Vec<i64>,
    pub abandoned: bool,
    pub created_at_ms: u64,
    #[serde(default)]
    pub attention: Option<AttentionRequiredData>,
    /// Task CAS version and durable attention-note revision, captured before delivery.
    #[serde(default)]
    pub task_version: Option<(u64, u64)>,
}
impl TelegramDelivery {
    pub fn new(
        id: String,
        destination: TelegramDestination,
        notification: Notification,
        now: u64,
    ) -> Self {
        let parts = render(&notification);
        Self {
            id,
            destination,
            notification,
            parts,
            next_part: 0,
            in_flight: false,
            outcome_unknown: false,
            abandoned_by_operator: None,
            message_ids: Vec::new(),
            abandoned: false,
            created_at_ms: now,
            attention: None,
            task_version: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.next_part == self.parts.len() && !self.in_flight && !self.abandoned
    }
    pub fn valid(&self) -> bool {
        !self.id.is_empty()
            && self.destination.bot_id > 0
            && self.destination.bot_id < (1 << 52)
            && self.destination.chat_id != 0
            && self.destination.chat_id.unsigned_abs() < (1 << 52)
            && self.destination.topic_id.is_none_or(|id| id > 0)
            && self.parts == render(&self.notification)
            && !self.parts.is_empty()
            && self.next_part <= self.parts.len()
            && self.message_ids.len() == self.next_part
            && self.message_ids.iter().all(|id| *id > 0)
            && (!self.outcome_unknown || self.in_flight)
            && (self.abandoned_by_operator.is_none() || (self.abandoned && self.outcome_unknown))
            && (!self.in_flight || self.next_part < self.parts.len())
    }
}

/// Telegram trims outer whitespace. A visible frame preserves the original
/// payload inside it. Chunks count UTF-16 units without splitting UTF-8 scalars.
pub fn render(note: &Notification) -> Vec<String> {
    let severity = match note.severity {
        NotificationSeverity::Info => "Info",
        NotificationSeverity::Attention => "Attention",
        NotificationSeverity::Error => "Error",
    };
    let task = note.task_id.as_deref().unwrap_or("—");
    let payload = format!(
        "[{severity}]\nTitle: {}\nTask: {task}\n\n{}",
        note.title, note.body
    );
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut units = 0;
    for (offset, character) in payload.char_indices() {
        if units + character.len_utf16() > 4000 {
            chunks.push(payload[start..offset].to_string());
            start = offset;
            units = 0;
        }
        units += character.len_utf16();
    }
    chunks.push(payload[start..].to_string());
    let total = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| format!("AgEnD · {}/{total}\n{chunk}\n——", index + 1))
        .collect()
}

/// A current needs-you item. Identity is stable across daemon restarts;
/// content changes or a resolved-and-reopened item start a new delivery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramNotice {
    /// Explicit team topic; None uses the needs-you destination.
    #[serde(default)]
    pub topic_id: Option<i64>,
    pub key: String,
    pub attention: Option<AttentionRequiredData>,
    /// Task CAS version and durable attention-note revision, captured before delivery.
    #[serde(default)]
    pub task_version: Option<(u64, u64)>,
    pub notification: Notification,
}

pub trait TelegramStore: Sync {
    type Error: Send;
    fn observe_telegram<'a>(
        &'a self,
        notices: &'a [TelegramNotice],
        destination: &'a TelegramDestination,
        now: u64,
    ) -> impl Future<Output = Result<Vec<TelegramDelivery>, Self::Error>> + Send + 'a;
    fn enqueue_telegram<'a>(
        &'a self,
        delivery: &'a TelegramDelivery,
    ) -> impl Future<Output = Result<TelegramDelivery, Self::Error>> + Send + 'a;
    /// Unsent auxiliary notifications, excluding reconciled attention/summary rows.
    fn pending_telegram<'a>(
        &'a self,
        destination: &'a TelegramDestination,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<TelegramDelivery>, Self::Error>> + Send + 'a;
    fn telegram_delivery<'a>(
        &'a self,
        id: &'a str,
    ) -> impl Future<Output = Result<Option<TelegramDelivery>, Self::Error>> + Send + 'a;
    fn claim_telegram_part<'a>(
        &'a self,
        id: &'a str,
        part: usize,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send + 'a;
    fn mark_telegram_unknown<'a>(
        &'a self,
        id: &'a str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;
    fn confirm_telegram_part<'a>(
        &'a self,
        id: &'a str,
        part: usize,
        message_id: i64,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send + 'a;
}

/// Durable inbound intent. Claiming a Telegram update precedes every operator
/// action; a missing completion remains unknown and is never dispatched again.
pub trait TelegramInboundStore: Sync {
    type Error: Send;
    fn telegram_offset(&self, bot: u64) -> impl Future<Output = Result<i64, Self::Error>> + Send;
    fn claim_telegram_update<'a>(
        &'a self,
        bot: u64,
        update: i64,
        fingerprint: &'a str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send + 'a;
    /// Consume a notification before its first operator effect, including unknown outcomes.
    fn claim_telegram_action<'a>(
        &'a self,
        bot: u64,
        update: i64,
        delivery: &'a str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send + 'a;
    fn finish_telegram_update<'a>(
        &'a self,
        bot: u64,
        update: i64,
        outcome: &'a str,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send + 'a;
    fn telegram_message(
        &self,
        bot: u64,
        chat: i64,
        message: i64,
    ) -> impl Future<Output = Result<Option<TelegramDelivery>, Self::Error>> + Send;
}

/// Failed-instance timestamps identify durable episodes; other boot-local waits may vary.
pub fn same_attention(a: &AttentionRequiredData, b: &AttentionRequiredData) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    if !a
        .attention_id
        .as_deref()
        .is_some_and(|id| id.starts_with("instance-failed:"))
    {
        a.waiting_since_unix_ms = None;
        b.waiting_since_unix_ms = None;
    }
    a == b
}
