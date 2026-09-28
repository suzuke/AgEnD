//! Reaps holders inherited through an `exec` restart (gate 9 P7). After
//! `agend daemon restart` the holders the old image started are still
//! children of this pid, but the threads that `wait`ed for them are gone;
//! one that ends would stay a zombie.
//!
//! At boot, before any holder is started, every pid written in a holder
//! lock file gets one `waitpid(pid, WNOHANG)`: not a child (`ECHILD`) or
//! already reaped → dropped at once; a live child → polled every second
//! until it is reaped or `ECHILD`, then never again (a reused pid is never
//! touched). Never `waitpid(-1)`: that would steal the exit status that
//! `runtime`'s per-holder `wait` threads and the preflight child rely on.
//!
//! Must NOT: wait for any pid not read from a lock file at boot.

use std::path::Path;
use std::time::Duration;

use crate::log;
use crate::runtime::files;

/// How often inherited children are polled.
pub const POLL_EVERY: Duration = Duration::from_secs(1);

/// One `waitpid(pid, WNOHANG)`: true while `pid` is a live child of ours.
fn still_ours(pid: u32) -> bool {
    let Ok(raw) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if raw <= 1 {
        return false;
    }
    let mut status = 0;
    // SAFETY: waitpid on one positive pid > 1; `status` is ours.
    let got = unsafe { libc::waitpid(raw, &mut status, libc::WNOHANG) };
    match got {
        0 => true,
        n if n == raw => {
            log::line(&format!(
                "reaped inherited holder pid {pid} ({})",
                describe(status)
            ));
            false
        }
        // ECHILD (not our child) or anything else: never ask again.
        _ => false,
    }
}

fn describe(status: libc::c_int) -> String {
    if libc::WIFEXITED(status) {
        format!("exit {}", libc::WEXITSTATUS(status))
    } else if libc::WIFSIGNALED(status) {
        format!("signal {}", libc::WTERMSIG(status))
    } else {
        format!("status {status}")
    }
}

/// The first poll of every lock-file pid under `home`, now (call it before
/// any holder is started); returns the live children to keep polling.
pub fn inherited(home: &Path) -> Vec<u32> {
    files::lock_pids(home)
        .into_iter()
        .filter(|&pid| still_ours(pid))
        .collect()
}

/// Polls `pids` every [`POLL_EVERY`] on the daemon's runtime until each is
/// reaped or not a child any more.
pub fn watch(mut pids: Vec<u32>) {
    if pids.is_empty() {
        return;
    }
    log::line(&format!(
        "inherited holder children (exec restart): {}",
        pids.iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    ));
    tokio::spawn(async move {
        while !pids.is_empty() {
            tokio::time::sleep(POLL_EVERY).await;
            pids.retain(|&pid| still_ours(pid));
        }
    });
}
