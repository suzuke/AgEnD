//! AgEnD client: synchronous I/O connection to the daemon, retry, protocol
//! version check (D11). Shared by the CLI, the TUI and a future Rust GUI.
//!
//! Must NOT: start an async runtime, read config files, or open the DB; CLI
//! startup must stay light (measured p50 4.1 ms, unix-socket round trip
//! 0.014 ms, plan §4.7).

pub mod connection;
pub mod retry;
pub mod version;
