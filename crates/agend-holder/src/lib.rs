//! AgEnD holder: one process per instance that owns the agent's PTY and its
//! rendered screen (alacritty_terminal), so the daemon can restart without the
//! agent noticing (D3). Runs from the same `agend` binary as
//! `agend holder <instance-id>` (D11, gate 4 P1), on std threads only.
//!
//! Process rules (gate 4 P2, P3):
//! - Own session (`setsid`), stdin `/dev/null`, stdout/stderr appended to
//!   `$AGEND_HOME/run/holders/<id>.log`.
//! - SIGHUP, SIGINT, SIGQUIT and SIGTERM are ignored; only the protocol
//!   `Shutdown` stops it. SIGKILL cannot be ignored: the agent then dies with
//!   the holder (the known ceiling; recovery is gate 6).
//! - Holds an exclusive `flock` on `<id>.lock` (content: its pid) for its whole
//!   life; a second holder for the same id exits 1.
//! - Safety net: exits (stopping the agent first) when `AGEND_HOME` is deleted,
//!   or when no agent is running and no client has been connected for 24 h
//!   (`AGEND_HOLDER_IDLE_EXIT_SECS` overrides this, for tests).
//!
//! Must NOT: contain pipeline/delivery logic, open the DB, write agent message
//! text to the PTY (see `pty` for the three kinds of bytes it may write), or
//! restart the agent.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

pub mod client;
pub mod exit;
pub mod paths;
pub mod pty;
pub mod screen;
pub mod server;
pub mod sidecar;

use paths::HolderPaths;

/// Environment variable that overrides the 24 h idle exit (seconds, > 0).
pub const IDLE_EXIT_ENV: &str = "AGEND_HOLDER_IDLE_EXIT_SECS";

/// Entry point of `agend holder <instance-id>`. Exit codes: 0 after a stop,
/// 1 when another holder already runs for the id, 2 on usage or setup errors.
pub fn run(args: Vec<OsString>) -> ExitCode {
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let [id] = args.as_slice() else {
        eprintln!("usage: agend holder <instance-id>");
        return ExitCode::from(2);
    };
    match start(id) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("agend holder: {message}");
            ExitCode::from(2)
        }
    }
}

fn start(id: &str) -> Result<ExitCode, String> {
    paths::validate_instance_id(id)?;
    let agend_home = paths::agend_home()?;
    let paths = HolderPaths::new(&agend_home, id);
    paths.check_socket_len()?;
    let idle_exit = idle_exit_from_env()?;

    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&paths.dir)
        .map_err(|e| format!("create {}: {e}", paths.dir.display()))?;
    fs::set_permissions(&paths.dir, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("chmod {}: {e}", paths.dir.display()))?;

    // Before anything else is touched, so a refused start changes nothing.
    let Some(mut lock) = take_lock(&paths.lock)? else {
        let pid = paths::read_lock_pid(&paths.lock).map_or("?".into(), |p| p.to_string());
        eprintln!("holder for {id} already running (pid {pid})");
        return Ok(ExitCode::from(1));
    };
    lock.set_len(0)
        .and_then(|()| writeln!(lock, "{}", std::process::id()))
        .map_err(|e| format!("write {}: {e}", paths.lock.display()))?;

    detach(&paths.log)?;
    server::log(id, &format!("started (pid {})", std::process::id()));

    match fs::remove_file(&paths.socket) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("remove old {}: {e}", paths.socket.display())),
    }
    let listener = UnixListener::bind(&paths.socket)
        .map_err(|e| format!("bind {}: {e}", paths.socket.display()))?;

    let stop = server::serve(
        listener,
        server::Config {
            instance_id: id.to_string(),
            agend_home,
            idle_exit,
        },
    );
    let _ = fs::remove_file(&paths.socket);
    server::log(id, &format!("stopped ({stop:?}); socket removed"));
    drop(lock); // releases the flock; exiting would too
    Ok(ExitCode::SUCCESS)
}

fn idle_exit_from_env() -> Result<Duration, String> {
    match std::env::var(IDLE_EXIT_ENV) {
        Err(_) => Ok(server::DEFAULT_IDLE_EXIT),
        Ok(value) => match value.parse::<u64>() {
            Ok(secs) if secs > 0 => Ok(Duration::from_secs(secs)),
            _ => Err(format!(
                "{IDLE_EXIT_ENV} must be a positive number of seconds, got {value:?}"
            )),
        },
    }
}

/// Opens and exclusively locks the lock file; `None` if another process
/// holds it. The file is not truncated before the lock is ours.
fn take_lock(path: &Path) -> Result<Option<File>, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    // SAFETY: flock on an fd we own; held until `file` is dropped or we exit.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(file));
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(None)
    } else {
        Err(format!("lock {}: {err}", path.display()))
    }
}

/// New session, terminating signals ignored, stdio away from any terminal.
fn detach(log: &Path) -> Result<(), String> {
    // SAFETY: plain syscalls on this process; no memory is shared.
    unsafe {
        if libc::setsid() == -1 {
            return Err(format!(
                "setsid: {} (start the holder from another process, not as a process group leader)",
                io::Error::last_os_error()
            ));
        }
        for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM] {
            libc::signal(signal, libc::SIG_IGN);
        }
    }
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log)
        .map_err(|e| format!("open {}: {e}", log.display()))?;
    let null = File::open("/dev/null").map_err(|e| format!("open /dev/null: {e}"))?;
    // SAFETY: dup2 onto the standard descriptors from fds we own.
    let ok = unsafe {
        libc::dup2(null.as_raw_fd(), 0) != -1
            && libc::dup2(log_file.as_raw_fd(), 1) != -1
            && libc::dup2(log_file.as_raw_fd(), 2) != -1
    };
    if ok {
        Ok(())
    } else {
        Err(format!("redirect stdio: {}", io::Error::last_os_error()))
    }
}
