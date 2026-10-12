//! Operator preparation and reconciliation, serialized with instance lifecycle.
//! Preparation pauses delivery; it does not stop a holder or activate a version.
use super::*;
use agend_core::{
    protocol::client::BackendSwitchCommand, runtime_records::BackendSwitch,
    setup::backend::valid_version,
};

fn refused(message: impl Into<String>) -> Refusal {
    (error_code::INVALID_REQUEST, message.into())
}

impl Supervisor {
    pub(super) fn show_switch_problem(&self, record: &BackendSwitch) {
        let Some(problem) = &record.problem else {
            return;
        };
        self.fleet.upsert_attention(AttentionRequiredData {
            attention_id: Some(format!(
                "backend-switch:{}:{}",
                record.instance_id, record.id
            )),
            instance_id: Some(record.instance_id.clone()),
            task_id: None,
            ask: None,
            recap: None,
            reason: format!(
                "{}: backend switch held: {}",
                record.instance_id, problem.reason
            ),
            unblocks: Some(0),
            waiting_since_unix_ms: Some(problem.since_unix_ms),
            if_ignored: Some(format!(
                "delivery remains paused; inspect agend backend switch status {}",
                record.instance_id
            )),
            actions: Vec::new(),
        });
    }
    pub(super) async fn report_switch_problem(&self, id: &str, reason: &str) {
        let result = async {
            let Some(record) = self.store.backend_switch(id).await? else {
                return Ok(());
            };
            if !record.phase.pending() {
                return Ok(());
            }
            let mut reason = reason.to_owned();
            if reason.len() > 4096 {
                let mut end = 4096;
                while !reason.is_char_boundary(end) {
                    end -= 1;
                }
                reason.truncate(end);
            }
            let saved = self
                .store
                .backend_switch_problem(&record, &reason, log::now_unix_ms())
                .await?;
            self.show_switch_problem(&saved);
            Ok::<(), crate::store::StoreError>(())
        }
        .await;
        if let Err(error) = result {
            log::line(&format!("{id}: cannot save switch problem: {error}"));
        }
    }
    pub(super) fn dismiss_switch_problem(&self, record: &BackendSwitch) {
        self.fleet.dismiss(&format!(
            "backend-switch:{}:{}",
            record.instance_id, record.id
        ));
    }

    /// Generic restart/retry must not replace a durable switch's source launch.
    /// Read failures also preserve the holder; only switch reconciliation may
    /// decide what to stop or launch while the transition is pending.
    pub(super) async fn switch_holds_recovery(&self, id: &str) -> bool {
        let reason = match self.store.backend_switch(id).await {
            Ok(Some(record)) if record.phase.pending() => {
                format!(
                    "backend switch {} is pending; ordinary recovery held",
                    record.id
                )
            }
            Ok(_) => return false,
            Err(error) => format!("cannot inspect backend switch: {error}; ordinary recovery held"),
        };
        log::line(&format!("{id}: {reason}"));
        self.set_state(id, AgentState::Unknown, reason);
        true
    }

