//! Scoped persisted and read-only native observations from the connected daemon.
use super::{Check, CheckStatus, check};
use agend_client::Client;
use agend_core::protocol::client::{
    ClientRequest, ClientResponse, CommandResult, InstanceView, OperatorCommand, OperatorData, V1_9,
};
use agend_core::setup::backend::observation::BackendCapabilityPolicy;
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub(super) fn checks(home: &Path, client: &Client, instances: &[InstanceView]) -> Vec<Check> {
    let mut out = vec![setup(home, client)];
    for instance in instances {
        let (detail, policies, runtime) = read(home, client, instance)
            .unwrap_or_else(|e| (format!("unknown; {e}"), vec![], None));
        out.push(check(&format!("observation/{}", instance.instance_id), CheckStatus::Warn, detail,
            Some("compare the recorded scope and attempt time with the current configuration; disk observations and launch reservations do not prove the running image or login".into())));
        out.extend(runtime_checks(instance, runtime));
        for policy in policies {
            out.push(check(&format!("policy/{}/{}", instance.instance_id, policy.capability_id),
                CheckStatus::Ok,
                format!("daemon policy {:?}: {}; requires {}; evidence scope: {}; policy description only; not a runtime grant",
                    policy.policy_kind, policy.version_constraint, policy.additional_requirements, policy.evidence_scope),
                None));
        }
    }
    out
}

fn read(
    home: &Path,
    client: &Client,
    instance: &InstanceView,
) -> Result<
    (
        String,
        Vec<BackendCapabilityPolicy>,
        Option<agend_core::setup::backend::observation::RuntimeDiagnostic>,
    ),
    String,
> {
    if client.daemon().selected < V1_9 {
        return Err("daemon does not support backend diagnostics; upgrade and reconnect".into());
    }
    let request = ClientRequest::Operator {
        data: OperatorData {
            request_id: format!("doctor:{}", instance.instance_id),
            command: OperatorCommand::BackendDiagnostic {
                instance_id: instance.instance_id.clone(),
            },
        },
    };
    let boot_id = client
        .daemon()
        .boot_id
        .ok_or("daemon boot identity unknown")?;
    let response = agend_client::exchange_once(
        &home.join(agend_core::protocol::client::DAEMON_SOCKET),
        None,
        V1_9,
        &request,
        Instant::now() + Duration::from_secs(20),
    )
    .map_err(|e| e.to_string())?;
    let ClientResponse::CommandResult { data } = response else {
        return Err("unexpected daemon diagnostic response".into());
    };
    let CommandResult::BackendDiagnostic { data: reply } = data.result else {
        return Err("no diagnostic snapshot for this instance".into());
    };
    describe(*reply, instance, boot_id)
}

fn describe(
    reply: agend_core::setup::backend::observation::BackendDiagnosticReply,
    instance: &InstanceView,
    boot_id: u64,
) -> Result<
    (
        String,
        Vec<BackendCapabilityPolicy>,
        Option<agend_core::setup::backend::observation::RuntimeDiagnostic>,
    ),
    String,
> {
    if reply.boot_id != boot_id {
        return Err("daemon restarted since fleet snapshot; run doctor again".into());
    }
    let snapshot = reply
        .snapshot
        .ok_or("no diagnostic snapshot for this instance")?;
    if snapshot.instance_id != instance.instance_id
        || snapshot.backend != instance.backend
        || Some(snapshot.configured_program.as_str()) != instance.program.as_deref()
        || Some(snapshot.working_directory.as_str()) != instance.working_directory.as_deref()
    {
        return Err("configuration changed since fleet snapshot; run doctor again".into());
    }
    let mut parts = vec![format!(
        "daemon boot {:?}; persisted configuration snapshot",
        Some(boot_id)
    )];
    if let Some(row) = snapshot.external_version {
        parts.push(format!(
            "external probe attempt={} started_ms={} completed_ms={:?} generation={:?} error={:?}",
            row.attempt, row.started_ms, row.completed_ms, row.generation, row.error
        ));
        if let Some(version) = row.latest {
            parts.push(format!("last successful disk sample: program={:?} version={:?} sha256={}; may predate the current attempt; not a loaded-holder identity",
                version.resolved_program, version.version_output, version.sha256));
        } else {
            parts.push("no successful external disk sample".into());
        }
    } else {
        parts.push("no matching external version observation".into());
    }
    if let Some(row) = snapshot.managed_reservation {
        parts.push(format!("managed pre-spawn reservation binding={:?} version={:?} sha256={}; not proof of a running holder",
            row.binding, row.artifact.version, row.artifact.sha256));
    } else {
        parts.push("no matching managed launch reservation".into());
    }
    parts.push(
        "disk evidence does not establish authentication or the running daemon binary digest"
            .into(),
    );
    Ok((parts.join("; "), reply.policies, reply.runtime))
}

