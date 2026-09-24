//! codex driver: JSON-RPC over WebSocket over a unix socket to
//! `codex app-server` (held by the holder). Queue with `thread/queue/add`,
//! steer with `turn/steer`, interrupt with `turn/interrupt`; after reconnect,
//! `thread/resume` then `thread/turns/list`.
//!
//! Must NOT: call `thread/queue/start` after `thread/queue/add` (the server
//! dequeues automatically), or connect to the literal `--listen` path.

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
