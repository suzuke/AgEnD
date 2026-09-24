//! Contract test suites: one suite per `agend_core::traits` trait, run against
//! both the fake and the real implementation so fakes cannot drift (v1 #1483).
//!
//! Must NOT: hand-write wire shapes; build inputs with the real producer (#1493).
