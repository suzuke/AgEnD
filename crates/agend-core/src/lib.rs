//! AgEnD core: the pure-logic crate every other crate builds on.
//!
//! Holds the shared types, the public protocol definitions, the boundary
//! traits, the pipeline state machine, the policies and the screen
//! classifier. Everything here is a pure function over data.
//!
//! The crate is `#![no_std]` + `alloc`: the compiler itself guarantees no
//! filesystem, process, network, env, thread, stdio or clock access. Time
//! comes in only through the `Clock` trait. Dependency rules (no tokio,
//! rusqlite, process/network crates, other `agend-*`) are checked by
//! `cargo xtask check-deps`, which also fails if `#![no_std]` is removed.
//!
//! Must NOT: gain a `std` feature or dependency that reintroduces I/O.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod config;
pub mod model;
pub mod pipeline;
pub mod policy;
pub mod protocol;
pub mod screen;
pub mod traits;
