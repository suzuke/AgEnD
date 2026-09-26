//! Records the agent's exit status (code or signal). The holder keeps it and
//! reports it on every connection until `Shutdown` (P7).
//!
//! The ended agent is deliberately left a zombie (`waitid` with `WNOWAIT`)
//! until the holder stops: while the zombie exists its pid, and so its
//! process group id, cannot be reused. `Shutdown` can then SIGKILL leftover
//! children in that group without any chance of hitting an unrelated process,
//! and reaps the agent afterwards (`reap`).
//!
//! Must NOT: restart the agent on its own; the daemon decides.

use agend_core::protocol::holder::ExitedData;

/// Blocks until the agent `pid` (a child of this process) ends. Does not reap
/// it; see the module comment.
pub fn wait(pid: u32) -> ExitedData {
    loop {
        // SAFETY: zeroed siginfo_t is a valid out-parameter for waitid.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: waitid on our own child's pid with a valid out pointer.
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if rc == 0 {
            // SAFETY: waitid filled `info` for a child state change.
            let status = unsafe { info.si_status() };
            return from_child_info(info.si_code, status);
        }
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        // Not our child any more (should not happen): nothing is known.
        return ExitedData {
            code: None,
            signal: None,
        };
    }
}

/// Reaps the ended agent, if it is still a zombie of this process.
pub fn reap(pid: u32) {
    let mut status = 0;
    // SAFETY: non-blocking waitpid on our own child's pid.
    unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
}

/// Maps `waitid`'s `si_code`/`si_status` to the protocol's exit data.
pub fn from_child_info(code: libc::c_int, status: libc::c_int) -> ExitedData {
    if code == libc::CLD_EXITED {
        ExitedData {
            code: Some(status),
            signal: None,
        }
    } else {
        ExitedData {
            code: None,
            signal: Some(signal_name(status)),
        }
    }
}

/// `SIGKILL`-style name; unnamed numbers become `SIG<n>`.
pub fn signal_name(signal: libc::c_int) -> String {
    const NAMES: &[(libc::c_int, &str)] = &[
        (libc::SIGHUP, "SIGHUP"),
        (libc::SIGINT, "SIGINT"),
        (libc::SIGQUIT, "SIGQUIT"),
        (libc::SIGILL, "SIGILL"),
        (libc::SIGTRAP, "SIGTRAP"),
        (libc::SIGABRT, "SIGABRT"),
        (libc::SIGBUS, "SIGBUS"),
        (libc::SIGFPE, "SIGFPE"),
        (libc::SIGKILL, "SIGKILL"),
        (libc::SIGUSR1, "SIGUSR1"),
        (libc::SIGSEGV, "SIGSEGV"),
        (libc::SIGUSR2, "SIGUSR2"),
        (libc::SIGPIPE, "SIGPIPE"),
        (libc::SIGALRM, "SIGALRM"),
        (libc::SIGTERM, "SIGTERM"),
        (libc::SIGXCPU, "SIGXCPU"),
        (libc::SIGXFSZ, "SIGXFSZ"),
        (libc::SIGVTALRM, "SIGVTALRM"),
        (libc::SIGPROF, "SIGPROF"),
        (libc::SIGSYS, "SIGSYS"),
    ];
    NAMES
        .iter()
        .find(|(n, _)| *n == signal)
        .map_or_else(|| format!("SIG{signal}"), |(_, name)| (*name).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_names_are_portable_names() {
        assert_eq!(signal_name(libc::SIGKILL), "SIGKILL");
        assert_eq!(signal_name(libc::SIGTERM), "SIGTERM");
        assert_eq!(signal_name(200), "SIG200");
    }

    #[test]
    fn real_children_map_to_code_or_signal_and_stay_unreaped_until_reap() {
        // Produced by real processes (#1493), not hand-built statuses.
        let run = |script: &str| {
            // Reaped by `reap` below instead of `Child::wait`.
            #[allow(clippy::zombie_processes)]
            let child = std::process::Command::new("/bin/sh")
                .args(["-c", script])
                .spawn()
                .unwrap();
            let pid = child.id();
            let exited = wait(pid);
            // Still a zombie: waiting again reports the same status.
            assert_eq!(wait(pid), exited);
            reap(pid);
            exited
        };
        assert_eq!(
            run("exit 7"),
            ExitedData {
                code: Some(7),
                signal: None
            }
        );
        assert_eq!(
            run("kill -KILL $$"),
            ExitedData {
                code: None,
                signal: Some("SIGKILL".into())
            }
        );
    }
}
