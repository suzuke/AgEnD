//! Atomic program transition and rollback records. The supervisor must verify
//! canary admission and stop the exact old holder before committing either way.
use super::{SqliteStore, StoreError, instances, managed_launch};
use agend_core::runtime_records::{
    BackendSwitch, BackendSwitchPhase, Instance, InstanceStatus, ManagedLaunchIntent,
};
use agend_core::setup::backend::{ImportedBackend, valid_version};
use rusqlite::{Connection, OptionalExtension, params};

fn invalid(message: &str) -> StoreError {
    StoreError::Invalid(message.into())
}
fn read(conn: &Connection, instance: &str) -> Result<Option<BackendSwitch>, StoreError> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT id,record FROM backend_switches WHERE instance_id=?1",
            [instance],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(id, json)| {
        let record: BackendSwitch =
            serde_json::from_str(&json).map_err(|e| StoreError::Invalid(e.to_string()))?;
        if record.id != id
            || record.instance_id != instance
            || record.previous.instance_id != instance
            || !agend_core::protocol::client::is_uuid_v4(&id)
        {
            return Err(invalid("backend switch identity is inconsistent"));
        }
        Ok(record)
    })
    .transpose()
}
/// Read on the same DB thread/transaction as the attempt reservation. Existing
/// receipts remain valid; new content is held until activation is verified.
pub(crate) fn delivery_paused(conn: &Connection, instance: &str) -> Result<bool, StoreError> {
    Ok(read(conn, instance)?.is_some_and(|r| r.phase.pending()))
}

/// Prepared holds the old backend still. Committed/Restoring must permit the
/// newly reserved launch to pass startup menus before activation is finished.
pub(crate) fn startup_paused(conn: &Connection, instance: &str) -> Result<bool, StoreError> {
    Ok(read(conn, instance)?.is_some_and(|r| {
        matches!(
            r.phase,
            BackendSwitchPhase::Prepared | BackendSwitchPhase::RollbackPrepared
        )
    }))
}

