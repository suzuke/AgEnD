//! Bounded version probes and cleanup of the direct child and its original group.
use std::{
    io::{self, Read},
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

const MAX_OUTPUT: usize = 64 * 1024;

pub fn version_line(program: &Path) -> Result<String, String> {
    probe(program, Duration::from_secs(5))
        .map_err(|e| format!("{} --version: {e}", program.display()))
}

fn probe(program: &Path, within: Duration) -> Result<String, String> {
    probe_command(Command::new(program), within)
}

/// Probe with exactly the caller-selected cwd and environment. Daemon callers
/// pass their launch whitelist; no unrelated daemon credentials are inherited.
pub fn version_line_in(
    program: &Path,
    cwd: &Path,
    environment: &std::collections::BTreeMap<String, String>,
) -> Result<String, String> {
    let mut command = Command::new(program);
    command.current_dir(cwd).env_clear().envs(environment);
    probe_command(command, Duration::from_secs(5))
        .map_err(|e| format!("{} --version: {e}", program.display()))
}

fn probe_command(mut command: Command, within: Duration) -> Result<String, String> {
    let deadline = Instant::now() + within;
    let child = command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot run: {e}"))?;
    let mut owned = ProbeChild(Some(child));
    let result = (|| {
        let child = owned.0.as_mut().expect("owned child");
        let mut stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        nonblocking(&stdout)
            .and_then(|()| nonblocking(&stderr))
            .map_err(|e| e.to_string())?;
        let pid = child.id();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (mut out_eof, mut err_eof) = (false, false);
        loop {
            if Instant::now() >= deadline {
                return Err(format!("did not complete within {} ms", within.as_millis()));
            }
            if !out_eof {
                out_eof = read_chunk(&mut stdout, &mut out).map_err(|e| e.to_string())?;
            }
            if !err_eof {
                err_eof = read_chunk(&mut stderr, &mut err).map_err(|e| e.to_string())?;
            }
            if out.len() + err.len() > MAX_OUTPUT {
                return Err(format!("output exceeds {MAX_OUTPUT} bytes"));
            }
            if exited_unreaped(pid).map_err(|e| e.to_string())? && out_eof && err_eof {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok::<_, String>((out, err))
    })();
    let status = owned.finish().map_err(|cleanup| match &result {
        Err(primary) => format!("{primary}; cleanup: {cleanup}"),
        Ok(_) => format!("cleanup: {cleanup}"),
    })?;
    let (out, err) = result?;
    if !status.success() {
        return Err(format!("exited unsuccessfully ({status})"));
    }
    // Keep stdout and stderr separate: a missing newline must not concatenate versions.
    let out = String::from_utf8(out).map_err(|_| "output is not UTF-8")?;
    let err = String::from_utf8(err).map_err(|_| "output is not UTF-8")?;
    for text in [out, err] {
        if let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty()) {
            return Ok(line.to_owned());
        }
    }
    Err("printed nothing".into())
}

fn nonblocking(pipe: &impl AsRawFd) -> io::Result<()> {
    // SAFETY: the pipe owns this live descriptor; only its status flags change.
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn read_chunk(pipe: &mut impl Read, output: &mut Vec<u8>) -> io::Result<bool> {
    let mut chunk = [0; 4096];
    match pipe.read(&mut chunk) {
        Ok(0) => Ok(true),
        Ok(count) => {
            output.extend_from_slice(&chunk[..count]);
            Ok(false)
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

fn exited_unreaped(pid: u32) -> io::Result<bool> {
    // SAFETY: waitid writes an initialized siginfo for our child. WNOWAIT keeps
    // its PID reserved until group cleanup, even if its descendants hold pipes open.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result < 0 {
        let e = io::Error::last_os_error();
        if e.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(e);
    }
    // SAFETY: waitid initialized the child-status fields above.
    Ok(unsafe { info.si_pid() } != 0)
}

struct ProbeChild(Option<Child>);
impl ProbeChild {
    // On a kernel cleanup failure, report the PID instead of blocking forever
    // in wait. Only an observed exited child can be synchronously reaped.
    #[allow(clippy::zombie_processes)]
    fn finish(&mut self) -> io::Result<ExitStatus> {
        let mut child = self.0.take().expect("unreaped child");
        let pid = child.id() as libc::pid_t;
        let deadline = Instant::now() + Duration::from_secs(2);
        // SAFETY: the unreaped child pins its PID. Its original group cannot
        // have been reused. Signal the direct PID too if it changed groups.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
            let error = io::Error::last_os_error();
            if !exited_unreaped(child.id())? {
                return Err(io::Error::other(format!(
                    "cannot stop probe pid {pid}: {error}"
                )));
            }
        }
        while !exited_unreaped(child.id())? {
            if Instant::now() >= deadline {
                return Err(io::Error::other(format!(
                    "probe pid {pid} did not exit after SIGKILL"
                )));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let status = child.wait()?;
        loop {
            // Read-only after reaping: never signal a possibly reused group.
            if unsafe { libc::kill(-pid, 0) } != 0 {
                if io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                    return Ok(status);
                }
                return Err(io::Error::other(format!(
                    "cannot verify probe group {pid} cleanup"
                )));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::other(format!(
                    "probe group {pid} remains after cleanup"
                )));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Drop for ProbeChild {
    fn drop(&mut self) {
        if self.0.is_some() {
            let _ = self.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn script(root: &Path, text: &str) -> std::path::PathBuf {
        let path = root.join("probe");
        fs::write(&path, format!("#!/bin/sh\n{text}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn native_version_probe_requires_success_and_keeps_streams_separate() {
        let root = TempDir::new("g13-version").unwrap();
        let path = script(root.path(), "printf '1.2.3'; printf 'warning' >&2");
        assert_eq!(version_line(&path).unwrap(), "1.2.3");
        script(
            root.path(),
            "printf 'error pretending to be a version'; exit 7",
        );
        assert!(version_line(&path).unwrap_err().contains("unsuccessfully"));
        script(root.path(), "printf '\\n1.2.4\\n' >&2");
        assert_eq!(version_line(&path).unwrap(), "1.2.4");
        script(root.path(), "printf '1.2.4'; printf '\\377' >&2");
        assert!(version_line(&path).unwrap_err().contains("UTF-8"));
    }

    #[test]
    fn native_version_probe_stops_its_child_after_it_changes_groups() {
        let root = TempDir::new("g13-version-escape").unwrap();
        let source = root.path().join("probe.c");
        let program = root.path().join("probe");
        let marker = root.path().join("moved");
        fs::write(
            &source,
            format!(
                r#"
#include <unistd.h>
#include <stdio.h>
int main(void) {{
    if (setpgid(0, getpgid(getppid()))) return 3;
    FILE *f = fopen({:?}, "w");
    if (!f) return 4;
    fprintf(f, "%d", getpid()); fclose(f);
    sleep(30);
    return 0;
}}
"#,
                marker.to_str().unwrap()
            ),
        )
        .unwrap();
        assert!(
            Command::new("/usr/bin/cc")
                .arg(source)
                .arg("-o")
                .arg(&program)
                .status()
                .unwrap()
                .success()
        );
        let start = Instant::now();
        assert!(
            version_line(&program)
                .unwrap_err()
                .contains("did not complete")
        );
        assert!(start.elapsed() < Duration::from_secs(8));
        let pid = fs::read_to_string(marker).unwrap();
        assert!(
            !Command::new("/bin/ps")
                .args(["-p", pid.trim(), "-o", "pid="])
                .output()
                .unwrap()
                .status
                .success()
        );
    }

    #[test]
    fn native_version_probe_bounds_output_and_inherited_pipes() {
        let root = TempDir::new("g13-version-bound").unwrap();
        let path = script(
            root.path(),
            "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
        );
        let error = probe(&path, Duration::from_secs(2)).unwrap_err();
        assert!(error.contains("output exceeds"), "{error}");
        // The direct shell exits, but its native sleep child retains both pipes.
        let pid_file = root.path().join("descendant");
        script(
            root.path(),
            &format!(
                "/bin/sleep 30 &\necho $! > '{}'\nprintf '1.2.3'",
                pid_file.display()
            ),
        );
        let start = Instant::now();
        let error = probe(&path, Duration::from_millis(250)).unwrap_err();
        assert!(error.contains("did not complete"), "{error}");
        // The 250 ms probe and the separate 2 s cleanup budget are additive.
        assert!(start.elapsed() < Duration::from_secs(3), "{error}");
        let pid = fs::read_to_string(pid_file).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let status = Command::new("/bin/ps")
                .args(["-o", "stat=", "-p", pid.trim()])
                .output()
                .unwrap();
            let state = String::from_utf8_lossy(&status.stdout);
            if !status.status.success() || state.trim().starts_with('Z') {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "probe descendant remains: {state}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn native_scoped_probe_uses_only_selected_environment_and_cwd() {
        let dir = TempDir::new("scoped-version-probe").unwrap();
        let path = script(
            dir.path(),
            r#"[ "$1" = --version ] || exit 2
[ "$PWD" = "$EXPECTED_CWD" ] || exit 3
[ "$PROBE_MARK" = scoped ] || exit 4
[ -z "${HOME+x}" ] || exit 5
printf 'scoped 1.0\n'"#,
        );
        let cwd = dir.path().canonicalize().unwrap();
        let env = std::collections::BTreeMap::from([
            ("EXPECTED_CWD".into(), cwd.to_string_lossy().into_owned()),
            ("PROBE_MARK".into(), "scoped".into()),
        ]);
        assert_eq!(version_line_in(&path, &cwd, &env).unwrap(), "scoped 1.0");
    }
}
