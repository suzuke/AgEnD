//! AgEnD core: the pure-logic crate every other crate builds on.
//!
//! Holds the shared types, the public protocol definitions, the boundary
//! traits, the pipeline state machine, the policies and the screen
//! classifier. Everything here is a pure function over data.
//!
//! Must NOT: depend on tokio, rusqlite, or any process/network crate, or on
//! another `agend-*` crate (enforced by `cargo xtask check-deps`); spawn
//! processes, open sockets or files, read env vars, or spawn threads (enforced
//! by clippy via this crate's `clippy.toml`).

pub mod config;
pub mod model;
pub mod pipeline;
pub mod policy;
pub mod protocol;
pub mod screen;
pub mod traits;
