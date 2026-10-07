//! Reviewable user-service plan. Applying it is a separate installation step.
//! Keep the service's executable under its home, independent of build targets.

use std::path::{Path, PathBuf};

use agend_core::setup::service::{Manager, ServiceSpec};
use serde_json::json;

use crate::cli::{Failure, Output};

pub fn plan(manager: Option<&str>) -> Result<Output, Failure> {
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