    pub(super) async fn backend_switch(
        &mut self,
        command: BackendSwitchCommand,
    ) -> Result<Option<BackendSwitch>, Refusal> {
        let id = match &command {
            BackendSwitchCommand::Status { instance_id }
            | BackendSwitchCommand::Prepare { instance_id, .. }
            | BackendSwitchCommand::Cancel { instance_id, .. }
            | BackendSwitchCommand::Activate { instance_id, .. }
            | BackendSwitchCommand::Rollback { instance_id, .. } => instance_id,
        };
        validate_id(id).map_err(|e| refused(e.to_string()))?;
        let instance = self
            .store
            .instance(id)
            .await
            .map_err(|e| refused(e.to_string()))?
            .ok_or_else(|| (error_code::UNKNOWN_INSTANCE, format!("no instance {id}")))?;
        match command {
            BackendSwitchCommand::Activate { switch_id, .. } => self
                .activate_switch(&instance, &switch_id, false)
                .await
                .map(Some)
                .map_err(refused),
            BackendSwitchCommand::Rollback { switch_id, .. } => self
                .activate_switch(&instance, &switch_id, true)
                .await
                .map(Some)
                .map_err(refused),
            BackendSwitchCommand::Status { .. } => self
                .store
                .backend_switch(&instance.id)
                .await
                .map_err(|e| refused(e.to_string())),
            BackendSwitchCommand::Cancel { switch_id, .. } => {
                let record = self
                    .store
                    .backend_switch(&instance.id)
                    .await
                    .map_err(|e| refused(e.to_string()))?
                    .filter(|record| record.id == switch_id)
                    .ok_or_else(|| {
                        refused("backend switch changed or missing; query status before cancelling")
                    })?;
                let cancelled = self
                    .store
                    .cancel_backend_switch(&record)
                    .await
                    .map_err(|e| refused(e.to_string()))?;
                if instance.status == InstanceStatus::Running {
                    match files::running(&self.home, &instance.id) {
                        Ok(Some(pid)) => {
                            let connected = self
                                .runtime
                                .terminal_connection(&instance.id)
                                .is_ok_and(|c| c.is_current());
                            if connected {
                                let restart_delivery = match instance.backend {
                                    Backend::Claude => false,
                                    Backend::Codex => {
                                        self.codex.connected_busy(&instance.id).is_none()
                                    }
                                    Backend::Opencode => {
                                        self.opencode.workers_stopped(&instance.id)
                                    }
                                };
                                if restart_delivery
                                    && let Some(watch) = self.watches.get(&instance.id)
                                {
                                    self.connect_codex(&instance, watch.generation);
                                }
                            } else {
                                self.reconnect(&instance, pid).await;
                            }
                        }
                        Ok(None) => self.start(&instance, instance.session_started, None).await,
                        Err(error) => log::line(&format!(
                            "{}: cancelled but resume inspection failed: {error}",
                            instance.id
                        )),
                    }
                }
                self.dismiss_switch_problem(&cancelled);
                Ok(Some(cancelled))
            }
            BackendSwitchCommand::Prepare {
                version,
                expected_previous,
                ..
            } => {
                if !valid_version(&version) {
                    return Err(refused("invalid backend version"));
                }
                if instance.status != InstanceStatus::Running {
                    return Err(refused(
                        "backend switch requires a running managed instance",
                    ));
                }
                // Inspect the source without requiring a new canary: existing
                // holders may legitimately belong to an earlier daemon build.
                self.managed_reconnect(&instance)
                    .await
                    .map_err(refused)?
                    .ok_or_else(|| refused("backend switch requires a managed source launch"))?;
                let program = self
                    .home
                    .canonicalize()
                    .map_err(|e| refused(e.to_string()))?
                    .join("backends")
                    .join(instance.backend.as_str())
                    .join(&version)
                    .join("program");
                let program = program
                    .to_str()
                    .ok_or_else(|| refused("backend path is not UTF-8"))?;
                let artifact = self
                    .runtime
                    .check_backend_program(instance.backend, program, &instance.working_directory)
                    .await
                    .map_err(|e| refused(e.to_string()))?
                    .ok_or_else(|| refused("backend switch target is not managed"))?;
                let replies = self.delivery_replies.as_ref().ok_or_else(|| {
                    refused("backend switch requires the active server reply tracker")
                })?;
                let record = self
                    .store
                    .prepare_backend_switch(
                        &instance,
                        artifact,
                        program,
                        expected_previous.as_deref(),
                    )
                    .await
                    .map_err(|e| refused(e.to_string()))?;
                let fence = replies.fence(&instance.id);
                let drained = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    while !fence.drained() {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await;
                if drained.is_err() {
                    return Err(refused(format!(
                        "switch {} remains prepared; prior delivery replies have not drained; query status before proceeding",
                        record.id
                    )));
                }
                Ok(Some(record))
            }
        }
    }
}
