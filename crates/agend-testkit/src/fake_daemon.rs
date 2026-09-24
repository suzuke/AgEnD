//! Fake daemon server speaking protocol v1, for testing `agend-client`, the CLI
//! and the TUI without a real daemon.
//!
//! Must NOT: share code paths with the real server beyond `agend_core::protocol`.
