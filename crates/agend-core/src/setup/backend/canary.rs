//! Portable canary evidence and pure admission checks. Passing these checks
//! verifies a retained report's bindings; it does not activate a version.
use super::ImportedBackend;
use crate::protocol::client::{
    MessageDeliveryData, MessageDeliveryState, MessageOutcomeData, MessageOutcomeState,
    OPERATOR_MESSAGE_SENDER, is_uuid_v4,
};
use alloc::{format, string::String, vec::Vec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanaryReport {
    pub format: u32,
    pub run_id: String,
    pub artifact: ImportedBackend,
    pub agend_sha256: String,
    pub os: String,
    pub arch: String,
    pub started_at_unix_ms: u64,
    pub elapsed_ms: u64,
    pub observed_version: Option<String>,
    pub receipts: Vec<MessageDeliveryData>,
    #[serde(default)]
    pub outcomes: Vec<MessageOutcomeData>,
    pub passed: bool,
    pub cleanup_complete: bool,
    pub error: Option<String>,
    pub retained_home: Option<String>,
}

impl CanaryReport {
    pub fn validate(
        &self,
        artifact: &ImportedBackend,
        agend_sha256: &str,
        os: &str,
        arch: &str,
    ) -> Result<(), &'static str> {
        if self.format != 1 || !is_uuid_v4(&self.run_id) {
            return Err("invalid canary report identity");
        }
        if !self.passed
            || !self.cleanup_complete
            || self.error.is_some()
            || self.retained_home.is_some()
        {
            return Err("canary did not complete with successful cleanup");
        }
        if &self.artifact != artifact
            || artifact.format != 1
            || !super::valid_version(&artifact.version)
        {
            return Err("canary artifact binding changed");
        }
        if self.agend_sha256 != agend_sha256
            || agend_sha256.len() != 64
            || !agend_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || self.os != os
            || self.arch != arch
        {
            return Err("canary daemon binary or platform binding changed");
        }
        let expected = match artifact.backend.as_str() {
            "codex" => format!("codex-cli {}", artifact.version),
            "claude" => format!("{} (Claude Code)", artifact.version),
            "opencode" => artifact.version.clone(),
            _ => return Err("unknown canary backend"),
        };
        if self.observed_version.as_deref() != Some(expected.as_str()) {
            return Err("canary observed version does not match its artifact");
        }
        let end = self
            .started_at_unix_ms
            .checked_add(self.elapsed_ms)
            .ok_or("canary time range overflow")?;
        if self.started_at_unix_ms == 0
            || self.elapsed_ms == 0
            || self.receipts.len() != 3
            || self.outcomes.len() != 3
        {
            return Err("canary requires three timed delivery receipts");
        }
        let mut previous = self.started_at_unix_ms;
        for (index, receipt) in self.receipts.iter().enumerate() {
            let attempted = receipt
                .attempted_at_unix_ms
                .ok_or("canary receipt was not attempted")?;
            if !is_uuid_v4(&receipt.message_id)
                || self.receipts[..index]
                    .iter()
                    .any(|r| r.message_id == receipt.message_id)
                || receipt.from_instance != OPERATOR_MESSAGE_SENDER
                || receipt.to_instance != "canary"
                || receipt.state != MessageDeliveryState::Confirmed
                || attempted < previous
                || receipt.updated_at_unix_ms < attempted
                || receipt.updated_at_unix_ms > end
            {
                return Err("canary delivery identity, state or timing mismatch");
            }
            let outcome = &self.outcomes[index];
            if outcome.message_id != receipt.message_id
                || outcome.instance_id != receipt.to_instance
                || outcome.execution_id.as_ref().is_none_or(|s| s.is_empty())
                || outcome.turn_id != receipt.turn_id
                || self.outcomes[..index]
                    .iter()
                    .any(|o| o.execution_id == outcome.execution_id)
                || outcome.state != MessageOutcomeState::Completed
            {
                return Err("canary lacks a successful correlated response");
            }
            if self.artifact.backend == "claude" {
                if receipt.turn_id.is_some()
                    || !outcome.execution_id.as_deref().is_some_and(is_uuid_v4)
                {
                    return Err("invalid Claude prompt evidence");
                }
            } else if outcome.execution_id != receipt.turn_id {
                return Err("backend execution reference mismatch");
            }
            previous = receipt.updated_at_unix_ms;
        }
        Ok(())
    }
}
