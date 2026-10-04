//! Driver composition for all dispatch paths. Inbox instances keep the generic
//! persisted message path; only Claude push uses the Claude adapter.
use super::claude::ClaudeDriver;
use super::codex::{CodexDriver, DriverError};
use crate::store::SqliteStore;
use agend_core::{model::Backend, policy::busy::BusyLevel, traits::*};
use std::sync::Arc;

#[derive(Clone)]
pub struct BackendDriver {
    codex: CodexDriver,
    claude: ClaudeDriver,
    store: Arc<SqliteStore>,
}
impl BackendDriver {
    pub fn new(codex: CodexDriver, store: Arc<SqliteStore>) -> Self {
        Self {
            codex,
            claude: ClaudeDriver::new(store.clone()),
            store,
        }
    }
    async fn claude_push(&self, id: &str) -> Result<bool, DriverError> {
        let instance = self
            .store
            .instance(id)
            .await?
            .ok_or_else(|| DriverError::UnknownInstance(id.into()))?;
        Ok(instance.backend == Backend::Claude && instance.delivery == "push")
    }
}
impl Driver for BackendDriver {
    type Error = DriverError;
    async fn deliver(
        &self,
        id: &str,
        message: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, Self::Error> {
        if self.claude_push(id).await? {
            self.claude.deliver(id, message, mode).await
        } else {
            self.codex.deliver(id, message, mode).await
        }
    }
    async fn events(
        &self,
        id: &str,
        cursor: Option<&str>,
    ) -> Result<Vec<DriverEvent>, Self::Error> {
        if self.claude_push(id).await? {
            self.claude.events(id, cursor).await
        } else {
            self.codex.events(id, cursor).await
        }
    }
}
