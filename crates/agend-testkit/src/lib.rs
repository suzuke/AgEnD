//! AgEnD testkit: shared test infrastructure. Used only as a
//! `[dev-dependencies]` entry (D10 criterion 4; checked by `cargo xtask check-deps`).
//!
//! Must NOT: be a normal dependency of any crate, or contain production logic.

pub mod contract;
pub mod fake_agent;
pub mod fake_daemon;
pub mod fakes;
pub mod git_fixture;
pub mod tempdir;
