//! AgEnD core: the pure-logic crate every other crate builds on.
//!
//! Holds the shared types, the public protocol definitions, the boundary
//! traits, the pipeline state machine, the policies and the screen
//! classifier. Everything here is a pure function over data.
//!
//! The crate is `#![no_std]` + `alloc` with `#![forbid(unsafe_code)]`; time
//! comes in only through the `Clock` trait. `cargo xtask check-deps` builds it
//! for a target without std (all features, `-F unsafe-code`) and checks via
//! `cargo metadata` that it has no build script, no features and no
//! dependencies. These guards stop accidental I/O, not deliberate evasion.
//!
//! Must NOT: perform I/O of any kind, or gain a build script or dependency.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod config;
pub mod model;
pub mod pipeline;
pub mod policy;
pub mod protocol;
pub mod screen;
pub mod setup;
pub mod traits;
