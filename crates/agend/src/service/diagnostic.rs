//! Read-only service observations: no installation lock creation or manager mutation.
use super::{
    Plan, files, lifecycle,
    manager::{Native, ServiceManager, State},
};
use agend_core::setup::{Check, CheckStatus, service::InstallPhase};
use std::path::Path;

fn result(status: CheckStatus, detail: impl Into<String>, fix: Option<&str>) -> Check {
    Check {
        check: "service".into(),
        status,
        detail: detail.into(),
        fix: fix.map(str::to_owned),
    }
}

pub(super) fn check(home: &Path) -> Check {
    let observed = (|| {
        if !files::existing_directory(&home.join("service"))? {
            return Ok(result(
                CheckStatus::Ok,
                "not installed; foreground daemon is supported",
                None,
            ));
        }
        let plan = super::installation_plan().map_err(|e| format!("{e:?}"))?;
        observe(&plan, &Native)
    })();
    observed.unwrap_or_else(|reason: String| result(CheckStatus::Fail, reason, Some("agend service status; inspect the installation receipt and owned files before repairing")))
}

pub(super) fn observe(plan: &Plan, manager: &impl ServiceManager) -> Result<Check, String> {
    let dir = Path::new(&plan.spec.home).join("service");
    let Some(record) = files::load(&dir)? else {
        return Ok(result(
            CheckStatus::Warn,
            "service directory exists without an installation receipt",
            Some("agend service status; inspect unowned files before installing"),
        ));
    };
    lifecycle::validate(&record, plan)?;
    if !files::matches_program(&record)?
        || !files::matches_bytes(
            Path::new(&record.service_path),
            record.spec.render()?.as_bytes(),
        )?
    {
        return Err("owned service executable or definition is missing".into());
    }
    let state = manager.inspect(&record)?;
    // Recheck the durable observation after querying the external manager.
    if files::load(&dir)?.as_ref() != Some(&record) {
        return Err("installation changed during diagnosis; run agend doctor again".into());
    }
    if record.phase != InstallPhase::Registered {
        return Ok(result(
            CheckStatus::Warn,
            format!("installation {:?}; manager {state:?}", record.phase),
            Some(if record.phase == InstallPhase::Removing {
                "agend uninstall"
            } else {
                "agend service install"
            }),
        ));
    }
    Ok(match state {
        State::Owned { running: true } => result(
            CheckStatus::Ok,
            "owned service running; daemon readiness is checked separately",
            None,
        ),
        _ => result(
            CheckStatus::Warn,
            format!("owned installation is not running: {state:?}"),
            Some("agend service status; agend service install"),
        ),
    })
}
