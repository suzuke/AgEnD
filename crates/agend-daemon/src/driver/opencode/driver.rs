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
    /// Fresh session status with holder, handoff and credential checks on both
    /// sides. Callers must first pause/drain all input and separately verify
    /// the managed launch identity; this observation alone cannot authorize a stop.
    pub async fn session_idle(&self, id: &str) -> Result<bool, String> {
        let store = self.store.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || {
            use super::{api::Session, http::Http, launch::Layout};
            let lookup = id.clone();
            let instance = store
                .call_blocking(move |c| crate::store::instances::get(c, &lookup))
                .map_err(|e| e.to_string())?
                .ok_or("instance missing")?;
            if instance.backend != Backend::Opencode
                || instance.status != crate::store::InstanceStatus::Running
            {
                return Err("not a running OpenCode instance".into());
            }
            let session_id = instance.session_id.as_deref().ok_or("session missing")?;
            let holder = crate::runtime::files::running(store.home(), &id)
                .map_err(|e| e.to_string())?
                .ok_or("holder missing")?;
            let layout = Layout::new(store.home(), &id)?;
            let endpoint = layout
                .session_endpoint(holder, session_id)
                .map_err(|e| e.to_string())?;
            let password = layout.password().map_err(|e| e.to_string())?;
            let session = Session::resume(
                Http::new(endpoint.0, &password, &instance.working_directory)
                    .map_err(|e| e.to_string())?,
                session_id,
            )?;
            let idle = !session.busy()?;
            let lookup = id.clone();
            let current = store
                .call_blocking(move |c| crate::store::instances::get(c, &lookup))
                .map_err(|e| e.to_string())?;
            if current.as_ref() != Some(&instance)
                || crate::runtime::files::running(store.home(), &id).map_err(|e| e.to_string())?
                    != Some(holder)
                || layout
                    .session_endpoint(holder, session_id)
                    .map_err(|e| e.to_string())?
                    != endpoint
                || layout.password().map_err(|e| e.to_string())? != password
            {
                return Err("OpenCode identity changed during idle query".into());
            }
            Ok(idle)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Bounded read-only snapshot of the current holder/session's latest history.
    pub async fn message_outcome(
        &self,
        row: crate::store::Message,
    ) -> Result<agend_core::protocol::client::MessageOutcomeState, String> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            use super::{api::Session, history, http::Http, launch::Layout};
            use agend_core::protocol::client::MessageOutcomeState as State;
            if row.state != agend_core::model::DeliveryState::Confirmed {
                return Ok(State::Unknown);
            }
            let id = row.to_instance.clone();
            let lookup = id.clone();
            let instance = store
                .call_blocking(move |c| crate::store::instances::get(c, &lookup))
                .map_err(|e| e.to_string())?
                .ok_or("instance missing")?;
            if instance.backend != Backend::Opencode
                || instance.status != crate::store::InstanceStatus::Running
            {
                return Ok(State::Unknown);
            }
            let session_id = instance.session_id.as_deref().ok_or("session missing")?;
            let expected =
                crate::store::opencode::reference(session_id, &history::message_id(&row.id));
            if row.turn_id.as_deref() != Some(expected.as_str()) {
                return Ok(State::Unknown);
            }
            let holder = crate::runtime::files::running(store.home(), &id)
                .map_err(|e| e.to_string())?
                .ok_or("holder missing")?;
            let layout = Layout::new(store.home(), &id)?;
            let endpoint = layout.endpoint(holder).map_err(|e| e.to_string())?;
            let http = Http::new(
                endpoint.0,
                &layout.password().map_err(|e| e.to_string())?,
                &instance.working_directory,
            )
            .map_err(|e| e.to_string())?;
            let session = Session::resume(http, session_id)?;
            let (history, _) = session.history_page(16, None)?;
            let body =
                crate::delivery::render(&row.from_instance, row.task_id.as_deref(), &row.body);
            let outcome = history::message_outcome(session_id, &row.id, &body, &history)?;
            let lookup = id.clone();
            let current = store
                .call_blocking(move |c| crate::store::instances::get(c, &lookup))
                .map_err(|e| e.to_string())?;
            if current.as_ref() != Some(&instance)
                || crate::runtime::files::running(store.home(), &id).map_err(|e| e.to_string())?
                    != Some(holder)
                || layout.endpoint(holder).map_err(|e| e.to_string())? != endpoint
            {
                return Ok(State::Unknown);
            }
            Ok(outcome)
        })
        .await
        .map_err(|e| e.to_string())?
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
