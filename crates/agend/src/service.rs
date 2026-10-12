//! Reviewable user-service plan. Applying it is a separate installation step.
//! Keep the service's executable under its home, independent of build targets.

mod diagnostic;
pub(crate) mod files;
mod lifecycle;
mod manager;
mod purge;
#[cfg(test)]
mod tests;

use manager::ServiceManager;
use std::path::{Path, PathBuf};

use agend_core::setup::service::{Manager, ServiceSpec};
use serde_json::json;

use crate::cli::{Failure, Output};

#[derive(Clone)]
pub struct Plan {
    pub spec: ServiceSpec,
    pub service_path: PathBuf,
    pub source: PathBuf,
}

fn plan_data(manager: Option<&str>) -> Result<Plan, Failure> {
    let manager = match manager {
        Some("launchd") => Manager::Launchd,
        Some("systemd") => Manager::Systemd,
        None if cfg!(target_os = "macos") => Manager::Launchd,
        None if cfg!(target_os = "linux") => Manager::Systemd,
        _ => return Err(Failure::usage("user services require macOS or Linux")),
    };
    let home = crate::home::resolve()?;
    let user_home = absolute_env("HOME")?;
    let service_path = match manager {
        Manager::Launchd => user_home.join("Library/LaunchAgents"),
        Manager::Systemd => match std::env::var_os("XDG_CONFIG_HOME") {
            Some(_) => absolute_env("XDG_CONFIG_HOME")?,
            None => user_home.join(".config"),
        }
        .join("systemd/user"),
    }
    .join(manager.filename());
    let program = home.join("service/agend");
    let source = std::env::current_exe()
        .map_err(|e| Failure::new("service_plan_failed", format!("current executable: {e}")))?;
    let spec = ServiceSpec {
        manager,
        program: utf8(&program)?,
        home: utf8(&home)?,
        user_home: utf8(&user_home)?,
        search_path: std::env::var("PATH")
            .map_err(|_| Failure::usage("service PATH must be set and valid UTF-8"))?,
    };
    spec.render().map_err(Failure::usage)?;
    Ok(Plan {
        spec,
        service_path,
        source,
    })
}

pub fn plan(manager: Option<&str>) -> Result<Output, Failure> {
    let Plan {
        spec,
        service_path,
        source,
    } = plan_data(manager)?;
    let program = Path::new(&spec.program);
    let definition = spec.render().map_err(Failure::usage)?;
    Ok(Output::new(
        vec![
            format!("service file: {}", service_path.display()),
            format!("managed executable: {}", program.display()),
            format!("copy source: {}", source.display()),
            "preview only; no files written or services changed".into(),
            definition.clone(),
        ],
        json!({
            "preview": true,
            "spec": spec,
            "service_path": service_path,
            "source_program": source,
            "definition": definition,
        }),
    ))
}

fn absolute_env(name: &str) -> Result<PathBuf, Failure> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty() && Path::new(value).is_absolute())
        .map(PathBuf::from)
        .ok_or_else(|| Failure::usage(format!("{name} must be a nonempty absolute path")))
}

fn utf8(path: &Path) -> Result<String, Failure> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Failure::usage("service paths must be valid UTF-8"))
}

fn installation_plan() -> Result<Plan, Failure> {
    if std::env::var_os("AGEND_INSTANCE").is_some() {
        return Err(Failure::usage(
            "service installation is an operator setup operation; use an operator terminal",
        ));
    }
    let mut plan = plan_data(None)?;
    plan.spec.home = utf8(&files::canonical_location(Path::new(&plan.spec.home)).map_err(failed)?)?;
    plan.spec.user_home =
        utf8(&files::canonical_location(Path::new(&plan.spec.user_home)).map_err(failed)?)?;
    plan.spec.program = utf8(&Path::new(&plan.spec.home).join("service/agend"))?;
    plan.service_path = files::canonical_location(&plan.service_path).map_err(failed)?;
    if utf8(&plan.service_path)?.chars().any(char::is_control) {
        return Err(Failure::usage(
            "service file path must contain no control characters",
        ));
    }
    plan.spec.render().map_err(Failure::usage)?;
    Ok(plan)
}

fn failed(message: impl Into<String>) -> Failure {
    Failure::new("service_failed", message)
}

pub fn install(no_start: bool) -> Result<Output, Failure> {
    let plan = installation_plan()?;
    let native = manager::Native;
    let backend: Option<&dyn ServiceManager> = if no_start { None } else { Some(&native) };
    let mut owned = lifecycle::Owned::prepare(&plan, &plan.source, backend).map_err(failed)?;
    if !no_start {
        owned.register(&native).map_err(failed)?;
    }
    let phase = if no_start { "prepared" } else { "registered" };
    Ok(Output::new(
        vec![
            format!("service {phase}: {}", owned.record.service_path),
            "run agend doctor to check daemon readiness".into(),
        ],
        json!({"phase":phase,"installation":owned.record}),
    ))
}

pub fn status() -> Result<Output, Failure> {
    let plan = installation_plan()?;
    let Some(owned) = lifecycle::Owned::open(&plan).map_err(failed)? else {
        return Ok(Output::new(
            vec!["no owned service installation".into()],
            json!({"installed":false}),
        ));
    };
    let state = manager::Native.inspect(&owned.record).map_err(failed)?;
    Ok(Output::new(
        vec![format!("service: {state:?}")],
        json!({"installed":true,"state":format!("{state:?}"),"installation":owned.record}),
    ))
}

pub(crate) fn diagnostic(home: &Path) -> agend_core::setup::Check {
    diagnostic::check(home)
}

pub fn uninstall(delete_data: bool, confirm_home: Option<&str>) -> Result<Output, Failure> {
    let plan = installation_plan()?;
    if delete_data && confirm_home != Some(plan.spec.home.as_str()) {
        return Err(Failure::usage(format!(
            "deleting all data is irreversible; confirm this exact canonical home with --delete-data --confirm-home {:?}",
            plan.spec.home
        )));
    }
    let Some(mut owned) = lifecycle::Owned::open(&plan).map_err(failed)? else {
        if delete_data {
            return Err(failed(
                "no owned installation receipt; refusing data deletion",
            ));
        }
        return Ok(Output::new(
            vec!["no owned service installation; data retained".into()],
            json!({"installed":false,"data_retained":true}),
        ));
    };
    owned
        .remove_with_data(&manager::Native, delete_data)
        .map_err(failed)?;
    Ok(Output::new(
        vec![
            "owned service, executable and shims removed".into(),
            format!(
                "data {}: {}",
                if delete_data {
                    "deleted (empty lock files retained)"
                } else {
                    "retained"
                },
                plan.spec.home
            ),
        ],
        json!({"installed":false,"data_retained":!delete_data,"home":plan.spec.home}),
    ))
}
