//! Installation reconciliation. A durable receipt precedes publication and
//! registration; removal leaves data and foreign artifacts intact.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agend_core::setup::service::{InstallPhase, Installation};

use super::Plan;
use super::files;
use super::manager::{ServiceManager, State};

pub struct Owned {
    pub record: Installation,
    dir: PathBuf,
    source: PathBuf,
    _lock: File,
}

impl Owned {
    pub fn prepare(
        plan: &Plan,
        source: &Path,
        manager: Option<&dyn ServiceManager>,
    ) -> Result<Self, String> {
        let home = Path::new(&plan.spec.home);
        if home.parent().is_none() || home == Path::new(&plan.spec.user_home) {
            return Err("choose a dedicated AgEnD home, not / or your user home".into());
        }
        files::private_dir(home)?;
        let dir = home.join("service");
        let lock = files::lock(&dir)?;
        let existing = files::load(&dir)?;
        let hash = files::sha256(File::open(source).map_err(|e| e.to_string())?)?;
        let record = if let Some(record) = existing {
            validate(&record, plan)?;
            if record.phase == InstallPhase::Removing {
                return Err("an uninstall is incomplete; finish agend uninstall first".into());
            }
            if record.program_sha256 != hash {
                return Err("another binary version is installed; uninstall its service before installing this version (data is retained)".into());
            }
            record
        } else {
            if fs::symlink_metadata(&plan.service_path).is_ok()
                || fs::symlink_metadata(&plan.spec.program).is_ok()
            {
                return Err("service file or managed executable already exists without an installation receipt; preserving it".into());
            }
            let record = Installation {
                version: 1,
                spec: plan.spec.clone(),
                service_path: plan.service_path.to_string_lossy().into_owned(),
                program_sha256: hash,
                phase: InstallPhase::Prepared,
            };
            if let Some(manager) = manager
                && manager.inspect(&record)? != State::Absent
            {
                return Err(
                    "service label exists without this installation receipt; preserving it".into(),
                );
            }
            files::save(&dir, &record, false)?;
            record
        };
        let owned = Self {
            record,
            dir,
            source: plan.source.clone(),
            _lock: lock,
        };
        owned.check_artifacts()?;
        if !files::matches_program(&owned.record)? {
            files::publish(Path::new(&owned.record.spec.program), 0o700, false, |out| {
                let mut input = File::open(source)?;
                std::io::copy(&mut input, out).map(|_| ())
            })?;
            files::matches_program(&owned.record)?;
        }
        let path = Path::new(&owned.record.service_path);
        let definition = owned.record.spec.render()?.into_bytes();
        files::private_dir(path.parent().ok_or("service path has no parent")?)?;
        files::private_dir(&home.join("logs"))?;
        if !files::matches_bytes(path, &definition)? {
            files::publish(path, 0o600, false, |file| file.write_all(&definition))?;
        }
        Ok(owned)
    }

