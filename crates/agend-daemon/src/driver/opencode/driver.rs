//! Durable claims are cheap; only the supervisor's instance worker writes to
//! the backend. Reading a receipt never grants another send attempt.
use crate::driver::codex::DriverError;
use crate::store::{Claim, NewMessage, SqliteStore};
use agend_core::{model::Backend, policy::busy::BusyLevel, traits::*};
use std::sync::Arc;

#[derive(Clone)]
pub struct OpenCodeDriver {
    store: Arc<SqliteStore>,
}
impl OpenCodeDriver {
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self { store }
    }
}
impl Driver for OpenCodeDriver {
    type Error = DriverError;
    async fn deliver(
        &self,
        id: &str,
        message: &AgentMessage,
        level: BusyLevel,
    ) -> Result<DeliveryReceipt, DriverError> {
        let instance = self
            .store
            .instance(id)
            .await?
            .ok_or_else(|| DriverError::UnknownInstance(id.into()))?;
        if instance.backend != Backend::Opencode || instance.delivery != "push" {
            return Err(DriverError::InvalidRequest(
                "not an OpenCode push instance".into(),
            ));
        }
        let mut row = match self
            .store
            .claim_message(
                &NewMessage {
                    id: message.id.clone(),
                    from_instance: message.from.clone(),
                    to_instance: id.into(),
                    task_id: message.task_id.clone(),
                    body: message.body.clone(),
                    level,
                },
                crate::log::now_unix_ms(),
            )
            .await?
        {
            Claim::Different(_) => {
                return Err(DriverError::InvalidRequest(
                    "message id already used for other content".into(),
                ));
            }
            Claim::Inserted(row) | Claim::Existing(row) => row,
        };
        if row.state == agend_core::model::DeliveryState::Queued
            && row.attempted_at_unix_ms.is_none()
            && instance.status == crate::store::InstanceStatus::Running
            && instance.session_id.is_some()
        {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while row.state == agend_core::model::DeliveryState::Queued
                && std::time::Instant::now() < until
            {
                if tokio::runtime::Handle::try_current().is_ok() {
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                row =
                    self.store.message(&message.id).await?.ok_or_else(|| {
                        DriverError::Backend("OpenCode message disappeared".into())
                    })?;
            }
        }
        Ok(DeliveryReceipt {
            state: row.state,
            backend_message_id: row
                .turn_id
                .and_then(|s| s.split_once('|').map(|(_, id)| id.to_owned())),
        })
    }

    async fn events(
        &self,
        id: &str,
        cursor: Option<&str>,
    ) -> Result<Vec<DriverEvent>, DriverError> {
        let instance = self
            .store
            .instance(id)
            .await?
            .ok_or_else(|| DriverError::UnknownInstance(id.into()))?;
        if instance.backend != Backend::Opencode || instance.delivery != "push" {
            return Err(DriverError::InvalidRequest(
                "not an OpenCode push instance".into(),
            ));
        }
        let after = match cursor {
            None => 0,
            Some(c) => c
                .strip_prefix("opencode:")
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|n| *n >= 0)
                .ok_or_else(|| {
                    DriverError::InvalidRequest("invalid OpenCode event cursor".into())
                })?,
        };
        Ok(self.store.opencode_events(id, after).await?)
    }
}
