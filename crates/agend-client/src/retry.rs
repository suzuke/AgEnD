//! Retry while the daemon is restarting: up to 10 seconds, then print a clear
//! message (plan §4.7, gate 8 P7).
//!
//! Retried: the socket is missing, the connection is refused, or it ends
//! before a request was sent (including during `hello`). Not retried: a
//! version mismatch, an error the daemon answered, or a request that was
//! sent and is not marked [`Redo::Safe`].
//!
//! Must NOT: retry forever or hide the final failure.

use std::time::Duration;

/// How long a CLI call keeps retrying while the daemon is unreachable.
pub const RESTART_RETRY_WINDOW: Duration = Duration::from_secs(10);
/// Pause between two connection attempts.
pub const RETRY_EVERY: Duration = Duration::from_millis(100);

/// Whether a request may be sent again after the connection ended with the
/// request already sent (the daemon may or may not have done it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redo {
    /// Doing it twice is the same as once (reads such as `get_fleet`).
    Safe,
    /// It may change something: report "daemon restarted during the request".
    Never,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_window_matches_the_plan() {
        assert_eq!(RESTART_RETRY_WINDOW, Duration::from_secs(10));
        assert_eq!(RETRY_EVERY, Duration::from_millis(100));
    }
}
