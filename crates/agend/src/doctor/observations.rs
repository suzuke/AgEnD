//! Persisted evidence from the connected daemon; never starts a probe.
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
    let mut out = Vec::new();
    for instance in instances {
        let (detail, policies) =
            read(home, client, instance).unwrap_or_else(|e| (format!("unknown; {e}"), vec![]));
        out.push(check(&format!("observation/{}", instance.instance_id), CheckStatus::Warn, detail,
            Some("compare the recorded scope and attempt time with the current configuration; disk observations and launch reservations do not prove the running image or login".into())));
        for policy in policies {
            out.push(check(&format!("capability/{}/{}", instance.instance_id, policy.capability_id),
                CheckStatus::Warn,
                format!("daemon policy {:?}: {}; requires {}; evidence scope: {}; runtime eligibility unknown",
                    policy.policy_kind, policy.version_constraint, policy.additional_requirements, policy.evidence_scope),
                Some("check this capability's own runtime prerequisites; a matching disk version or successful canary does not establish eligibility".into())));
        }
    }
    out
}

fn read(
    home: &Path,
    client: &Client,
    instance: &InstanceView,
) -> Result<(String, Vec<BackendCapabilityPolicy>), String> {
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
        Instant::now() + Duration::from_secs(3),
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
) -> Result<(String, Vec<BackendCapabilityPolicy>), String> {
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
    parts.push("live authentication and running daemon binary digest unknown".into());
    Ok((parts.join("; "), reply.policies))
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
        let (detail, policies) = describe(reply.clone(), &view, 101).unwrap();
        assert_eq!(policies.len(), 1);
        assert!(detail.contains("no matching external version observation"));
        assert!(detail.contains("live authentication and running daemon binary digest unknown"));
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
