//! Where AgEnD keeps its data (gate 9 P3): `AGEND_HOME`, which must always
//! be set (there is no default before gate 13) and absolute. A directory
//! with v1's `fleet.yaml` in it is refused, so v2 never writes into a v1
//! home. The CLI, `agend daemon`, `agend doctor` and `agend init` all go
//! through [`resolve`].
//!
//! Must NOT: create or change anything.

use std::path::{Path, PathBuf};

use crate::cli::Failure;

/// The v1 file that marks a v1 home.
pub const V1_MARKER: &str = "fleet.yaml";

/// `AGEND_HOME` when it is set and absolute (it may not exist yet).
pub fn from_env() -> Result<PathBuf, Failure> {
    match std::env::var_os("AGEND_HOME") {
        None => Err(Failure::usage(
            "AGEND_HOME is not set; choose a directory for AgEnD's data and run: export AGEND_HOME=<absolute path>",
        )),
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
