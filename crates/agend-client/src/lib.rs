//! AgEnD client: synchronous I/O connection to the daemon, retry, protocol
//! version check (D11). Shared by the CLI, the TUI and a future Rust GUI.
//!
//! - [`Client::connect`]: connect and `hello`, retrying every 100 ms for up
//!   to 10 s while the daemon is restarting (gate 8 P7);
//!   [`Client::connect_once`]: one attempt, for callers with their own
//!   reconnect loop (the TUI, `agend debug watch`).
//! - [`Client::request`]: send, wait for the reply with the same request id
//!   (10 s); after a disconnect only [`Redo::Safe`] requests are sent again.
//! - [`Client::next_event`]: the next event after `subscribe_events`.
//! - Errors ([`ClientError`]): unreachable, version mismatch, or an error the
//!   daemon answered (with its `error_code`), so the CLI picks its message
//!   and exit code.
//!
//! The caller passes the socket path and the caller identity; this crate
//! reads no environment variable and no file.
//!
//! Must NOT: start an async runtime, read config files, or open the DB; CLI
//! startup must stay light (measured p50 4.1 ms, unix-socket round trip
//! 0.014 ms, plan §4.7).

pub mod connection;
pub mod retry;
pub mod version;

use std::fmt;
use std::path::PathBuf;

pub use connection::Client;
pub use retry::{RESTART_RETRY_WINDOW, Redo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// No daemon answered within [`RESTART_RETRY_WINDOW`] ([`Client::connect`]).
    Unreachable { socket: PathBuf, cause: String },
    /// One attempt failed ([`Client::connect_once`]), or an error that
    /// retrying cannot fix (for example permission denied).
    Connect { socket: PathBuf, cause: String },
    /// The daemon speaks an incompatible protocol version; not retried.
    Version(String),
    /// The daemon answered an error (`code` is a `client::error_code`).
    Daemon { code: String, message: String },
    /// The connection ended after a request that is not [`Redo::Safe`] was
    /// sent: it may or may not have been done.
    Restarted,
    /// The connection ended or broke while reading (events, a reply that
    /// never came, a line that is not the protocol).
    Disconnected(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable { socket, cause } => write!(
                f,
                "cannot reach the AgEnD daemon at {} after {} s ({cause}). Is it running? Start it with: agend daemon",
                socket.display(),
                RESTART_RETRY_WINDOW.as_secs()
            ),
            Self::Connect { socket, cause } => write!(
                f,
                "cannot reach the AgEnD daemon at {} ({cause})",
                socket.display()
            ),
            Self::Version(message) => f.write_str(message),
            Self::Daemon { code, message } => write!(f, "{code}: {message}"),
            Self::Restarted => {
                f.write_str("daemon restarted during the request; check with agend status")
            }
            Self::Disconnected(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ClientError {}
