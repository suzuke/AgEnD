//! AgEnD testkit: shared test infrastructure. Used only as a
//! `[dev-dependencies]` entry (D10 criterion 4; checked by `cargo xtask check-deps`).
//!
//! - `fakes`: one deterministic, inspectable, scriptable fake per
//!   `agend_core::traits` trait.
//! - `contract`: one contract suite per trait, written against a fixture
//!   trait so the same cases run on the fake now and on the real
//!   implementation at its gate (v1 #1483).
//! - `fake_daemon`: in-process client protocol 1.1 server (unix only).
//! - `fake_agent`: the fake backend programs (`fake-codex-app-server`,
//!   `fake-opencode-serve`, `fake-claude`); the binaries in `src/bin/` are
//!   thin wrappers around these modules.
//! - `recorder`: drives the real backend CLIs (and the fakes) through fixed
//!   scenarios and records transcripts; the conformance test compares the
//!   fakes to the recordings by shape.
//!
//! Must NOT: be a normal dependency of any crate, or contain production logic.

pub mod contract;
pub mod executor;
#[cfg(unix)]
pub mod fake_agent;
#[cfg(unix)]
pub mod fake_daemon;
pub mod fakes;
pub mod git_fixture;
#[cfg(unix)]
pub mod recorder;
pub mod tempdir;

pub use executor::block_on;
