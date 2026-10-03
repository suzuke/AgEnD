//! Signal-context flag remains usable after Tokio's signal receiver is gone.
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) static STOP_SIGNALLED: AtomicBool = AtomicBool::new(false);
static EXEC_HANDOFF: AtomicBool = AtomicBool::new(false);
/// Called only after the server, connections and runtime have stopped.
/// A late signal must exit instead of racing the final flag check and exec.
pub(super) fn begin_exec_handoff() {
    EXEC_HANDOFF.store(true, Ordering::SeqCst);
}
pub(super) struct Guard(Vec<signal_hook_registry::SigId>);
impl Drop for Guard {
    fn drop(&mut self) {
        for id in self.0.drain(..) {
            signal_hook_registry::unregister(id);
        }
    }
}
pub(super) fn install() -> std::io::Result<Guard> {
    STOP_SIGNALLED.store(false, Ordering::SeqCst);
    EXEC_HANDOFF.store(false, Ordering::SeqCst);
    let mut guard = Guard(Vec::new());
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: only static atomics and async-signal-safe _exit are used;
        // no allocation, locks, panics or runtime work. _exit is enabled only
        // after daemon cleanup, so a late signal cannot run a new image.
        let id = unsafe {
            signal_hook_registry::register(signal, || {
                STOP_SIGNALLED.store(true, Ordering::SeqCst);
                if EXEC_HANDOFF.load(Ordering::SeqCst) {
                    libc::_exit(0);
                }
            })
        }?;
        guard.0.push(id);
    }
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signal_after_runtime_shutdown_still_prevents_exec() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "daemon::stop_flag::tests::child_probe",
                "--nocapture",
            ])
            .env("AGEND_TEST_STOPFLAG_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success(), "signal during exec handoff was lost");
    }
    #[test]
    fn signal_between_final_check_and_exec_exits_successfully() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "daemon::stop_flag::tests::child_probe",
                "--nocapture",
            ])
            .env("AGEND_TEST_STOPFLAG_CHILD", "handoff")
            .status()
            .unwrap();
        assert!(
            status.success(),
            "late signal did not stop the exec handoff"
        );
    }
    #[test]
    fn child_probe() {
        if std::env::var_os("AGEND_TEST_STOPFLAG_CHILD").is_none() {
            return;
        }
        let _guard = install().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (events, _queue) = tokio::sync::mpsc::unbounded_channel();
            super::super::forward_signal(
                tokio::signal::unix::SignalKind::interrupt(),
                "SIGINT",
                events,
            );
            tokio::task::yield_now().await;
        });
        drop(runtime);
        let handoff = std::env::var("AGEND_TEST_STOPFLAG_CHILD").unwrap() == "handoff";
        if handoff {
            begin_exec_handoff();
        }
        // SAFETY: only this isolated child is signalled; its handler is installed.
        assert_eq!(unsafe { libc::raise(libc::SIGINT) }, 0);
        assert!(!handoff, "signal returned during exec handoff");
        assert!(STOP_SIGNALLED.load(Ordering::SeqCst));
    }
}
