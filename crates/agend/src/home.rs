//! Where AgEnD keeps its data: explicit `AGEND_HOME`, or `$HOME/.agend`
//! for an operator. Agents must retain their explicit daemon home. A directory
//! with v1's `fleet.yaml` in it is refused, so v2 never writes into a v1
//! home. The CLI, `agend daemon`, `agend doctor` and `agend init` all go
//! through [`resolve`].
//!
//! Must NOT: create or change anything.

use std::path::{Path, PathBuf};

use crate::cli::Failure;

/// The v1 file that marks a v1 home.
pub const V1_MARKER: &str = "fleet.yaml";

/// Resolve an absolute home without reading configuration or changing files.
pub fn from_env() -> Result<PathBuf, Failure> {
    match std::env::var_os("AGEND_HOME") {
        None if std::env::var_os("AGEND_INSTANCE").is_some() => Err(Failure::usage(
            "AGEND_HOME is not set for this agent; restore its daemon-provided environment",
        )),
        None => match std::env::var_os("HOME") {
            Some(home) if !home.is_empty() && Path::new(&home).is_absolute() => {
                Ok(PathBuf::from(home).join(agend_core::setup::DEFAULT_HOME_DIRECTORY))
            }
            _ => Err(Failure::usage(
                "AGEND_HOME is not set and HOME is not an absolute path; run: export AGEND_HOME=<absolute path>",
            )),
        },
        Some(home) if home.is_empty() => Err(Failure::usage(
            "AGEND_HOME is empty; choose a directory for AgEnD's data and run: export AGEND_HOME=<absolute path>",
        )),
        Some(home) if !Path::new(&home).is_absolute() => Err(Failure::usage(format!(
            "AGEND_HOME must be an absolute path, got {}",
            Path::new(&home).display()
        ))),
        Some(home) => Ok(PathBuf::from(home)),
    }
}

/// Whether `home` looks like an AgEnD v1 home.
pub fn is_v1(home: &Path) -> bool {
    home.join(V1_MARKER).exists()
}

/// [`from_env`], refusing a v1 home.
pub fn resolve() -> Result<PathBuf, Failure> {
    let home = from_env()?;
    if is_v1(&home) {
        return Err(Failure::new(
            "v1_home",
            format!(
                "{} looks like an AgEnD v1 home ({V1_MARKER}); set AGEND_HOME to another directory",
                home.display()
            ),
        ));
    }
    Ok(home)
}
