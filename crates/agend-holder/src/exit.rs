//! Records the agent's exit status (code or signal). The holder keeps it and
//! reports it on every connection until `Shutdown` (P7).
//!
//! Must NOT: restart the agent on its own; the daemon decides.

use agend_core::protocol::holder::ExitedData;

/// Blocks until the agent `pid` (a child of this process) ends and reaps it.
pub fn wait(pid: u32) -> ExitedData {
    let mut status = 0;
    loop {
        // SAFETY: waitpid on our own child's pid with a valid out pointer.
        let rc = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) };
        if rc == pid as libc::pid_t {
            return from_wait_status(status);
        }
        if rc == -1 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        // Not our child any more (should not happen): nothing is known.
        return ExitedData {
            code: None,
            signal: None,
        };
    }
}

pub fn from_wait_status(status: libc::c_int) -> ExitedData {
    if libc::WIFSIGNALED(status) {
        ExitedData {
            code: None,
            signal: Some(signal_name(libc::WTERMSIG(status))),
        }
    } else {
        ExitedData {
            code: Some(libc::WEXITSTATUS(status)),
            signal: None,
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
    fn real_child_statuses_map_to_code_or_signal() {
        // Produced by real processes (#1493), not hand-built status words.
        use std::os::unix::process::ExitStatusExt;
        let code = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .status()
            .unwrap();
        assert_eq!(
            from_wait_status(code.into_raw()),
            ExitedData {
                code: Some(7),
                signal: None
            }
        );
        let killed = std::process::Command::new("/bin/sh")
            .args(["-c", "kill -KILL $$"])
            .status()
            .unwrap();
        assert_eq!(
            from_wait_status(killed.into_raw()),
            ExitedData {
                code: None,
                signal: Some("SIGKILL".into())
            }
        );
    }
}
