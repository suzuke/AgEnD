//! Driver composition for all dispatch paths. Inbox instances keep the generic
//! persisted message path; push instances use their backend adapter.
use super::claude::ClaudeDriver;
use super::codex::{CodexDriver, DriverError};
use crate::store::SqliteStore;
use agend_core::{model::Backend, policy::busy::BusyLevel, traits::*};
use std::sync::Arc;

#[derive(Clone)]
pub struct BackendDriver {
    codex: CodexDriver,
    claude: ClaudeDriver,
    opencode: super::opencode::OpenCodeDriver,
    store: Arc<SqliteStore>,
}
impl BackendDriver {
    pub fn new(codex: CodexDriver, store: Arc<SqliteStore>) -> Self {
        Self {
            codex,
            claude: ClaudeDriver::new(store.clone()),
            opencode: super::opencode::OpenCodeDriver::new(store.clone()),
            store,
        }
    }
    async fn push_backend(&self, id: &str) -> Result<Option<Backend>, DriverError> {
        let instance = self
            .store
            .instance(id)
            .await?
            .ok_or_else(|| DriverError::UnknownInstance(id.into()))?;
        Ok((instance.delivery == "push").then_some(instance.backend))
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
        match self.push_backend(id).await? {
            Some(Backend::Claude) => self.claude.deliver(id, message, mode).await,
            Some(Backend::Opencode) => self.opencode.deliver(id, message, mode).await,
            _ => self.codex.deliver(id, message, mode).await,
        }
    }
    async fn events(
        &self,
        id: &str,
        cursor: Option<&str>,
    ) -> Result<Vec<DriverEvent>, Self::Error> {
        match self.push_backend(id).await? {
            Some(Backend::Claude) => self.claude.events(id, cursor).await,
            Some(Backend::Opencode) => self.opencode.events(id, cursor).await,
            _ => self.codex.events(id, cursor).await,
        }
    }
}
