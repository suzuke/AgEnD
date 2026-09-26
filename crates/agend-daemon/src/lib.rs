//! AgEnD daemon: the only large I/O layer. Long-running (launchd/systemd), one
//! tokio multi-thread runtime, a dedicated SQLite thread, every external
//! command via `tokio::process` with a timeout (plan §4.1).
//!
//! Layers:
//! - entry: `daemon` (`agend daemon`, gate 6), `server`, `handlers`, `ingest`
//! - domain: `boot`, `fleet` (fleet view and event log, gate 8),
//!   `pipeline`, `delivery`, `supervisor`, `scheduler`, `reconcile`,
//!   `housekeeping`
//! - adapters: `driver`, `runtime`, `forge`, `git`, `runner`, `store`, `notifier`
//!
//! Domain modules talk to adapters only through the traits in
//! `agend_core::traits`, so each can be tested against `agend-testkit` fakes.
//!
//! Must NOT: own agent processes or their side processes (holders do), infer
//! task context from an agent's cwd, or type message text into a PTY.

// entry
#[cfg(unix)]
pub mod daemon;
#[cfg(unix)]
pub mod handlers;
pub mod ingest;
#[cfg(unix)]
pub mod server;

// domain
#[cfg(unix)]
pub mod boot;
pub mod delivery;
#[cfg(unix)]
pub mod fleet;
#[cfg(unix)]
pub mod housekeeping;
pub mod pipeline;
pub mod reconcile;
pub mod scheduler;
#[cfg(unix)]
pub mod supervisor;

// adapters
pub mod driver;
pub mod forge;
pub mod git;
pub mod notifier;
pub mod runner;
#[cfg(unix)]
pub mod runtime;
#[cfg(unix)]
pub mod store;

// the daemon's own log (gate 6 P8)
#[cfg(unix)]
pub mod log;
