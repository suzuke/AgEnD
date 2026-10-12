//! `agend init`: resolves the home, refuses v1, creates a private directory
//! and publishes initial configuration without replacing an existing file.
//! Runs doctor and prints the next steps; safe to run again.
//!
//! Not here: instances (only the daemon writes them: `agend instance add`), the
//! `general` team (built in, D12), service registration (gate 13).
//!
//! Must NOT: modify the user's own git or PATH, or start the daemon.

use std::fs::DirBuilder;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

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
    initial_config(&home).map_err(|e| Failure::new("init_failed", e))?;
    let mut out = doctor::report(&doctor::checks(&home));
    out.lines.insert(0, first);
    out.lines.push(
        "next: agend daemon   (then, in another terminal) agend instance add dev-1 claude".into(),
    );
    Ok(out)
}

fn existing_config(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(true),
        Ok(_) => Err("config.toml must be a regular file, not a symlink or directory".into()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("cannot inspect config.toml: {e}")),
    }
}

/// Publish a complete file exclusively. A competing init or user edit wins;
/// never truncate it or follow an existing configuration symlink.
fn initial_config(home: &Path) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = home.join("config.toml");
    if existing_config(&path)? {
        return Ok(());
    }
    for _ in 0..16 {
        let temp = home.join(format!(
            ".config-init-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
        {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot create initial configuration: {e}")),
        };
        let result = (|| {
            file.write_all(agend_core::setup::INITIAL_CONFIG.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|e| format!("cannot write initial configuration: {e}"))?;
            match std::fs::hard_link(&temp, &path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    if existing_config(&path)? {
                        Ok(())
                    } else {
                        Err("config.toml changed during init; run agend init again".into())
                    }
                }
                Err(e) => Err(format!("cannot publish initial configuration: {e}")),
            }
        })();
        drop(file);
        std::fs::remove_file(&temp)
            .map_err(|e| format!("cannot remove initial configuration temporary file: {e}"))?;
        return result;
    }
    Err("cannot reserve an initial configuration temporary file".into())
}
