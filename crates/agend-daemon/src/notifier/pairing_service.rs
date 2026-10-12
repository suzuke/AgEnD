//! One pairing HTTP owner per daemon; client cancellation does not release ownership.
use super::{config, http::Api, pairing};
use crate::store::SqliteStore;
use agend_core::{telegram::pairing::*, traits::Clock};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;

struct Now;
impl Clock for Now {
    fn now_unix_ms(&self) -> u64 {
        crate::log::now_unix_ms()
    }
}

pub struct PairingService {
    store: Arc<SqliteStore>,
    configured: bool,
    serial: Mutex<()>,
    stopping: AtomicBool,
    #[cfg(test)]
    api: Option<Arc<Api>>,
}
impl PairingService {
    /// A configured notifier owns getUpdates until the next daemon boot.
    pub fn new(store: Arc<SqliteStore>, configured: bool) -> Arc<Self> {
        Arc::new(Self {
            store,
            configured,
            serial: Mutex::new(()),
            stopping: AtomicBool::new(false),
            #[cfg(test)]
            api: None,
        })
    }
    #[cfg(test)]
    pub(super) fn local_test(
        store: Arc<SqliteStore>,
        configured: bool,
        api: Arc<Api>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            configured,
            serial: Mutex::new(()),
            stopping: AtomicBool::new(false),
            api: Some(api),
        })
    }
    fn api(&self, token: &agend_core::config::SecretRef) -> Result<Arc<Api>, String> {
        #[cfg(test)]
        if let Some(api) = &self.api {
            return Ok(api.clone());
        }
        Ok(Arc::new(Api::new(config::resolve(token)?)))
    }
    /// The owned task completes publication even if the requesting socket disappears.
    /// A disconnected client queries Status; it must not replay a mutation.
    pub async fn execute(
        self: &Arc<Self>,
        operation: PairingOperation,
    ) -> Result<Option<PairingRecord>, String> {
        let service = self.clone();
        tokio::spawn(async move {
            let _guard = service.serial.lock().await;
            if service.stopping.load(Ordering::SeqCst) {
                return Err("Telegram pairing is stopping; query status after restart".into());
            }
            service.run(operation).await
        })
        .await
        .map_err(|_| "Telegram pairing worker stopped; query status".to_owned())?
    }
    /// Stop admission and wait for an already-started bounded HTTP operation to publish.
    pub async fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let _guard = self.serial.lock().await;
    }
    async fn run(&self, operation: PairingOperation) -> Result<Option<PairingRecord>, String> {
        let current = self
            .store
            .telegram_pairing()
            .await
            .map_err(|e| e.to_string())?;
        if operation == PairingOperation::Status {
            return Ok(current);
        }
        if self.configured && !matches!(operation, PairingOperation::Cancel { .. }) {
            return Err(
                "Telegram is configured; remove its configuration and restart before pairing"
                    .into(),
            );
        }
        if let PairingOperation::Begin {
            id,
            token,
            previous,
        } = operation
        {
            // Validate references and stale requests before any network activity.
            TelegramPairing::new(id.clone(), token.clone(), 1, "validation_bot".into(), &Now)?;
            if current.as_ref().map(|r| &r.session.id) != previous.as_ref()
                || current.as_ref().is_some_and(|r| {
                    r.session.id == id
                        || (r.phase == PairingPhase::Pending
                            && Now.now_unix_ms() < r.session.expires_at_ms)
                })
            {
                return Err("pairing changed or is pending; query status before beginning".into());
            }
            let pending = pairing::begin(self.api(&token)?, id, token, &Now).await?;
            return self
                .store
                .begin_telegram_pairing(&pending, previous.as_deref(), Now.now_unix_ms())
                .await
                .map(Some)
                .map_err(|e| e.to_string());
        }
        let record = current.ok_or("no Telegram pairing; begin first")?;
        let id = match &operation {
            PairingOperation::Poll { id }
            | PairingOperation::Confirm { id, .. }
            | PairingOperation::Cancel { id } => id,
            _ => unreachable!(),
        };
        if id != &record.session.id || record.phase != PairingPhase::Pending {
            return Err("pairing changed or closed; query status".into());
        }
        let updated = match operation {
            PairingOperation::Poll { .. } => {
                record.session.check_time(&Now)?;
                let observed =
                    pairing::poll(self.api(&record.session.token)?, &record.session, &Now).await?;
                self.store
                    .observe_telegram_pairing(&record, &observed, Now.now_unix_ms())
                    .await
            }
            PairingOperation::Confirm { candidate, .. } => {
                // Reject a mismatching destination before resolving a secret or calling HTTP.
                record.session.confirm(&candidate, &Now)?;
                pairing::confirm(
                    self.api(&record.session.token)?,
                    &record.session,
                    &candidate,
                    &Now,
                )
                .await?;
                self.store
                    .confirm_telegram_pairing(&record, &candidate, Now.now_unix_ms())
                    .await
            }
            PairingOperation::Cancel { .. } => self.store.cancel_telegram_pairing(&record).await,
            _ => unreachable!(),
        };
        updated.map(Some).map_err(|e| e.to_string())
    }
}
