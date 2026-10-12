//! Describe the daemon's actual version gates, without issuing an admission grant.
use agend_core::{
    policy::codex_input::{APPROVED_CLI_VERSION, CodexInputPolicy},
    setup::backend::observation::{
        BackendCapabilityPolicy as Policy, BackendDiagnostic, CapabilityPolicyKind as Kind,
    },
};

pub(super) fn policies(instance: &BackendDiagnostic, input: &CodexInputPolicy) -> Vec<Policy> {
    match instance.backend.as_str() {
        "codex" => {
            let (kind, version) = if input.allows_instance(&instance.instance_id) {
                (Kind::VerificationOverride, "explicit verification override for this instance only".into())
            } else if input.allows_version(APPROVED_CLI_VERSION) {
                (Kind::ExactVersion, APPROVED_CLI_VERSION.into())
            } else {
                (Kind::Disabled, "no approved production input version in this daemon policy".into())
            };
            vec![Policy {
                capability_id: "codex_terminal_input".into(), policy_kind: kind, version_constraint: version,
                additional_requirements: "current holder PID (production mode also requires its matching launch record), connected driver link, durable own-clientId attribution and terminal control authorization".into(),
                evidence_scope: "operator terminal input only; not a global Codex driver version range".into(),
            }]
        }
        "claude" => vec![Policy {
            capability_id: "claude_startup_frame_recognition".into(), policy_kind: Kind::RecordedFrames,
            version_constraint: format!("recorded Claude {}; dimensions {:?}",
                agend_core::screen::claude_startup::RECORDED_VERSION,
                agend_core::screen::claude_startup::recorded_dimensions()),
            additional_requirements: "complete recorded frame and workspace match; only the approved single-row Ready suggestion may vary; startup actions still require their lifecycle and holder/session checks".into(),
            evidence_scope: "recorded screen evidence, not a CLI version admission rule, live readiness or login proof".into(),
        }],
        "opencode" => vec![
            Policy {
                capability_id: "opencode_driver_endpoint".into(), policy_kind: Kind::ScopedVersion,
                version_constraint: format!("exact canary scope first, then managed artifact, otherwise {}", crate::driver::opencode::UNMANAGED_ENDPOINT_VERSION),
                additional_requirements: "endpoint version equals selected expectation; health is healthy and reports the same version; holder/session identity checks still apply".into(),
                evidence_scope: "version-selection rule only; this diagnostic does not resolve canary scope or validate an artifact; local HTTP authentication is not provider authentication".into(),
            },
            Policy {
                capability_id: "opencode_permission_reply".into(), policy_kind: Kind::ExactVersion,
                version_constraint: crate::driver::opencode::PERMISSION_REPLY_VERSION.into(),
                additional_requirements: "current holder and endpoint, pending permission/session identity and durable decision checks".into(),
                evidence_scope: "permission replies only; a newer managed version passing canary does not widen this gate".into(),
            },
        ],
        _ => vec![],
    }
}

/// Observe without opening a second Codex connection or sending model input.
pub(super) async fn runtime(
    ctx: &super::Context,
    expected: &BackendDiagnostic,
) -> Option<agend_core::setup::backend::observation::RuntimeDiagnostic> {
    use agend_core::{
        model::Backend,
        setup::{Check, CheckStatus, backend::observation::RuntimeDiagnostic},
    };
    let id = &expected.instance_id;
    let before = ctx.store.instance(id).await.ok()??;
    if before.backend.as_str() != expected.backend
        || before.program != expected.configured_program
        || before.working_directory != expected.working_directory
    {
        return None;
    }
    let holder = crate::runtime::files::running(ctx.runtime.home(), id).ok()??;
    let result = match before.backend {
        Backend::Codex => {
            let configuration = ctx.codex.account_configuration(id).await.ok()?;
            let eligible = ctx.codex.can_input(id);
            RuntimeDiagnostic {
                session_id: before.session_id.clone()?, holder_pid: holder,
                observed_ms: crate::log::now_unix_ms(), credential_configuration: configuration,
                provider_observations: vec![],
                capabilities: vec![Check { check: "codex_terminal_input".into(),
                    status: if eligible { CheckStatus::Ok } else { CheckStatus::Warn },
                    detail: "current driver version/link/holder/strict attribution gate; terminal control must still be acquired separately".into(),
                    fix: (!eligible).then(|| "check the instance terminal and supported input version; reconnect the instance after repairing its configuration".into()) }],
            }
        }
        Backend::Opencode => crate::driver::opencode::OpenCodeDriver::new(ctx.store.clone())
            .diagnostic(id)
            .await
            .ok()?,
        Backend::Claude => return None,
    };
    if ctx.store.instance(id).await.ok()?.as_ref() != Some(&before)
        || crate::runtime::files::running(ctx.runtime.home(), id).ok()? != Some(holder)
        || result.holder_pid != holder
        || before.session_id.as_deref() != Some(result.session_id.as_str())
    {
        return None;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_policy_does_not_turn_a_verification_override_into_general_admission() {
        let mut instance = BackendDiagnostic {
            instance_id: "probe".into(),
            backend: "codex".into(),
            configured_program: "/bin/false".into(),
            working_directory: "/workspace".into(),
            external_version: None,
            managed_reservation: None,
        };
        assert_eq!(
            policies(&instance, &CodexInputPolicy::default())[0].policy_kind,
            Kind::Disabled
        );
        let approved = policies(&instance, &CodexInputPolicy::approved());
        assert_eq!(approved[0].policy_kind, Kind::ExactVersion);
        assert!(CodexInputPolicy::approved().allows_version(&approved[0].version_constraint));
        let override_policy = CodexInputPolicy::for_u17_verification("probe".into());
        assert_eq!(
            policies(&instance, &override_policy)[0].policy_kind,
            Kind::VerificationOverride
        );
        instance.instance_id = "other".into();
        assert_eq!(
            policies(&instance, &override_policy)[0].policy_kind,
            Kind::Disabled
        );
    }
}
