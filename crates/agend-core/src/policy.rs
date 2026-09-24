//! Pure policies used by the daemon's domain modules.
//!
//! Must NOT: perform I/O or keep state between calls (callers pass state in).

pub mod assign;
pub mod busy;
pub mod conflict;
pub mod debounce;
pub mod merge_gate;
