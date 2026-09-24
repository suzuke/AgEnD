//! Backend drivers: turn delivery requests into each backend's structured API
//! and normalise its events into busy/idle/exited (see docs/BACKEND-BEHAVIORS.md).
//!
//! Must NOT: spawn backend processes (holders do) or type message text into a PTY.

pub mod claude;
pub mod codex;
pub mod opencode;
