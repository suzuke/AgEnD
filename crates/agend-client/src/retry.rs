//! Retry while the daemon is restarting: up to 10 seconds, then print a clear
//! message (plan §4.7).
//!
//! Must NOT: retry forever or hide the final failure.

use std::time::Duration;

/// How long a CLI call keeps retrying while the daemon is unreachable.
pub const RESTART_RETRY_WINDOW: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_window_matches_the_plan() {
        assert_eq!(RESTART_RETRY_WINDOW, Duration::from_secs(10));
    }
}
