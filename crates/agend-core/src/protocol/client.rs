//! Client protocol v1: what TUI, CLI and future GUIs exchange with the daemon
//! over its unix socket.
//!
//! Must NOT: contain transport code (sockets live in `agend-client` and the
//! daemon's `server` module).
