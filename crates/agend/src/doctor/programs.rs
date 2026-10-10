//! Diagnose configured programs without pretending the operator PATH is the daemon PATH.
use super::{check, ok};
use agend_core::{
    protocol::client::InstanceView,
    setup::{Check, CheckStatus},
};
use std::{cell::OnceCell, path::Path};

pub(super) fn checks(home: &Path, instances: &[InstanceView]) -> Vec<Check> {
    let build = OnceCell::new();
    instances
        .iter()
        .flat_map(|instance| {
            let mut evidence = check(
                &format!("compatibility/{}", instance.instance_id),
                CheckStatus::Warn,
                "unknown; no verified canary evidence for this configured program".into(),
                Some("import a fixed backend and run an explicitly authorized canary with dedicated test credentials".into()),
            );
            let executable = diagnose(home, instance, &build, &mut evidence);
            [executable, evidence]
        })
        .collect()
}

fn diagnose(
    home: &Path,
    instance: &InstanceView,
    build: &OnceCell<Result<String, String>>,
    evidence: &mut Check,
) -> Check {
    let name = format!("backend/{}", instance.instance_id);
    let warn = |detail: String| {
        check(&name, CheckStatus::Warn, detail,
        Some("inspect this instance's configured program; import a fixed native version, run backend canary, then prepare backend switch".into()))
    };
    let Some(configured) = &instance.program else {
        return warn(
            "daemon did not report the configured program; upgrade the daemon to diagnose it"
                .into(),
        );
    };
    let requested = Path::new(configured);
    let selected = if requested.is_absolute() {
        requested.to_owned()
    } else if configured.starts_with("./") || configured.starts_with("../") {
        match instance
            .working_directory
            .as_deref()
            .map(Path::new)
            .filter(|p| p.is_absolute())
        {
            Some(cwd) => cwd.join(requested),
            None => {
                return warn(
                    "relative program has no absolute working directory; resolution unknown".into(),
                );
            }
        }
    } else {
        return warn(format!(
            "configured program {configured:?} requires daemon PATH resolution; operator PATH result does not prove this instance's executable"
        ));
    };
    // inspect_launch hashes managed bytes before any execution. A changed managed
    // program must never be executed just to diagnose its reported version.
    let inspected = agend_daemon::backend_versions::inspect_launch(
        home,
        &instance.backend,
        selected.to_str().unwrap_or(""),
        home,
        "",
    );
    let fail = |detail: String| {
        check(&name, CheckStatus::Fail, detail,
        Some("preserve the running holder; inspect the imported version and use backend switch to a verified version".into()))
    };
    match inspected {
        Err(error) => fail(error),
        Ok(Some(artifact)) => {
            let digest = build.get_or_init(|| std::env::current_exe().map_err(|e| e.to_string())
                .and_then(|exe| agend_daemon::backend_versions::fingerprint(&exe)));
            let verified = digest.as_ref().map_err(Clone::clone).and_then(|digest|
                agend_daemon::backend_versions::verify_canary(home, &instance.backend, &artifact.version, digest));
            evidence.detail = match verified {
                Ok(report) => {
                    evidence.fix = Some("review this historical scope against the running daemon and required capabilities; doctor has no live authentication producer".into());
                    format!(
                    "historical canary verified for this CLI build: backend={}, version={}, artifact_sha256={}, agend_sha256={}, platform={}/{}, started_at_unix_ms={}, elapsed_ms={}, requested_model={:?}; running daemon build, current login and other capabilities are not verified",
                    report.artifact.backend, report.artifact.version, report.artifact.sha256,
                    report.agend_sha256, report.os, report.arch, report.started_at_unix_ms,
                    report.elapsed_ms, report.model
                )
                },
                Err(error) => format!("unknown; canary not verified for current CLI build: {error}"),
            };
            ok(&name, format!("{}: imported bytes unchanged; executable integrity only", selected.display()))
        }
        Ok(None) => match crate::setup::version_line(&selected) {
            Ok(version) => warn(format!("{}: {version}; version probe only; unmanaged executable, no pinned bytes", selected.display())),
            Err(error) => check(&name, CheckStatus::Fail, format!("{}: {error}", selected.display()),
                Some("restore the configured executable or switch this instance to a verified imported backend".into())),
        },
    }
}
