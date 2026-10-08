//! Owned canary child processes and temporary home. Never registers a service.
use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
mod containment;

#[cfg(test)]
mod tests;

pub struct Lab {
    pub home: PathBuf,
    inode: u64,
    device: u64,
    daemon: Option<Child>,
    probe: Option<Probe>,
    finished: bool,
}
impl Lab {
    pub fn new(id: &str) -> Result<Self, String> {
        let home = Path::new("/tmp").join(format!("agend-canary-{}", &id[..18]));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&home)
            .map_err(|e| e.to_string())?;
        let inode = fs::symlink_metadata(&home)
            .map_err(|e| e.to_string())?
            .ino();
        let device = fs::symlink_metadata(&home)
            .map_err(|e| e.to_string())?
            .dev();
        let lab = Self {
            home,
            inode,
            device,
            daemon: None,
            probe: None,
            finished: false,
        };
        fs::DirBuilder::new()
            .mode(0o700)
            .create(lab.home.join("workspace"))
            .map_err(|e| e.to_string())?;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(lab.home.join("probe-home"))
            .map_err(|e| e.to_string())?;
        Ok(lab)
    }
    pub fn version(&mut self, program: &Path, deadline: Instant) -> Result<String, String> {
        let output = self.home.join("version-output");
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&output)
            .map_err(|e| e.to_string())?;
        let mut command = containment::command(program).map_err(|e| e.to_string())?;
        command
            .arg("--version")
            .env_clear()
            .env("HOME", self.home.join("probe-home"))
            .env("PATH", "/usr/bin:/bin")
            .env("DISABLE_AUTOUPDATER", "1")
            .env("DISABLE_UPDATES", "1")
            .env("OPENCODE_DISABLE_AUTOUPDATE", "1")
            .current_dir(&self.home)
            .stdin(Stdio::null())
            .stdout(file)
            .stderr(Stdio::null())
            .process_group(0);
        // SAFETY: only an async-signal-safe libc call runs in the fork child.
        // The limit applies solely to this version probe and bounds file output.
        unsafe {
            command.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: 8192,
                    rlim_max: 8192,
                };
                if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        self.probe = Some(Probe {
            child: command.spawn().map_err(|e| e.to_string())?,
            status: None,
        });
        let end = deadline.min(Instant::now() + Duration::from_secs(5));
        loop {
            if exited_unreaped(&self.probe.as_ref().unwrap().child)? {
                let status = stop_probe(self.probe.as_mut().unwrap())?;
                self.probe = None;
                if !status.success() {
                    return Err("backend version probe failed".into());
                }
                return fs::read_to_string(output)
                    .map_err(|_| "backend version output is not UTF-8".into());
            }
            if Instant::now() >= end {
                return Err("backend version probe timed out".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    pub fn start(&mut self, agend: &Path) -> Result<(), String> {
        let mut command = Command::new(agend);
        command
            .arg("daemon")
            .env_clear()
            .env("AGEND_HOME", &self.home)
            .current_dir(&self.home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.env("HOME", self.home.join("probe-home"));
        for name in [
            "USER", "LOGNAME", "PATH", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "TZ",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        self.daemon = Some(command.spawn().map_err(|e| e.to_string())?);
        Ok(())
    }
    pub fn check_alive(&mut self) -> Result<(), String> {
        if self
            .daemon
            .as_mut()
            .ok_or("canary daemon was not started")?
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("canary daemon exited before readiness".into());
        }
        Ok(())
    }
    fn check_home(&self) -> Result<(), String> {
        let meta = fs::symlink_metadata(&self.home).map_err(|e| e.to_string())?;
        // SAFETY: getuid reads only the current user identity.
        if !meta.is_dir()
            || meta.ino() != self.inode
            || meta.dev() != self.device
            || meta.uid() != unsafe { libc::getuid() }
            || meta.mode() & 0o077 != 0
        {
            return Err("canary home identity changed; preserving it".into());
        }
        Ok(())
    }
    pub fn cleanup(&mut self) -> Result<(), String> {
        let result = self.cleanup_inner();
        // An explicit failed cleanup is reported with retained ownership evidence.
        // Do not silently retry in Drop and make that report inaccurate.
        self.finished = true;
        result
    }
    fn cleanup_inner(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        // Stop the daemon before the holder so a restart cannot race cleanup.
        if let Some(probe) = &mut self.probe {
            stop_probe(probe)?;
        }
        if let Some(daemon) = &mut self.daemon {
            stop(daemon)?;
        }
        self.check_home()?;
        let run = self.home.join("run");
        let holders = run.join("holders");
        if super::files::existing_directory(&run)? && super::files::existing_directory(&holders)? {
            let lock = agend_daemon::runtime::files::lock_path(&self.home, super::ID);
            if let Some(file) = super::files::regular(&lock)? {
                if file.metadata().map_err(|e| e.to_string())?.nlink() != 1 {
                    return Err("canary holder lock has multiple links; preserving it".into());
                }
                if agend_daemon::runtime::files::running(&self.home, super::ID)
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    super::files::holder_socket(&agend_daemon::runtime::files::socket_path(
                        &self.home,
                        super::ID,
                    ))?;
                }
            }
        }
        agend_daemon::runtime::shutdown_holder_within(
            &self.home,
            super::ID,
            Duration::from_secs(5),
        )?;
        if agend_daemon::runtime::files::running(&self.home, super::ID)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("canary holder remains; temporary home retained".into());
        }
        self.check_home()?;
        fs::remove_dir_all(&self.home).map_err(|e| e.to_string())?;
        self.finished = true;
        Ok(())
    }
}
fn stop(child: &mut Child) -> Result<(), String> {
    if child.try_wait().map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }
    let pid = child.id();
    if pid <= 1 {
        return Err("invalid owned child pid".into());
    }
    for signal in [libc::SIGTERM, libc::SIGKILL] {
        // SAFETY: Child has not been reaped; its PID cannot have been reused.
        if unsafe { libc::kill(pid as i32, signal) } != 0
            && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        {
            return Err("cannot stop the owned canary daemon".into());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    Err("canary daemon did not stop; home retained".into())
}
impl Drop for Lab {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.cleanup();
        }
    }
}