fn runtime_checks(
    instance: &InstanceView,
    runtime: Option<agend_core::setup::backend::observation::RuntimeDiagnostic>,
) -> Vec<Check> {
    use agend_core::setup::backend::observation::{
        CredentialConfiguration as C, ProviderOutcome as P,
    };
    let name = format!("authentication/{}", instance.instance_id);
    let fix = Some("open this instance's terminal and configure its backend/provider login, then rerun doctor; doctor does not login, refresh, or send model requests".into());
    let Some(runtime) = runtime else {
        return vec![check(
            &name,
            CheckStatus::Warn,
            "current native account/session observation unavailable; provider validity unknown"
                .into(),
            fix,
        )];
    };
    let scope = format!(
        "session={} holder={} observed_ms={}",
        runtime.session_id, runtime.holder_pid, runtime.observed_ms
    );
    let (status, detail) = match runtime.credential_configuration {
        C::Configured => (
            CheckStatus::Ok,
            "native account configured; provider validity not checked",
        ),
        C::Missing => (
            CheckStatus::Fail,
            "native account missing and authentication required",
        ),
        C::NotRequired => (
            CheckStatus::Ok,
            "native backend reports OpenAI authentication not required; other provider validity not checked",
        ),
        C::Unknown => (
            CheckStatus::Warn,
            "credential configuration unknown; local endpoint access does not prove provider login",
        ),
    };
    let mut out = vec![check(
        &name,
        status,
        format!("{scope}; {detail}"),
        (status != CheckStatus::Ok).then(|| fix.clone().unwrap()),
    )];
    for provider in runtime.provider_observations {
        let status = match provider.outcome {
            P::Succeeded => CheckStatus::Ok,
            P::AuthenticationRejected | P::AccessDenied => CheckStatus::Fail,
            _ => CheckStatus::Warn,
        };
        out.push(check(&format!("provider/{}/{}/{}", instance.instance_id, provider.provider, provider.model), status,
            format!("{scope}; latest completed result in native history page at {}: {:?}; historical request evidence, not current token validity", provider.completed_ms, provider.outcome),
            (status != CheckStatus::Ok).then(|| match provider.outcome {
                P::AuthenticationRejected => "repair this provider's login in the instance terminal; a later successful request for the same provider/model supersedes this failure",
                P::AccessDenied => "check provider/model access; HTTP 403 is not proof of invalid login",
                P::RateLimited => "check provider quota/rate limits; do not replace credentials solely because of HTTP 429",
                _ => "inspect this instance's latest provider error in its terminal",
            }.into())));
    }
    out.extend(runtime.capabilities.into_iter().map(|mut c| {
        c.check = format!("capability/{}/{}", instance.instance_id, c.check);
        c.detail = format!("{scope}; {}", c.detail);
        c
    }));
    out
}

fn setup(home: &Path, client: &Client) -> Check {
    let unknown = || {
        check(
            "authentication/github",
            CheckStatus::Warn,
            "daemon GitHub scope/authentication observation unavailable".into(),
            Some("start or upgrade the daemon, then rerun doctor".into()),
        )
    };
    if client.daemon().selected < V1_9 {
        return unknown();
    }
    let request = ClientRequest::Operator {
        data: OperatorData {
            request_id: "doctor:setup".into(),
            command: OperatorCommand::SetupDiagnostic,
        },
    };
    let response = agend_client::exchange_once(
        &home.join(agend_core::protocol::client::DAEMON_SOCKET),
        None,
        V1_9,
        &request,
        Instant::now() + Duration::from_secs(8),
    );
    match response {
        Ok(ClientResponse::CommandResult { data }) => match data.result {
            CommandResult::SetupDiagnostic { data }
                if Some(data.boot_id) == client.daemon().boot_id =>
            {
                let mut check = data.github;
                check.detail = format!(
                    "daemon boot={} observed_ms={}; {}",
                    data.boot_id, data.observed_ms, check.detail
                );
                check
            }
            _ => unknown(),
        },
        _ => unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::{
        model::Backend,
        protocol::client::AgentState,
        runtime_records::{Instance, InstanceStatus},
        setup::backend::observation::BackendDiagnosticReply,
    };
    use agend_testkit::{block_on, tempdir::TempDir};

    #[test]
    fn persisted_snapshot_cannot_cross_boot_or_configuration_boundaries() {
        let root = TempDir::new("doctor-snapshot-boundary").unwrap();
        let store = agend_daemon::store::SqliteStore::open(root.path(), 0).unwrap();
        let instance = Instance {
            id: "diagnostic".into(),
            backend: Backend::Codex,
            program: "/bin/echo".into(),
            args: vec![],
            working_directory: root.path().display().to_string(),
            session_id: None,
            status: InstanceStatus::Failed,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        };
        block_on(store.add_instance(&instance)).unwrap();
        let reply = BackendDiagnosticReply {
            boot_id: 101,
            runtime: None,
            policies: vec![BackendCapabilityPolicy {
                capability_id: "must-not-cross-boundary".into(),
                policy_kind: agend_core::setup::backend::observation::CapabilityPolicyKind::Unknown,
                version_constraint: "unknown".into(),
                additional_requirements: "unknown".into(),
                evidence_scope: "adversarial sentinel".into(),
            }],
            snapshot: block_on(store.backend_diagnostic(&instance.id)).unwrap(),
        };
        let mut view = InstanceView {
            instance_id: instance.id,
            backend: "codex".into(),
            program: Some(instance.program),
            working_directory: Some(instance.working_directory),
            team_id: "general".into(),
            state: AgentState::Unknown,
        };
        let (detail, policies, _) = describe(reply.clone(), &view, 101).unwrap();
        assert_eq!(policies.len(), 1);
        assert!(detail.contains("no matching external version observation"));
        assert!(detail.contains("disk evidence does not establish authentication"));
        assert!(
            describe(reply.clone(), &view, 100)
                .unwrap_err()
                .contains("daemon restarted")
        );
        view.program = Some("/different/program".into());
        assert!(
            describe(reply, &view, 101)
                .unwrap_err()
                .contains("configuration changed")
        );
    }
}
