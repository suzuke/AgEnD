//! Daemon-level `config.toml`: Telegram connection settings and
//! references (env var name or file path) to secrets.
//!
//! This is the only human-written config file. The daemon reads it and never
//! writes it back. Instances, teams, repos and workflows live in the DB, not
//! here (D8).
//!
//! Must NOT: read files or env vars itself (callers pass the text in), or hold
//! secret values inline.

use alloc::{collections::BTreeMap, string::String, vec::Vec};
use serde::{Deserialize, Serialize};

/// References only: secret values never belong in configuration or Debug output.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SecretRef {
    Env(String),
    File(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramConfig {
    pub token: SecretRef,
    pub chat_id: i64,
    #[serde(default)]
    pub allow_user_ids: Vec<u64>,
    #[serde(default)]
    pub needs_you_topic: Option<i64>,
    #[serde(default)]
    pub team_topics: BTreeMap<String, i64>,
}

impl TelegramConfig {
    /// Empty allowlists disable inbound control; doctor must explain this.
    pub fn validate(&self) -> Result<(), &'static str> {
        let valid_id = |id: i64| id != 0 && id.unsigned_abs() < (1_u64 << 52);
        if !valid_id(self.chat_id)
            || self
                .allow_user_ids
                .iter()
                .any(|id| *id == 0 || *id >= (1_u64 << 52))
            || self.needs_you_topic.is_some_and(|id| id <= 0)
            || self
                .team_topics
                .iter()
                .any(|(team, id)| team.is_empty() || *id <= 0)
        {
            return Err("invalid Telegram chat, user or topic id");
        }
        match &self.token {
            SecretRef::Env(name) => {
                let mut chars = name.bytes();
                if !chars
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
                    || !chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
                {
                    return Err("Telegram token environment reference is invalid");
                }
            }
            SecretRef::File(path) if !path.starts_with('/') || path.contains('\0') => {
                return Err("Telegram token file must be an absolute path");
            }
            SecretRef::File(_) => {}
        }
        Ok(())
    }

    pub fn allows(&self, chat_id: i64, user_id: u64, is_bot: bool) -> bool {
        !is_bot && chat_id == self.chat_id && self.allow_user_ids.contains(&user_id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// None enables daily public registry discovery; false supports offline installations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_checks: Option<bool>,
    pub telegram: Option<TelegramConfig>,
}
