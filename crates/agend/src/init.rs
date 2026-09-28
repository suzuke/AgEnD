//! `agend init` (gate 9 P9): takes `AGEND_HOME` (it must be set; a v1 home
//! is refused), creates it 0700 unless it exists, runs `agend doctor`, and
//! prints the next steps. It asks nothing and can run again.
//!
//! Not here (P9): `config.toml` (the first gate that reads it creates it),
//! instances (only the daemon writes them: `agend instance add`), the
//! `general` team (built in, D12), service registration (gate 13).
//!
//! Must NOT: modify the user's own git or PATH, or start the daemon.

use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;

use crate::cli::{Failure, Output};
use crate::{doctor, home};

pub fn run() -> Result<Output, Failure> {
    let home = home::resolve()?;
    let first = if home.exists() {
        format!("{} already exists", home.display())
    } else {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&home)
            .map_err(|e| {
                Failure::new(
                    "init_failed",
                    format!("cannot create {}: {e}", home.display()),
                )
            })?;
        format!("created {} (0700)", home.display())
    };
    let mut out = doctor::report(&doctor::checks(&home));
    out.lines.insert(0, first);
    out.lines.push(
        "next: agend daemon   (then, in another terminal) agend instance add dev-1 claude".into(),
    );
    Ok(out)
}