    pub fn open(plan: &Plan) -> Result<Option<Self>, String> {
        let dir = Path::new(&plan.spec.home).join("service");
        if fs::symlink_metadata(&dir).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
            return Ok(None);
        }
        let lock = files::lock(&dir)?;
        let Some(record) = files::load(&dir)? else {
            return Ok(None);
        };
        validate(&record, plan)?;
        let value = Self {
            record,
            dir,
            source: plan.source.clone(),
            _lock: lock,
        };
        value.check_artifacts()?;
        Ok(Some(value))
    }

    fn check_artifacts(&self) -> Result<(), String> {
        files::matches_program(&self.record)?;
        files::matches_bytes(
            Path::new(&self.record.service_path),
            self.record.spec.render()?.as_bytes(),
        )?;
        Ok(())
    }

    pub fn register(&mut self, manager: &impl ServiceManager) -> Result<(), String> {
        self.check_artifacts()?;
        preflight_shims(&self.record, &self.source)?;
        if !matches!(
            manager.inspect(&self.record)?,
            State::Owned { running: true }
        ) && socket_live(Path::new(&self.record.spec.home))
        {
            return Err("a daemon is already running outside this service; stop it first (holders keep running)".into());
        }
        if !matches!(
            manager.inspect(&self.record)?,
            State::Owned { running: true }
        ) {
            // Reject an existing standalone owner even before its socket binds.
            // Release before asking the manager to start: its daemon takes the
            // shared lock and SQLite still arbitrates competing starts.
            let probe = agend_daemon::store::maintenance::Maintenance::acquire(Path::new(
                &self.record.spec.home,
            ))
            .map_err(|e| format!("home is already in use: {e}"))?;
            drop(probe);
        }
        manager.start(&self.record)?;
        if manager.inspect(&self.record)? == State::Absent {
            return Err(
                "service manager did not retain the installation; retry to reconcile".into(),
            );
        }
        self.record.phase = InstallPhase::Registered;
        files::save(&self.dir, &self.record, true)
    }

    #[cfg(test)]
    pub fn remove(&mut self, manager: &impl ServiceManager) -> Result<(), String> {
        self.remove_with_data(manager, false)
    }

    pub fn remove_with_data(
        &mut self,
        manager: &impl ServiceManager,
        delete_data: bool,
    ) -> Result<(), String> {
        self.check_artifacts()?;
        if delete_data {
            super::purge::preflight(Path::new(&self.record.spec.home))?;
        }
        holder_paths(Path::new(&self.record.spec.home))?;
        // Inspect before recording removal so a foreign service is unchanged.
        manager.inspect(&self.record)?;
        self.record.phase = InstallPhase::Removing;
        files::save(&self.dir, &self.record, true)?;
        manager.stop(&self.record)?;
        let home = Path::new(&self.record.spec.home);
        if socket_live(home) {
            return Err("a daemon still owns this home; data and executable preserved".into());
        }
        // Keep startup excluded until all managed resources are removed. The
        // daemon takes the shared side before even creating/opening its DB.
        let _maintenance = agend_daemon::store::maintenance::Maintenance::acquire(home)
            .map_err(|e| format!("cannot exclusively maintain this home: {e}"))?;
        stop_holders(home)?;
        self.check_artifacts()?;
        remove_shims(&self.record)?;
        let unit = Path::new(&self.record.service_path);
        remove_if_present(unit)?;
        manager.reload(&self.record)?;
        remove_if_present(Path::new(&self.record.spec.program))?;
        if delete_data {
            super::purge::delete(home)?;
        }
        remove_if_present(&self.dir.join(files::RECORD))?;
        // install.lock stays at the same inode for concurrent waiters. The
        // user's task/config/archive data is deliberately retained.
        Ok(())
    }
}

pub(super) fn validate(record: &Installation, plan: &Plan) -> Result<(), String> {
    if record.version != 1
        || record.spec.manager != plan.spec.manager
        || record.spec.home != plan.spec.home
        || record.spec.user_home != plan.spec.user_home
        || record.spec.program != plan.spec.program
        || Path::new(&record.service_path) != plan.service_path
        || record.program_sha256.len() != 64
        || !record.program_sha256.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err("installation receipt does not belong to this home and service path; preserving all files".into());
    }
    record.spec.render()?;
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => File::open(path.parent().ok_or("removed file has no parent")?)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove {}: {e}", path.display())),
    }
}

fn socket_live(home: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(home.join(agend_core::protocol::client::DAEMON_SOCKET))
        .is_ok()
}

/// Validate the whole candidate set before sending any shutdown. A link in
/// either directory component or at the socket must never select another home.
fn holder_paths(home: &Path) -> Result<Vec<String>, String> {
    if !files::existing_directory(home)?
        || !files::existing_directory(&home.join("run"))?
        || !files::existing_directory(&home.join("run/holders"))?
    {
        return Ok(Vec::new());
    }
    let dir = home.join("run/holders");
    let mut live = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|s| s.strip_suffix(".lock")) else {
            continue;
        };
        files::regular(&entry.path())?.ok_or("holder lock disappeared")?;
        if agend_daemon::runtime::files::lock_holder(&entry.path())
            .map_err(|e| e.to_string())?
            .is_some()
        {
            files::holder_socket(&dir.join(format!("{id}.sock")))?;
            live.push(id.to_owned());
        }
    }
    Ok(live)
}

