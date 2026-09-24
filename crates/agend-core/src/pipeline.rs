//! Pipeline state machine as pure functions (D11): workflows are ordered
//! compositions of six basic stages, stored in the DB with versions. Adding a
//! composition or changing parameters needs no code change.
//!
//! Must NOT: execute anything. The daemon's `pipeline` module drives this
//! state machine and performs the side effects.

pub mod stage;
pub mod task;
pub mod workflow;