fn write(conn: &Connection, record: &BackendSwitch) -> Result<(), StoreError> {
    let json = serde_json::to_string(record).map_err(|e| StoreError::Invalid(e.to_string()))?;
    conn.execute(
        "INSERT INTO backend_switches(instance_id,id,record) VALUES (?1,?2,?3) \
        ON CONFLICT(instance_id) DO UPDATE SET id=excluded.id,record=excluded.record",
        params![record.instance_id, record.id, json],
    )?;
    Ok(())
}
fn config_matches(instance: &Instance, record: &BackendSwitch, program: &str) -> bool {
    instance.program == program
        && instance.backend.as_str() == record.previous.artifact.backend
        && instance.args == record.previous.configured_args
        && instance.working_directory == record.previous.working_directory
        && instance.delivery == record.previous.delivery
        && instance.session_id == record.session_id
}
impl SqliteStore {
    pub async fn backend_switch(
        &self,
        instance: &str,
    ) -> Result<Option<BackendSwitch>, StoreError> {
        let instance = instance.to_owned();
        self.call(move |conn| read(conn, &instance)).await
    }
    /// Record an exact pending transition's first failure time. Repeated
    /// observations do not create a fresh notification episode.
    pub async fn backend_switch_problem(
        &self,
        expected: &BackendSwitch,
        reason: &str,
        now: u64,
    ) -> Result<BackendSwitch, StoreError> {
        if reason.is_empty() || reason.len() > 4096 {
            return Err(invalid("backend switch problem must be 1..4096 bytes"));
        }
        let expected = expected.clone();
        let reason = reason.to_owned();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let mut current = read(&tx, &expected.instance_id)?
                .ok_or_else(|| invalid("backend switch missing"))?;
            if current != expected || !current.phase.pending() {
                return Err(invalid("backend switch changed or is no longer pending"));
            }
            let since = current.problem.as_ref().map_or(now, |p| p.since_unix_ms);
            current.problem = Some(
                agend_core::runtime_records::backend_switch::BackendSwitchProblem {
                    reason,
                    since_unix_ms: since,
                },
            );
            if current != expected {
                write(&tx, &current)?;
            }
            tx.commit()?;
            Ok(current)
        })
        .await
    }
    /// Reserve without changing program. Unknown results are reconciled by read,
    /// never a new request. Only a completed previous transition can be replaced.
    pub async fn prepare_backend_switch(
        &self,
        instance: &Instance,
        target: ImportedBackend,
        program: &str,
        expected_previous: Option<&str>,
    ) -> Result<BackendSwitch, StoreError> {
        if target.format != 1
            || target.backend != instance.backend.as_str()
            || !valid_version(&target.version)
            || target.bytes == 0
            || target.sha256.len() != 64
            || !target.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || !std::path::Path::new(program).is_absolute()
        {
            return Err(invalid("invalid backend switch target"));
        }
        let instance = instance.clone();
        let program = program.to_owned();
        let expected = expected_previous.map(str::to_owned);
        let id = instances::new_session_id().map_err(StoreError::Io)?;
        self.call(move |conn| {
            let tx = conn.transaction()?;
            if instances::get(&tx, &instance.id)?.as_ref() != Some(&instance) {
                return Err(invalid("instance changed before backend switch"));
            }
            let old = read(&tx, &instance.id)?;
            if old.as_ref().map(|s| s.id.as_str()) != expected.as_deref()
                || old.as_ref().is_some_and(|s| s.phase.pending())
            {
                return Err(invalid("backend switch changed or is still pending"));
            }
            let previous = managed_launch::get(&tx, &instance.id)?
                .ok_or_else(|| invalid("managed launch proof required before switching"))?;
            let record = BackendSwitch {
                id,
                instance_id: instance.id.clone(),
                previous,
                target,
                target_program: program,
                session_id: instance.session_id.clone(),
                phase: BackendSwitchPhase::Prepared,
                problem: None,
                activation_deadline_unix_ms: None,
            };
            if !config_matches(&instance, &record, &record.previous.configured_program)
                || record.target == record.previous.artifact
            {
                return Err(invalid(
                    "backend switch source changed or target is already selected",
                ));
            }
            write(&tx, &record)?;
            tx.commit()?;
            Ok(record)
        })
        .await
    }
    /// Cancel only an uncommitted request. No process or program mutation is
    /// needed; even a running original agent may remain in place.
    pub async fn cancel_backend_switch(
        &self,
        expected: &BackendSwitch,
    ) -> Result<BackendSwitch, StoreError> {
        let expected = expected.clone();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let mut current = read(&tx, &expected.instance_id)?
                .ok_or_else(|| invalid("backend switch missing"))?;
            if current != expected || current.phase != BackendSwitchPhase::Prepared {
                return Err(invalid(
                    "only the current prepared backend switch can be cancelled",
                ));
            }
            let instance = instances::get(&tx, &current.instance_id)?
                .ok_or_else(|| invalid("backend switch instance disappeared"))?;
            if !config_matches(&instance, &current, &current.previous.configured_program) {
                return Err(invalid(
                    "backend switch configuration changed before cancellation",
                ));
            }
            current.phase = BackendSwitchPhase::Cancelled;
            current.problem = None;
            current.activation_deadline_unix_ms = None;
            write(&tx, &current)?;
            tx.commit()?;
            Ok(current)
        })
        .await
    }

    /// Persist rollback intent before any side effect, including failed activation.
    pub async fn prepare_backend_rollback(
        &self,
        expected: &BackendSwitch,
    ) -> Result<BackendSwitch, StoreError> {
        let expected = expected.clone();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let mut current = read(&tx, &expected.instance_id)?
                .ok_or_else(|| invalid("backend switch missing"))?;
            if current != expected
                || !matches!(
                    current.phase,
                    BackendSwitchPhase::Activated | BackendSwitchPhase::Committed
                )
            {
                return Err(invalid(
                    "only the current committed or activated switch can prepare rollback",
                ));
            }
            let instance = instances::get(&tx, &current.instance_id)?
                .ok_or_else(|| invalid("instance missing"))?;
            if !config_matches(&instance, &current, &current.target_program) {
                return Err(invalid(
                    "backend switch configuration changed before rollback",
                ));
            }
            current.phase = BackendSwitchPhase::RollbackPrepared;
            write(&tx, &current)?;
            tx.commit()?;
            Ok(current)
        })
        .await
    }

    /// Caller proves holder absence first. Persist phase and program in one
    /// transaction; preserve the old launch proof until a new native spawn.
    pub async fn commit_backend_switch(
        &self,
        expected: &BackendSwitch,
        rollback: bool,
        now: u64,
    ) -> Result<BackendSwitch, StoreError> {
        let expected = expected.clone();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let mut current = read(&tx, &expected.instance_id)?
                .ok_or_else(|| invalid("backend switch missing"))?;
            if current != expected {
                return Err(invalid("backend switch changed; reconcile before retry"));
            }
            let (required, phase, from, to) = if rollback {
                (
                    BackendSwitchPhase::Committed,
                    BackendSwitchPhase::Restoring,
                    current.target_program.clone(),
                    current.previous.configured_program.clone(),
                )
            } else {
                (
                    BackendSwitchPhase::Prepared,
                    BackendSwitchPhase::Committed,
                    current.previous.configured_program.clone(),
                    current.target_program.clone(),
                )
            };
            if current.phase != required
                && !(rollback && current.phase == BackendSwitchPhase::RollbackPrepared)
            {
                return Err(invalid(
                    "backend switch phase does not allow this operation",
                ));
            }
            if !rollback
                && managed_launch::get(&tx, &current.instance_id)?.as_ref()
                    != Some(&current.previous)
            {
                return Err(invalid(
                    "source launch reservation changed before backend switch",
                ));
            }
            let instance = instances::get(&tx, &current.instance_id)?
                .ok_or_else(|| invalid("backend switch instance disappeared"))?;
            if !config_matches(&instance, &current, &from) || instance.agent_pid.is_some() {
                return Err(invalid(
                    "backend switch configuration changed or agent cleanup is unproven",
                ));
            }
            tx.execute(
                "UPDATE instances SET program=?2 WHERE id=?1",
                params![current.instance_id, to],
            )?;
            current.phase = phase;
            current.activation_deadline_unix_ms =
                Some(now.saturating_add(
                    agend_core::runtime_records::backend_switch::ACTIVATION_WINDOW_MS,
                ));
            write(&tx, &current)?;
            tx.commit()?;
            Ok(current)
        })
        .await
    }

    /// The caller verifies native readiness and launch binding first, serialized
    /// with lifecycle operations. This transaction only validates that its exact
    /// instance/launch snapshot still applies before releasing queued delivery.
    /// Program commit alone is never activation evidence.
    pub async fn finish_backend_switch(
        &self,
        expected: &BackendSwitch,
        ready: &Instance,
        launch: &ManagedLaunchIntent,
    ) -> Result<BackendSwitch, StoreError> {
        let expected = expected.clone();
        let ready = ready.clone();
        let launch = launch.clone();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let mut current = read(&tx, &expected.instance_id)?
                .ok_or_else(|| invalid("backend switch missing"))?;
            if current != expected {
                return Err(invalid(
                    "backend switch changed; reconcile before finishing",
                ));
            }
            let (program, artifact, phase) = match current.phase {
                BackendSwitchPhase::Committed => (
                    &current.target_program,
                    &current.target,
                    BackendSwitchPhase::Activated,
                ),
                BackendSwitchPhase::Restoring => (
                    &current.previous.configured_program,
                    &current.previous.artifact,
                    BackendSwitchPhase::RolledBack,
                ),
                _ => return Err(invalid("backend switch is not awaiting activation")),
            };
            if ready.id != current.instance_id
                || instances::get(&tx, &current.instance_id)?.as_ref() != Some(&ready)
                || ready.status != InstanceStatus::Running
                || !ready.session_started
                || ready.agent_pid.is_none_or(|pid| pid <= 1)
                || !config_matches(&ready, &current, program)
                || managed_launch::get(&tx, &current.instance_id)?.as_ref() != Some(&launch)
                || launch.binding == current.previous.binding
                || launch.instance_id != current.instance_id
                || launch.artifact != *artifact
                || launch.configured_program != *program
                || launch.configured_args != ready.args
                || launch.working_directory != ready.working_directory
                || launch.delivery != ready.delivery
                || launch.session_id != ready.session_id
            {
                return Err(invalid(
                    "backend activation snapshot or launch identity changed",
                ));
            }
            current.phase = phase;
            current.problem = None;
            current.activation_deadline_unix_ms = None;
            write(&tx, &current)?;
            tx.commit()?;
            Ok(current)
        })
        .await
    }
}
