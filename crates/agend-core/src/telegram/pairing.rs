//! Expiring pairing proof; observation never enables Telegram control.
use crate::{
    config::{SecretRef, TelegramConfig},
    traits::Clock,
};
use alloc::{collections::BTreeMap, format, string::String, vec};
use serde::{Deserialize, Serialize};

pub const PAIRING_WINDOW_MS: u64 = 600_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingCandidate {
    pub chat_id: i64,
    pub user_id: u64,
    pub topic_id: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramPairing {
    pub id: String,
    pub token: SecretRef,
    pub bot_id: u64,
    pub bot_username: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub offset: i64,
    pub candidate: Option<PairingCandidate>,
}
impl TelegramPairing {
    pub fn new(
        id: String,
        token: SecretRef,
        bot_id: u64,
        bot_username: String,
        clock: &impl Clock,
    ) -> Result<Self, &'static str> {
        if !crate::protocol::client::is_uuid_v4(&id)
            || bot_id == 0
            || bot_id >= (1 << 52)
            || !(5..=32).contains(&bot_username.len())
            || !bot_username
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err("invalid Telegram pairing identity");
        }
        let now = clock.now_unix_ms();
        let expires = now
            .checked_add(PAIRING_WINDOW_MS)
            .ok_or("pairing clock exhausted")?;
        let value = Self {
            id,
            token,
            bot_id,
            bot_username,
            created_at_ms: now,
            expires_at_ms: expires,
            offset: 0,
            candidate: None,
        };
        value
            .config_for(&PairingCandidate {
                chat_id: 1,
                user_id: 1,
                topic_id: None,
            })?
            .validate()?;
        Ok(value)
    }
    pub fn command(&self) -> String {
        format!("/start agend_{}", self.id)
    }
    pub fn check_time(&self, clock: &impl Clock) -> Result<(), &'static str> {
        let now = clock.now_unix_ms();
        if now < self.created_at_ms || now >= self.expires_at_ms {
            return Err("Telegram pairing expired or clock moved backwards; begin again");
        }
        Ok(())
    }
    /// A caller must first validate that this is a direct, unedited human message.
    pub fn observe(
        &mut self,
        candidate: PairingCandidate,
        text: &str,
        date_seconds: u64,
        clock: &impl Clock,
    ) -> Result<bool, &'static str> {
        self.check_time(clock)?;
        let payload = format!("agend_{}", self.id);
        let Some((command, value)) = text.split_once(' ') else {
            return Ok(false);
        };
        let tagged = format!("/start@{}", self.bot_username);
        if value != payload || !(command == "/start" || command.eq_ignore_ascii_case(&tagged)) {
            return Ok(false);
        }
        if date_seconds < self.created_at_ms / 1000 || date_seconds > clock.now_unix_ms() / 1000 {
            return Ok(false);
        }
        self.config_for(&candidate)?.validate()?;
        if self.candidate.as_ref().is_some_and(|old| old != &candidate) {
            return Err("multiple Telegram pairing destinations; begin again");
        }
        self.candidate = Some(candidate);
        Ok(true)
    }
    /// The operator must echo the exact observed destination; never auto-confirm.
    pub fn confirm(
        &self,
        expected: &PairingCandidate,
        clock: &impl Clock,
    ) -> Result<TelegramConfig, &'static str> {
        self.check_time(clock)?;
        if self.candidate.as_ref() != Some(expected) {
            return Err("Telegram pairing destination changed or is missing");
        }
        self.config_for(expected)
    }
    fn config_for(&self, candidate: &PairingCandidate) -> Result<TelegramConfig, &'static str> {
        let config = TelegramConfig {
            token: self.token.clone(),
            chat_id: candidate.chat_id,
            allow_user_ids: vec![candidate.user_id],
            needs_you_topic: candidate.topic_id,
            team_topics: BTreeMap::new(),
        };
        config.validate()?;
        Ok(config)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingPhase {
    Pending,
    Confirmed,
    Cancelled,
}

/// One durable setup operation. Confirmation is distinct from applying config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingRecord {
    pub session: TelegramPairing,
    pub phase: PairingPhase,
}

/// Operator-only setup actions. No inline token or automatic confirmation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum PairingOperation {
    Status,
    Begin {
        id: String,
        token: SecretRef,
        previous: Option<String>,
    },
    Poll {
        id: String,
    },
    Confirm {
        id: String,
        candidate: PairingCandidate,
    },
    Cancel {
        id: String,
    },
}