// Keep the group leader unreaped until its descendants have received SIGKILL;
// its PID cannot be reused while we still own the zombie child.
fn exited_unreaped(child: &Child) -> Result<bool, String> {
    // SAFETY: zeroed siginfo is initialized by waitid; the PID is our child.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: waitid initialized the child-status variant (or zero for no event).
    Ok(unsafe { info.si_pid() } != 0)
}
struct Probe {
    child: Child,
    status: Option<std::process::ExitStatus>,
}
fn stop_probe(probe: &mut Probe) -> Result<std::process::ExitStatus, String> {
    let pid = probe.child.id();
    if pid <= 1 {
        return Err("invalid owned probe pid".into());
    }
    if probe.status.is_none() {
        // The probe cannot fork, but may move itself out of its original group.
        // Its unreaped Child still pins its PID, so stop it directly as well.
        if unsafe { libc::kill(pid as i32, libc::SIGKILL) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH)
                && !(error.raw_os_error() == Some(libc::EPERM) && exited_unreaped(&probe.child)?)
            {
                return Err(format!("cannot stop the owned version probe: {error}"));
            }
        }
        // SAFETY: version probes use their own group and have never been reaped.
        if unsafe { libc::kill(-(pid as i32), libc::SIGKILL) } != 0 {
            let error = std::io::Error::last_os_error();
            // macOS reports EPERM for a group containing only its zombie leader.
            // Reap only a proven exited child, then require group absence below.
            // A surviving inaccessible descendant still prevents success.
            if error.raw_os_error() != Some(libc::ESRCH)
                && !(error.raw_os_error() == Some(libc::EPERM) && exited_unreaped(&probe.child)?)
            {
                return Err(format!(
                    "cannot stop the owned version probe group: {error}"
                ));
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while !exited_unreaped(&probe.child)? {
            if Instant::now() >= deadline {
                return Err("version probe did not stop; home retained".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        probe.status = Some(probe.child.wait().map_err(|e| e.to_string())?);
    }
    let status = probe.status.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        // Read-only existence check after reaping; never signal a reused group.
        if unsafe { libc::kill(-(pid as i32), 0) } != 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                return Ok(status);
            }
            return Err("cannot establish version probe group cleanup".into());
        }
        if Instant::now() >= deadline {
            return Err("version probe descendants remain; home retained".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
