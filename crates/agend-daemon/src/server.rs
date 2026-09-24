//! Protocol server: serves client protocol v1 on the daemon's unix socket
//! (a WebSocket listener is added only when a GUI needs it).
//!
//! Must NOT: contain command logic (that is `handlers`).
