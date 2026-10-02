//! Protocol version check against the daemon at connect time (gate 8 P3,
//! gate 9 P4). This client needs 1.3 (pipeline commands and action notes); a daemon that negotiates less is an old binary
//! that must be restarted, so it fails at once without retrying. A daemon
//! with `daemon_restart` (1.2 and later) is told to restart itself; an older
//! one must be stopped and started by hand.
//!
//! Must NOT: silently continue on an incompatible version.

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::client::{V1_2, V1_3};

/// The oldest version this client works with.
pub const NEEDED: ProtocolVersion = V1_3;
/// The first version with `daemon_restart` (its shape never changes).
pub const RESTART_SINCE: ProtocolVersion = V1_2;

/// `Err` with the message to print when the daemon selected `selected`.
pub fn check(selected: ProtocolVersion) -> Result<(), String> {
    check_at_least(selected, NEEDED)
}

/// [`check`] against another floor (`agend daemon restart` needs only
/// [`RESTART_SINCE`], so a newer CLI can still restart an older daemon).
pub fn check_at_least(selected: ProtocolVersion, needed: ProtocolVersion) -> Result<(), String> {
    if selected.major == needed.major && selected.minor >= needed.minor {
        return Ok(());
    }
    let what = if selected.major == RESTART_SINCE.major && selected.minor >= RESTART_SINCE.minor {
        "run: agend daemon restart"
    } else {
        "stop the daemon (Ctrl-C) and start this binary: agend daemon"
    };
    Err(format!(
        "the daemon speaks client protocol {}.{}; this agend needs {}.{} — {what}",
        selected.major, selected.minor, needed.major, needed.minor
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::protocol::client::{V1, V1_1};

    #[test]
    fn an_older_daemon_is_refused_with_what_to_do() {
        assert_eq!(check(V1_3), Ok(()));
        assert_eq!(check(ProtocolVersion::new(1, 7)), Ok(()));
        assert_eq!(
            check(V1).unwrap_err(),
            "the daemon speaks client protocol 1.0; this agend needs 1.3 — stop the daemon (Ctrl-C) and start this binary: agend daemon"
        );
        assert_eq!(
            check(V1_1).unwrap_err(),
            "the daemon speaks client protocol 1.1; this agend needs 1.3 — stop the daemon (Ctrl-C) and start this binary: agend daemon"
        );
        // A later CLI that needs 1.3 tells a 1.2 daemon to restart itself.
        assert_eq!(
            check_at_least(V1_2, ProtocolVersion::new(1, 3)).unwrap_err(),
            "the daemon speaks client protocol 1.2; this agend needs 1.3 — run: agend daemon restart"
        );
        assert_eq!(check_at_least(V1_2, RESTART_SINCE), Ok(()));
    }
}
