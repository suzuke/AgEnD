//! AgEnD holder: one process per instance that owns the agent's PTY, its
//! rendered screen (alacritty_terminal) and its side processes (codex
//! app-server, opencode serve), so the daemon can restart without the agent
//! noticing (D3). Runs from the same `agend` binary.
//!
//! Known ceiling: if the holder itself is hard-killed, the agent dies with it.
//!
//! Must NOT: contain pipeline/delivery logic, open the DB, or accept message
//! text for the PTY (only single control keys).

pub mod exit;
pub mod pty;
pub mod screen;
pub mod server;
pub mod sidecar;
