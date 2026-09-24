//! Forges: how a branch is submitted and merged (D4). v2.0 ships `local` and
//! `github`.
//!
//! Must NOT: let agents merge; only the daemon merges.

pub mod github;
pub mod local;
