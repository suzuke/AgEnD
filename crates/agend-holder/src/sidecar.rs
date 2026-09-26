//! Side processes (codex app-server, opencode serve) are NOT in gate 4: they
//! move to gate 7 (gate 4 P8), with a new `SpawnSidecar` request and the
//! readiness check in the daemon's driver. In v1 the daemon spawned them
//! (`src/transport/codex_app_server.rs:286-316`,
//! `src/transport/opencode_server.rs:452-500`), which v2 must change so they
//! survive a daemon restart.
//!
//! Must NOT: speak the backend's protocol (the daemon's driver does).