fn stop_holders(home: &Path) -> Result<(), String> {
    for id in holder_paths(home)? {
        agend_daemon::runtime::shutdown_holder_within(home, &id, Duration::from_secs(5))?;
    }
    Ok(())
}

fn remove_shims(record: &Installation) -> Result<(), String> {
    let home = Path::new(&record.spec.home);
    let program = Path::new(&record.spec.program);
    let pinned =
        agend_daemon::backend_versions::verified_pinned_executable(home, &record.program_sha256)
            .ok();
    let names = agend_daemon::runtime::shims::NAMES
        .iter()
        .map(|name| ("bin", *name))
        .chain(
            agend_shim::hook::NAMES
                .split_whitespace()
                .map(|name| ("hooks", name)),
        );
    for (dir, name) in names {
        let parent = home.join(dir);
        if !parent.exists() {
            continue;
        }
        files::private_dir(&parent)?;
        let path = parent.join(name);
        match fs::read_link(&path) {
            Ok(target) if target == program || pinned.as_ref() == Some(&target) => {
                remove_if_present(&path)?
            }
            Ok(_) => {} // Foreign shim preserved.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {} // Ordinary user files are not ours.
        }
    }
    Ok(())
}

/// Boot repairs its reserved shim names. Refuse foreign occupants before
/// registration so the new daemon cannot overwrite them as a side effect.
fn preflight_shims(record: &Installation, source: &Path) -> Result<(), String> {
    let bin = Path::new(&record.spec.home).join("bin");
    if !files::existing_directory(&bin)? {
        return Ok(());
    }
    let pinned = agend_daemon::backend_versions::verified_pinned_executable(
        Path::new(&record.spec.home),
        &record.program_sha256,
    )
    .ok();
    for name in agend_daemon::runtime::shims::NAMES {
        for (path, allow_owned) in [
            (bin.join(name), true),
            (bin.join(format!(".{name}.new")), false),
        ] {
            match fs::symlink_metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
                Ok(_) => {}
            }
            if allow_owned
                && fs::read_link(&path).is_ok_and(|target| {
                    target == Path::new(&record.spec.program)
                        || target == source
                        || pinned.as_ref() == Some(&target)
                })
            {
                continue;
            }
            return Err(format!(
                "{} is not an owned shim; move it before installing the service",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod pinned_tests {
    use super::*;
    use agend_core::setup::service::{Manager, ServiceSpec};
    use agend_testkit::tempdir::TempDir;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn pinned_shims_reconcile_only_the_recorded_installation_digest() {
        let root = TempDir::new("g13-pinned-shims").unwrap();
        let home = root.path();
        let source = std::env::current_exe().unwrap();
        let (pinned, binding) =
            agend_daemon::backend_versions::ExecutableBinding::pin_running(home, &source).unwrap();
        let record = Installation {
            version: 1,
            spec: ServiceSpec {
                manager: Manager::Launchd,
                program: home.join("service/agend").display().to_string(),
                home: home.display().to_string(),
                user_home: home.display().to_string(),
                search_path: "/usr/bin:/bin".into(),
            },
            service_path: home.join("service.plist").display().to_string(),
            program_sha256: binding.digest_if_unchanged(&pinned).unwrap().into(),
            phase: InstallPhase::Registered,
        };
        let bin = home.join("bin");
        fs::create_dir(&bin).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&pinned, bin.join("git")).unwrap();
        preflight_shims(&record, &source).unwrap();
        let mut wrong = record.clone();
        wrong.program_sha256 = "0".repeat(64);
        assert!(preflight_shims(&wrong, &source).is_err());
        remove_shims(&wrong).unwrap();
        assert_eq!(fs::read_link(bin.join("git")).unwrap(), pinned);
        let foreign = home.join("foreign");
        fs::write(&foreign, b"preserve").unwrap();
        symlink(&foreign, bin.join("gh")).unwrap();
        assert!(preflight_shims(&record, &source).is_err());
        remove_shims(&record).unwrap();
        assert!(fs::symlink_metadata(bin.join("git")).is_err());
        assert_eq!(fs::read_link(bin.join("gh")).unwrap(), foreign);
        assert_eq!(fs::read(&foreign).unwrap(), b"preserve");
        assert!(pinned.exists(), "default uninstall keeps runtime data");
    }
}
