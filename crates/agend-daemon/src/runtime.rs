//! Runtime adapter: the daemon-side client of the holder protocol. On daemon
//! restart it reconnects to every holder and takes the holder's current
//! screen (no byte replay).
//!
//! Must NOT: own the PTY or the agent process.
