//! No side-process protocol (gate 7 P2 overturned gate 4 P8's
//! `SpawnSidecar`): codex's app-server is started by the agent itself, a
//! fixed `sh` wrapper in the PTY (`agend_daemon::driver::codex::launch`), in
//! the agent's process group, so every guarantee this crate gives the PTY
//! child covers it; opencode (gate 12) is expected to do the same.
//!
//! Must NOT: grow a second child-process manager here.
