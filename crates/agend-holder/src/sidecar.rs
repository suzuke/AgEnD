//! Starts and owns the backend's side process (codex app-server, opencode
//! serve) so it survives a daemon restart. In v1 the daemon spawned them
//! (`src/transport/codex_app_server.rs:286-316`,
//! `src/transport/opencode_server.rs:452-500`), which v2 must change.
//!
//! Must NOT: speak the backend's protocol (the daemon's driver does).
