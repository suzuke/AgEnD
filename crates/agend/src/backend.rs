//! Operator-only backend import, verification, and explicit isolated canary.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use agend_core::model::Backend;
use agend_core::setup::backend::{ImportedBackend, valid_version};
use clap::Subcommand;
use serde_json::json;

use crate::cli::{Failure, Output};
use crate::service::files;
mod canary;
mod switch;

#[derive(Subcommand)]
pub enum Command {
    /// Read the public registry's latest tag (no installation or model execution)
    Latest {
        #[arg(value_parser = ["claude", "codex", "opencode"])]
        backend: String,
    },
    /// Prepare, inspect or cancel a backend version switch through the daemon
    #[command(subcommand)]
    Switch(switch::Command),
    /// Run a bounded, three-message model canary in an isolated daemon home
    Canary {
        #[arg(value_parser = ["claude", "codex", "opencode"])]
        backend: String,
        #[arg(long)]
        version: String,
        /// Explicitly allow backend execution and three model messages
        #[arg(long)]
        allow_model: bool,
        #[arg(long, default_value_t = 180)]
        timeout_seconds: u64,
        /// Explicit model; OpenCode requires provider/model
        #[arg(long)]
        model: Option<String>,
        /// Dedicated credential file: Codex/OpenCode auth JSON or Claude OAuth token
        #[arg(long)]
        auth_file: Option<PathBuf>,
    },
    /// Copy a native executable into an isolated, unverified version directory
    Import {
        #[arg(value_parser = ["claude", "codex", "opencode"])]
        backend: String,
        #[arg(long)]
        version: String,
        /// Native binary, not an npm/shell wrapper; the source is never changed
        #[arg(long)]
        program: PathBuf,
    },
    /// Verify the stored bytes of an imported version (does not run a canary)
    Inspect {
        #[arg(value_parser = ["claude", "codex", "opencode"])]
        backend: String,
        #[arg(long)]
        version: String,
    },
}

fn failed(message: impl Into<String>) -> Failure {
    Failure::new("backend_install_failed", message)
}

pub fn run(command: Command) -> Result<Output, Failure> {
    if let Command::Switch(command) = command {
        return switch::run(command);
    }
    if std::env::var_os("AGEND_INSTANCE").is_some() {
        return Err(Failure::usage(
            "backend installation requires an operator terminal",
        ));
    }
    if let Command::Latest { backend } = &command {
        let backend =
            Backend::parse(backend).ok_or_else(|| Failure::usage("unsupported backend"))?;
        let release = agend_daemon::backend_versions::registry::latest(backend)
            .map_err(|reason| Failure::new("backend_registry_failed", reason))?;
        return Ok(Output::new(
            vec![format!(
                "{}: registry latest {}; not installed or canary-verified",
                backend.as_str(),
                release.version
            )],
            json!({"release":release,"source":"https://registry.npmjs.org","installed":false,"canary_verified":false}),
        ));
    }
    let home = crate::home::resolve()?;
    let (backend, version) = match &command {
        Command::Switch(_) | Command::Latest { .. } => unreachable!("handled before local import"),
        Command::Import {
            backend, version, ..
        }
        | Command::Inspect { backend, version }
        | Command::Canary {
            backend, version, ..
        } => (backend, version),
    };
    if Backend::parse(backend).is_none() || !valid_version(version) {
        return Err(Failure::usage(
            "backend version must be 1-80 ASCII letters/digits or .-_+, starting with a letter/digit",
        ));
    }
    if let Command::Canary {
        backend,
        version,
        allow_model,
        timeout_seconds,
        model,
        auth_file,
    } = command
    {
        if !allow_model {
            return Err(Failure::usage(
                "canary executes the backend and three model messages; use --allow-model to proceed",
            ));
        }
        let report = canary::run(
            &home,
            &backend,
            &version,
            timeout_seconds,
            model.as_deref(),
            auth_file.as_deref(),
        )
        .map_err(failed)?;
        if !report.passed {
            return Err(failed(format!(
                "canary failed: {}; report: {}",
                report.error.as_deref().unwrap_or("unknown failure"),
                home.join("backends")
                    .join(&backend)
                    .join(&version)
                    .join("canary.json")
                    .display()
            )));
        }
        return Ok(Output::new(vec!["canary passed; three confirmed deliveries; owned temporary home removed; fleet unchanged".into()], serde_json::to_value(report).map_err(|e| failed(e.to_string()))?));
    }
    if let Command::Inspect { backend, version } = &command {
        return inspect_output(&home, backend, version).map_err(failed);
    }
    let root = home.join("backends");
    let parent = root.join(backend);
    let dir = parent.join(version);
    let record = match command {
        Command::Switch(_) | Command::Latest { .. } => unreachable!("handled before local import"),
        Command::Canary { .. } => unreachable!("handled before import"),
        Command::Import {
            backend,
            version,
            program,
        } => import(&home, &root, &parent, &dir, &backend, &version, &program).map_err(failed)?,
        Command::Inspect { backend, version } => {
            agend_daemon::backend_versions::inspect(&home, &backend, &version).map_err(failed)?
        }
    };
    Ok(Output::new(
        vec![
            format!(
                "{} {} imported; bytes verified; canary not run",
                record.backend, record.version
            ),
            format!("isolated executable: {}", dir.join("program").display()),
            "fleet unchanged; import does not authorize activation".into(),
        ],
        json!({"import":record,"program":dir.join("program"),"canary":"not_run","active":false}),
    ))
}

