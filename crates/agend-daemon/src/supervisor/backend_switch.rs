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
    pub(super) async fn backend_switch(
        &mut self,
        command: BackendSwitchCommand,
    ) -> Result<Option<BackendSwitch>, Refusal> {
        let id = match &command {
            BackendSwitchCommand::Status { instance_id }
            | BackendSwitchCommand::Prepare { instance_id, .. }
            | BackendSwitchCommand::Cancel { instance_id, .. } => instance_id,
        };
        validate_id(id).map_err(|e| refused(e.to_string()))?;
        let instance = self
            .store
            .instance(id)
            .await
            .map_err(|e| refused(e.to_string()))?
            .ok_or_else(|| (error_code::UNKNOWN_INSTANCE, format!("no instance {id}")))?;
        match command {
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
                self.store
                    .cancel_backend_switch(&record)
                    .await
                    .map(Some)
                    .map_err(|e| refused(e.to_string()))
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
                self.store
                    .prepare_backend_switch(
                        &instance,
                        artifact,
                        program,
                        expected_previous.as_deref(),
                    )
                    .await
                    .map(Some)
                    .map_err(|e| refused(e.to_string()))
            }
        }
    }
}
