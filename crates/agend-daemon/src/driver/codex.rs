//! codex driver (gate 7): JSON-RPC over WebSocket over a unix socket to
//! `codex app-server`, which runs in the instance's holder next to the TUI
//! (`launch`: the `sh` wrapper, P2). Queue with `thread/queue/add`, steer
//! with `turn/steer`, interrupt with `turn/interrupt` (`send`, P6); after a
//! reconnect, `thread/resume` then `thread/turns/list` (`link`, P7).
//!
//! - [`CodexDriver`]: the `Driver` trait over the `messages` table (the one
//!   idempotency layer, P5) and one link per instance.
//! - `history`: the thread history as the event log, and matching user
//!   messages to our messages (P5, P7).
//! - `sweep`: the SIGKILL of a dead holder's leftover codex group (P2).
//!
//! Must NOT: call `thread/queue/start` after `thread/queue/add`, except
//! once when the reply finds the thread already idle (owner-approved P6
//! exception: codex may not have started it); connect to the literal
//! `--listen` path; write anything under `~/.codex`.

pub mod driver;
pub mod history;
pub mod launch;
pub(crate) mod link;
pub mod rpc;
pub mod send;
pub mod sweep;

pub use driver::{CodexDriver, DriverError};
pub use link::{CodexEvent, CodexSink};

use std::io;
use std::path::{Path, PathBuf};

/// Path to actually connect to for a codex app-server socket.
///
/// When the `--listen unix://<path>` path is longer than the AF_UNIX limit,
/// codex binds a short socket elsewhere and leaves a symlink at `<path>`;
/// connecting to the literal path fails. Always resolve first.
pub fn socket_connect_path(listen_path: &Path) -> io::Result<PathBuf> {
    std::fs::canonicalize(listen_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;

    #[test]
    fn follows_symlink_to_the_real_socket_path() {
        let dir = TempDir::new("codex-sock").unwrap();
        let real = dir.path().join("real.sock");
        std::fs::write(&real, b"").unwrap();
        let link = dir.path().join("a-very-long-listen-path.sock");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let resolved = socket_connect_path(&link).unwrap();
        assert_eq!(resolved, std::fs::canonicalize(&real).unwrap());
    }

    #[test]
    fn missing_socket_is_an_error_not_a_guess() {
        let dir = TempDir::new("codex-sock-missing").unwrap();
        assert!(socket_connect_path(&dir.path().join("absent.sock")).is_err());
    }
}
