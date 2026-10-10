//! Managed admission and durable intent reconciliation before holder IO.
use super::*;
use agend_core::runtime_records::ManagedLaunchIntent;
use agend_core::setup::backend::ImportedBackend;

impl Supervisor {
    pub(super) fn managed_program(&self, artifact: &ImportedBackend) -> Result<String, String> {
        self.home
            .canonicalize()
            .map(|home| {
                home.join("backends")
                    .join(&artifact.backend)
                    .join(&artifact.version)
                    .join("program")
            })
            .map_err(|e| e.to_string())?
            .into_os_string()
            .into_string()
            .map_err(|_| "managed executable path is not UTF-8".into())
    }

    pub(super) async fn managed_reconnect(
        &self,
        instance: &Instance,
    ) -> Result<Option<ManagedLaunchIntent>, String> {
        let previous = self
            .store
            .managed_launch(&instance.id)
            .await
            .map_err(|e| e.to_string())?;
        let artifact = self
            .runtime
            .inspect_backend_program(
                instance.backend,
                &instance.program,
                &instance.working_directory,
            )
            .await
            .map_err(|e| e.to_string())?;
        match (artifact, previous) {
            (None, None) => Ok(None),
            (Some(artifact), Some(intent)) => {
                if intent.artifact != artifact || !same_config(instance, &intent) {
                    return Err(
                        "managed launch configuration changed; existing holder preserved".into(),
                    );
                }
                // Retain the original actual argv (fresh versus resume); never
                // reconstruct it or manufacture a new reservation on reconnect.
                Ok(Some(intent))
            }
            _ => Err("managed holder has no matching persisted intent; holder preserved".into()),
        }
    }

    pub(super) async fn reserve_managed(
        &self,
        instance: &Instance,
        launch: &HolderLaunch,
        artifact: ImportedBackend,
    ) -> Result<ManagedLaunchIntent, String> {
        match crate::runtime::files::running(&self.home, &instance.id) {
            Ok(None) => {}
            Ok(Some(_)) => return Err("old managed holder still runs; intent preserved".into()),
            Err(e) => return Err(format!("cannot prove old managed holder absent: {e}")),
        }
        let current = self
            .store
            .instance(&instance.id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("managed instance disappeared before reservation")?;
        // Sweeping may clear agent_pid, but must not silently adopt new config.
        if current.backend != instance.backend
            || current.program != instance.program
            || current.args != instance.args
            || current.session_id != instance.session_id
            || current.delivery != instance.delivery
            || current.working_directory != instance.working_directory
            || current.agent_pid.is_some()
        {
            return Err("managed instance changed or orphan cleanup is unproven".into());
        }
        let previous = self
            .store
            .managed_launch(&instance.id)
            .await
            .map_err(|e| e.to_string())?;
        self.store
            .prepare_managed_launch(
                &current,
                launch,
                artifact,
                previous.as_ref().map(|intent| intent.binding.as_str()),
            )
            .await
            .map_err(|e| e.to_string())
    }
}

fn same_config(instance: &Instance, intent: &ManagedLaunchIntent) -> bool {
    instance.id == intent.instance_id
        && instance.backend.as_str() == intent.artifact.backend
        && instance.program == intent.configured_program
        && instance.args == intent.configured_args
        && launch_session_matches(
            instance.backend,
            intent.session_id.as_deref(),
            instance.session_id.as_deref(),
        )
        && instance.delivery == intent.delivery
        && instance.working_directory == intent.working_directory
}

// The intent records the session specified at launch, not a new authority for
// sessions discovered by the native driver after a first Codex/OpenCode spawn.
fn launch_session_matches(backend: Backend, launch: Option<&str>, current: Option<&str>) -> bool {
    launch == current || (launch.is_none() && matches!(backend, Backend::Codex | Backend::Opencode))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_native_first_session_discovery_can_differ_from_launch() {
        for backend in [Backend::Codex, Backend::Opencode] {
            assert!(launch_session_matches(
                backend,
                None,
                Some("native-session")
            ));
            assert!(launch_session_matches(
                backend,
                Some("original"),
                Some("original")
            ));
            assert!(!launch_session_matches(
                backend,
                Some("original"),
                Some("replacement")
            ));
            assert!(!launch_session_matches(backend, Some("original"), None));
        }
        assert!(!launch_session_matches(
            Backend::Claude,
            None,
            Some("native-session")
        ));
        assert!(!launch_session_matches(
            Backend::Claude,
            Some("original"),
            Some("replacement")
        ));
    }
}
