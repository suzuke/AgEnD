//! AgEnD daemon: the only large I/O layer. Long-running (launchd/systemd), one
//! tokio multi-thread runtime, a dedicated SQLite thread, every external
//! command via `tokio::process` with a timeout (plan §4.1).
//!
//! Layers:
//! - entry: `server`, `handlers`, `ingest`
//! - domain: `pipeline`, `delivery`, `supervisor`, `scheduler`, `reconcile`
//! - adapters: `driver`, `runtime`, `forge`, `git`, `runner`, `store`, `notifier`
//!
//! Domain modules talk to adapters only through the traits in
//! `agend_core::traits`, so each can be tested against `agend-testkit` fakes.
//!
//! Must NOT: own agent processes or their side processes (holders do), infer
//! task context from an agent's cwd, or type message text into a PTY.

// entry
pub mod handlers;
pub mod ingest;
pub mod server;

// domain
pub mod delivery;
pub mod pipeline;
pub mod reconcile;
pub mod scheduler;
pub mod supervisor;

// adapters
pub mod driver;
pub mod forge;
pub mod git;
pub mod notifier;
pub mod runner;
pub mod runtime;
pub mod store;
