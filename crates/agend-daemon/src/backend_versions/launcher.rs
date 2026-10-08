//! Execute the verified private launcher before holder connection deadlines.
//! A newly copied executable may spend seconds in OS first-exec processing.
use super::ExecutableBinding;
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) fn prepare(path: &Path, binding: &ExecutableBinding) -> Result<(), String> {
    prepare_within(path, binding, Duration::from_secs(30))
}

fn prepare_within(
    path: &Path,
    binding: &ExecutableBinding,
    within: Duration,
) -> Result<(), String> {
    binding.digest_if_unchanged(path)?;
    let mut child = Command::new(path)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot execute private launcher: {e}"))?;
    let deadline = Instant::now() + within;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(format!("private launcher --version failed: {status}"));
                }
                return binding.digest_if_unchanged(path).map(|_| ());
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => {
                // This is only our own short-lived --version child, not a
                // holder, backend, service or another operator's process.
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(e) => format!("cannot wait for private launcher: {e}"),
                    _ => "private launcher --version timed out".into(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn native_launcher_success_failure_timeout_and_changed_identity() {
        let dir = TempDir::new("launcher-preflight").unwrap();
        let path = dir.path().join("launcher");
        for (script, expected) in [
            ("#!/bin/sh\n[ \"$1\" = --version ]\n", None),
            ("#!/bin/sh\nexit 7\n", Some("failed")),
            ("#!/bin/sh\nexec /bin/sleep 5\n", Some("timed out")),
        ] {
            fs::write(&path, script).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            let binding = ExecutableBinding::capture(&path).unwrap();
            let within = if expected == Some("timed out") {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(5)
            };
            let result = prepare_within(&path, &binding, within);
            match expected {
                None => result.unwrap(),
                Some(message) => assert!(result.unwrap_err().contains(message)),
            }
        }
        let binding = ExecutableBinding::capture(&path).unwrap();
        fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(prepare_within(&path, &binding, Duration::from_secs(1)).is_err());
    }
}
