//! Durable Claude push delivery records. Times are supplied by the caller.
use super::Message;
use crate::model::DeliveryState;
use alloc::string::String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeRoute {
    Channel,
    Stop,
}
impl ClaudeRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::Stop => "stop",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Channel, Self::Stop]
            .into_iter()
            .find(|v| v.as_str() == text)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewClaudeDelivery {
    pub message_id: String,
    pub delivery_id: String,
    pub instance_id: String,
    pub session_id: String,
    pub route: ClaudeRoute,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeAttempt {
    pub delivery_id: String,
    pub session_id: String,
    pub route: ClaudeRoute,
    pub started_at_unix_ms: u64,
    /// When a successful write or a valid ACK was observed, not necessarily
    /// the physical write time if its result was lost.
    pub sent_at_unix_ms: Option<u64>,
    pub confirmed_at_unix_ms: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeDelivery {
    pub message_id: String,
    pub instance_id: String,
    pub attempt: Option<ClaudeAttempt>,
    pub abandoned_at_unix_ms: Option<u64>,
    pub abandonment_reason: Option<String>,
}
impl ClaudeDelivery {
    pub fn may_start(&self, message: &Message) -> bool {
        self.message_id == message.id
            && self.instance_id == message.to_instance
            && message.state == DeliveryState::Queued
            && message.attempted_at_unix_ms.is_none()
            && self.attempt.is_none()
            && self.abandoned_at_unix_ms.is_none()
    }
    pub fn outcome_unknown(&self, message: &Message) -> bool {
        self.message_id == message.id
            && self.instance_id == message.to_instance
            && message.state == DeliveryState::Queued
            && self.abandoned_at_unix_ms.is_none()
            && (self
                .attempt
                .as_ref()
                .is_some_and(|a| a.sent_at_unix_ms.is_none() && a.confirmed_at_unix_ms.is_none())
                || (self.attempt.is_none() && message.attempted_at_unix_ms.is_some()))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeReservation {
    /// Only this result authorizes the caller to start writing the content.
    Started(ClaudeDelivery),
    /// A previous reservation; never authorizes another write.
    Existing(ClaudeDelivery),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeAck {
    pub message_id: String,
    pub delivery_id: String,
    pub instance_id: String,
    pub session_id: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeAckResult {
    Confirmed,
    AlreadyConfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDriverEvent {
    pub id: String,
    pub instance_id: String,
    pub session_id: String,
    pub kind: String,
    /// Native JSON object, bounded by the store before insertion.
    pub payload: String,
    pub occurred_at_unix_ms: u64,
    pub replayed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverEvent {
    pub seq: i64,
    pub event: NewDriverEvent,
    pub ingested_at_unix_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverEventAppend {
    pub event: DriverEvent,
    pub inserted: bool,
}