fn inspect_output(home: &Path, backend: &str, version: &str) -> Result<Output, String> {
    let artifact = agend_daemon::backend_versions::inspect(home, backend, version)?;
    let dir = home.join("backends").join(backend).join(version);
    let missing = fs::symlink_metadata(dir.join("canary.json"))
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
    let (state, reason) = if missing {
        ("not_run", None)
    } else {
        let checked = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| agend_daemon::backend_versions::fingerprint(&exe))
            .and_then(|digest| {
                agend_daemon::backend_versions::verify_canary(home, backend, version, &digest)
            });
        match checked {
            Ok(_) => ("passed", None),
            Err(error) => ("invalid", Some(error)),
        }
    };
    Ok(Output::new(
        vec![
            format!("{backend} {version}: bytes verified; canary {state}"),
            "fleet unchanged; verified evidence does not activate a version".into(),
        ],
        json!({"import":artifact,"program":dir.join("program"),"canary":state,"canary_reason":reason,"active":false}),
    ))
}

fn import(
    home: &Path,
    root: &Path,
    parent: &Path,
    dir: &Path,
    backend: &str,
    version: &str,
    source: &Path,
) -> Result<ImportedBackend, String> {
    if !source.is_absolute() {
        return Err("source program must be an absolute native executable path".into());
    }
    // Resolve an operator's explicit entrypoint, but never change its target.
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&source)
        .map_err(|e| e.to_string())?;
    let initial = input.metadata().map_err(|e| e.to_string())?;
    if !initial.is_file() || initial.mode() & 0o111 == 0 || initial.len() > 1024 * 1024 * 1024 {
        return Err("source must be a native executable no larger than 1 GiB".into());
    }
    let mut magic = [0u8; 4];
    input.read_exact(&mut magic).map_err(|e| e.to_string())?;
    if ![
        b"\x7fELF",
        b"\xcf\xfa\xed\xfe",
        b"\xfe\xed\xfa\xcf",
        b"\xca\xfe\xba\xbe",
        b"\xbe\xba\xfe\xca",
    ]
    .contains(&&magic)
    {
        return Err("source is not a supported native binary; import the native backend executable, not its shell/npm wrapper".into());
    }
    files::private_dir(home)?;
    let _activity = agend_daemon::store::maintenance::Activity::acquire(home)
        .map_err(|e| format!("home is being maintained; retry after uninstall finishes: {e}"))?;
    for component in [root, parent] {
        files::private_dir(component)?;
    }
    let _lock = files::lock(root)?;
    if fs::symlink_metadata(dir).is_ok() {
        return Err("version directory already exists; preserving it (use backend inspect)".into());
    }
    // Exclusive directory creation prevents adoption of any preexisting entry.
    fs::DirBuilder::new()
        .mode(0o700)
        .create(dir)
        .map_err(|e| e.to_string())?;
    let owned_inode = fs::symlink_metadata(dir).map_err(|e| e.to_string())?.ino();
    let result = (|| {
        use std::io::{Seek, SeekFrom};
        input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        files::publish(&dir.join("program"), 0o500, false, |out| {
            let copied = std::io::copy(
                &mut Read::by_ref(&mut input).take(1024 * 1024 * 1024 + 1),
                out,
            )?;
            if copied > 1024 * 1024 * 1024 {
                return Err(std::io::Error::other("source exceeded 1 GiB during import"));
            }
            Ok(())
        })?;
        let digest = files::sha256(File::open(dir.join("program")).map_err(|e| e.to_string())?)?;
        input.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let source_digest = files::sha256(Read::by_ref(&mut input).take(1024 * 1024 * 1024 + 1))?;
        let after = input.metadata().map_err(|e| e.to_string())?;
        let current = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
        if digest != source_digest
            || (
                initial.dev(),
                initial.ino(),
                initial.len(),
                initial.mtime(),
                initial.mtime_nsec(),
            ) != (
                after.dev(),
                after.ino(),
                after.len(),
                after.mtime(),
                after.mtime_nsec(),
            )
            || (current.dev(), current.ino()) != (after.dev(), after.ino())
        {
            return Err("source changed during import; no version published".into());
        }
        let record = ImportedBackend {
            format: 1,
            backend: backend.into(),
            version: version.into(),
            sha256: digest,
            bytes: after.len(),
        };
        let bytes = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
        files::publish(&dir.join("import.json"), 0o600, false, |out| {
            out.write_all(&bytes)
        })?;
        Ok(record)
    })();
    if result.is_err()
        && fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir() && meta.ino() == owned_inode)
    {
        fs::remove_dir_all(dir).map_err(|e| format!("import failed; cleanup incomplete: {e}"))?;
    }
    result
}
