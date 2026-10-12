//! Serialized version transition. Durable pauses span process replacement.
use super::*;
use agend_core::runtime_records::{BackendSwitch, BackendSwitchPhase};

impl Supervisor {
    /// A current-generation native exit may roll back only an unactivated
    /// candidate whose persisted launch still identifies the target exactly.
    pub(super) async fn rollback_lost_candidate(&mut self, id: &str, agent_exited: bool) {
        let attempt = async {
            let record = self
                .store
                .backend_switch(id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("backend switch missing")?;
            if record.phase != BackendSwitchPhase::Committed
                && !(agent_exited && record.phase == BackendSwitchPhase::RollbackPrepared)
            {
                return Ok(());
            }
            let instance = self
                .store
                .instance(id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("backend switch instance missing")?;
            let launch = self
                .managed_reconnect(&instance)
                .await?
                .ok_or("candidate launch missing")?;
            if launch.binding == record.previous.binding
                || launch.artifact != record.target
                || instance.program != record.target_program
                || instance.session_id != record.session_id
                || instance.args != record.previous.configured_args
                || instance.working_directory != record.previous.working_directory
                || instance.delivery != record.previous.delivery
            {
                return Err("candidate death or launch identity not established".into());
            }
            if let Some(pid) = files::running(&self.home, id).map_err(|e| e.to_string())? {
                if !agent_exited {
                    return Err("candidate holder still exists".into());
                }
                // Only a verified current-generation native Exited event may
                // replace idle evidence. Driver disconnects are insufficient.
                if record.phase == BackendSwitchPhase::Committed {
                    self.store
                        .prepare_backend_rollback(&record)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                self.drain_switch_replies(id).await?;
                self.codex.disconnect(id);
                self.opencode.disconnect(id);
                self.wait_switch_workers(id, true).await?;
                self.runtime
                    .stop_reserved(&launch, pid, instance.agent_pid.ok_or("agent PID missing")?)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            self.activate_switch(&instance, &record.id, true).await?;
            log::line(&format!(
                "{id}: candidate exited; restoring previous backend"
            ));
            Ok::<(), String>(())
        }
        .await;
        if let Err(error) = attempt {
            log::line(&format!("{id}: automatic backend rollback held: {error}"));
            self.report_switch_problem(id, &error).await;
        }
    }

    async fn backend_idle(&self, instance: &Instance) -> Result<bool, String> {
        if instance.delivery != "push" {
            return Err("backend switch requires native push readiness".into());
        }
        match instance.backend {
            Backend::Claude => {
                self.claude
                    .as_ref()
                    .ok_or("Claude observer unavailable")?
                    .session_idle(&instance.id)
                    .await
            }
            Backend::Codex => self
                .codex
                .thread_idle(&instance.id)
                .await
                .map_err(|e| e.to_string()),
            Backend::Opencode => {
                crate::driver::opencode::OpenCodeDriver::new(self.store.clone())
                    .session_idle(&instance.id)
                    .await
            }
        }
    }

    pub(super) async fn activate_switch(
        &mut self,
        instance: &Instance,
        switch_id: &str,
        rollback: bool,
    ) -> Result<BackendSwitch, String> {
        let id = &instance.id;
        let mut record = self
            .store
            .backend_switch(id)
            .await
            .map_err(|e| e.to_string())?
            .filter(|r| r.id == switch_id)
            .ok_or("backend switch changed or missing")?;
        let phase = if rollback {
            BackendSwitchPhase::Committed
        } else {
            BackendSwitchPhase::Prepared
        };
        if record.phase != phase
            && !(rollback
                && matches!(
                    record.phase,
                    BackendSwitchPhase::Activated | BackendSwitchPhase::RollbackPrepared
                ))
        {
            return Err(
                "backend switch phase does not permit this transition; query status".into(),
            );
        }
        let program = if rollback {
            &record.previous.configured_program
        } else {
            &record.target_program
        };
        let expected = if rollback {
            &record.previous.artifact
        } else {
            &record.target
        };
        let artifact = self
            .runtime
            .check_backend_program(instance.backend, program, &instance.working_directory)
            .await
            .map_err(|e| e.to_string())?;
        if artifact.as_ref() != Some(expected) {
            return Err("backend switch destination changed or is not admitted".into());
        }
        if rollback
            && matches!(
                record.phase,
                BackendSwitchPhase::Activated | BackendSwitchPhase::Committed
            )
        {
            record = self
                .store
                .prepare_backend_rollback(&record)
                .await
                .map_err(|e| e.to_string())?;
        }
        self.drain_switch_replies(id).await?;
        let holder = files::running(&self.home, id).map_err(|e| e.to_string())?;
        if let Some(pid) = holder {
            let intent = self
                .managed_reconnect(instance)
                .await?
                .ok_or("managed source launch missing")?;
            if !rollback && intent != record.previous {
                return Err("source launch changed".into());
            }
            // OpenCode REST remains available after its delivery worker exits.
            if instance.backend == Backend::Opencode {
                self.opencode.disconnect(id);
                self.wait_switch_workers(id, false).await?;
            }
            if !self.backend_idle(instance).await? {
                return Err("backend is not proven idle; switch remains pending".into());
            }
            self.codex.disconnect(id);
            self.opencode.disconnect(id);
            self.wait_switch_workers(id, true).await?;
            let current = self.store.instance(id).await.map_err(|e| e.to_string())?;
            if current.as_ref() != Some(instance) {
                return Err("instance changed before managed stop".into());
            }
            self.runtime
                .stop_reserved(&intent, pid, instance.agent_pid.ok_or("agent PID missing")?)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            self.codex.disconnect(id);
            self.opencode.disconnect(id);
            self.wait_switch_workers(id, true).await?;
        }
        self.watches.remove(id);
        self.sweep(instance, "backend version transition").await;
        let committed = self
            .store
            .commit_backend_switch(&record, rollback, log::now_unix_ms())
            .await
            .map_err(|e| e.to_string())?;
        let current = self
            .store
            .instance(id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("instance disappeared")?;
        self.start(&current, current.session_started, None).await;
        Ok(committed)
    }

    async fn drain_switch_replies(&self, id: &str) -> Result<(), String> {
        let fence = self
            .delivery_replies
            .as_ref()
            .ok_or("server reply tracker unavailable")?
            .fence(id);
        tokio::time::timeout(Duration::from_secs(10), async {
            while !fence.drained() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "prior input has not drained; switch remains pending")?;
        Ok(())
    }

    async fn wait_switch_workers(&self, id: &str, codex: bool) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !self.opencode.workers_stopped(id) || (codex && !self.codex.workers_stopped(id)) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "delivery worker still running; holder preserved".into())
    }

    /// Boot may resume a committed destination, but must never fall through
    /// to ordinary start on missing admission or a changed configuration.
    pub(super) async fn recover_switch_start(&mut self, instance: &Instance) -> bool {
        let Ok(Some(record)) = self.store.backend_switch(&instance.id).await else {
            return false;
        };
        let (program, artifact) = match record.phase {
            BackendSwitchPhase::Committed => (&record.target_program, &record.target),
            BackendSwitchPhase::Restoring => (
                &record.previous.configured_program,
                &record.previous.artifact,
            ),
            _ => return false,
        };
        if instance.program != *program
            || instance.args != record.previous.configured_args
            || instance.working_directory != record.previous.working_directory
            || instance.session_id != record.session_id
            || instance.delivery != record.previous.delivery
            || instance.backend.as_str() != artifact.backend
        {
            log::line(&format!(
                "{}: pending switch configuration changed; recovery held",
                instance.id
            ));
            return false;
        }
        let admitted = self
            .runtime
            .check_backend_program(instance.backend, program, &instance.working_directory)
            .await;
        if !matches!(admitted, Ok(Some(ref current)) if current == artifact) {
            log::line(&format!(
                "{}: pending switch destination is not admitted; recovery held",
                instance.id
            ));
            return false;
        }
        self.start(instance, instance.session_started, None).await;
        true
    }

    /// Poll only committed launches. Native readiness precedes releasing delivery.
    pub(super) async fn finish_switches(&mut self) {
        let Ok(instances) = self.store.instances().await else {
            return;
        };
        for instance in instances {
            let Ok(Some(record)) = self.store.backend_switch(&instance.id).await else {
                continue;
            };
            if record.phase == BackendSwitchPhase::RollbackPrepared {
                // Server reply fencing exists only after boot. Resume the
                // persisted rollback here, whether its holder survived or not.
                if let Err(error) = self.activate_switch(&instance, &record.id, true).await {
                    self.report_switch_problem(&instance.id, &error).await;
                }
                continue;
            }
            if !matches!(
                record.phase,
                BackendSwitchPhase::Committed | BackendSwitchPhase::Restoring
            ) {
                continue;
            }
            let result = self.finish_switch(&instance, &record).await;
            if let Ok(true) = result {
                self.dismiss_switch_problem(&record);
                log::line(&format!(
                    "{}: backend switch {} activated",
                    instance.id, record.id
                ));
            } else if record.problem.is_none() {
                if record.activation_expired(log::now_unix_ms()) {
                    self.report_switch_problem(&instance.id,
                        "backend activation exceeded 300 seconds; holder preserved; inspect switch status and terminal"
                    ).await;
                } else if record.activation_deadline_unix_ms.is_none() {
                    self.report_switch_problem(&instance.id,
                        "pending activation predates deadline tracking; holder preserved; inspect switch status and terminal"
                    ).await;
                }
            }
        }
    }

    async fn finish_switch(
        &self,
        instance: &Instance,
        record: &BackendSwitch,
    ) -> Result<bool, String> {
        let Some(intent) = self.managed_reconnect(instance).await? else {
            return Ok(false);
        };
        if intent.binding == record.previous.binding {
            return Ok(false);
        }
        let Some(pid) = files::running(&self.home, &instance.id).map_err(|e| e.to_string())? else {
            return Ok(false);
        };
        let agent = instance.agent_pid.ok_or("agent PID missing")?;
        self.runtime
            .verify_reserved(&intent, pid, agent)
            .await
            .map_err(|e| e.to_string())?;
        if !self.backend_idle(instance).await? {
            return Ok(false);
        }
        self.runtime
            .verify_reserved(&intent, pid, agent)
            .await
            .map_err(|e| e.to_string())?;
        self.store
            .finish_backend_switch(record, instance, &intent)
            .await
            .map_err(|e| e.to_string())?;
        Ok(true)
    }
}
