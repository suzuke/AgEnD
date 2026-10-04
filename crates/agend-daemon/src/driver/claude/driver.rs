//! Claims messages once. Channel/Stop helpers are the only content writers.
use crate::driver::codex::DriverError;
use crate::store::{Claim, InstanceStatus, NewMessage, SqliteStore};
use agend_core::{
    model::{Backend, DeliveryState},
    policy::busy::BusyLevel,
    traits::*,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Wait for the channel helper's durable write receipt, never a backend ACK.
const RECEIPT_WITHIN: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct ClaudeDriver {
    store: Arc<SqliteStore>,
}
impl ClaudeDriver {
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self { store }
    }
}
impl Driver for ClaudeDriver {
    type Error = DriverError;
    async fn deliver(
        &self,
        id: &str,
        message: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, Self::Error> {
        let instance = self
            .store
            .instance(id)
            .await?
            .ok_or_else(|| DriverError::UnknownInstance(id.into()))?;
        if instance.backend != Backend::Claude || instance.delivery != "push" {
            return Err(DriverError::InvalidRequest(
                "not a Claude push instance".into(),
            ));
        }
        let claimed = self
            .store
            .claim_message(
                &NewMessage {
                    id: message.id.clone(),
                    from_instance: message.from.clone(),
                    to_instance: id.into(),
                    task_id: message.task_id.clone(),
                    body: message.body.clone(),
                    level: mode,
                },
                crate::log::now_unix_ms(),
            )
            .await?;
        let mut row = match claimed {
            Claim::Different(_) => {
                return Err(DriverError::InvalidRequest(
                    "message id already used for other content".into(),
                ));
            }
            Claim::Inserted(row) | Claim::Existing(row) => row,
        };
        // An instance failure is not an operator decision to abandon a message.
        // Retain unsent messages and unknown attempts until ACK or explicit abandonment.
        if row.state == DeliveryState::Queued
            && row.attempted_at_unix_ms.is_none()
            && instance.status == InstanceStatus::Running
            && let Some(session) = instance.session_id.as_deref()
            && self.store.claude_reported_idle(id, session).await?
        {
            // Only the bridge can reserve content; this loop observes the real
            // Written/ACK transaction. Timeout returns Queued, never fake Sent.
            // A cached idle from a prior daemon boot can at most cause a bounded
            // wait; it cannot start IO or bypass the bridge's fresh-hook gate.
            let until = Instant::now() + RECEIPT_WITHIN;
            while row.state == DeliveryState::Queued && Instant::now() < until {
                if tokio::runtime::Handle::try_current().is_ok() {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                } else {
                    std::thread::sleep(Duration::from_millis(25));
                }
                row = self.store.message(&message.id).await?.ok_or_else(|| {
                    DriverError::Backend("retained Claude message disappeared".into())
                })?;
            }
        }
        Ok(DeliveryReceipt {
            backend_message_id: None,
            state: row.state,
        })
    }
    async fn events(
        &self,
        id: &str,
        cursor: Option<&str>,
    ) -> Result<Vec<DriverEvent>, Self::Error> {
        if self.store.instance(id).await?.is_none() {
            return Err(DriverError::UnknownInstance(id.into()));
        }
        let after = match cursor {
            None => 0,
            Some(c) => c
                .strip_prefix("claude:")
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|n| *n >= 0)
                .ok_or_else(|| DriverError::InvalidRequest("invalid Claude event cursor".into()))?,
        };
        let rows = self.store.claude_adapter_events(id, after).await?;
        let mut events = Vec::new();
        for row in rows {
            if row.event.instance_id != id {
                continue;
            }
            let payload: serde_json::Value = serde_json::from_str(&row.event.payload)
                .map_err(|e| DriverError::Backend(e.to_string()))?;
            let kind = match row.event.kind.as_str() {
                "Stop" => DriverEventKind::TurnCompleted { summary: None },
                "AgendState" => DriverEventKind::BusyChanged {
                    busy: payload["busy"]
                        .as_bool()
                        .ok_or_else(|| DriverError::Backend("invalid stored state event".into()))?,
                },
                "AgendAck" => DriverEventKind::MessageConfirmed {
                    message_id: payload["message_id"]
                        .as_str()
                        .ok_or_else(|| DriverError::Backend("invalid stored ACK event".into()))?
                        .into(),
                },
                _ => continue,
            };
            events.push(DriverEvent {
                cursor: format!("claude:{}", row.seq),
                kind,
            });
        }
        Ok(events)
    }
}
