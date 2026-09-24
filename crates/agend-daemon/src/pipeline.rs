//! Drives the core pipeline state machine and performs its side effects:
//! assignment, bindings, submit, checks, approvals, daemon-only merge, cleanup.
//! "Done" is taken from the daemon's own merge record, never inferred from git.
//!
//! Must NOT: re-implement transition rules (they live in `agend_core::pipeline`).
