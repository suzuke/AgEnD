//! Protocol version check against the daemon at connect time (gate 8 P3).
//! This client needs 1.1 (`get_fleet`); a daemon that negotiates 1.0 is an
//! old binary that must be restarted, so it fails at once without retrying.
//!
//! Must NOT: silently continue on an incompatible version.

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::client::V1_1;

/// The oldest version this client works with.
pub const NEEDED: ProtocolVersion = V1_1;

/// `Err` with the message to print when the daemon selected `selected`.
pub fn check(selected: ProtocolVersion) -> Result<(), String> {
    if selected.major == NEEDED.major && selected.minor >= NEEDED.minor {
        return Ok(());
    }
    Err(format!(
        "the daemon speaks client protocol {}.{}; this agend needs {}.{} — restart the daemon with this binary",
        selected.major, selected.minor, NEEDED.major, NEEDED.minor
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::protocol::client::V1;

    #[test]
    fn a_1_0_daemon_is_refused_with_what_to_do() {
        assert_eq!(check(V1_1), Ok(()));
        assert_eq!(check(ProtocolVersion::new(1, 7)), Ok(()));
        assert_eq!(
            check(V1).unwrap_err(),
            "the daemon speaks client protocol 1.0; this agend needs 1.1 — restart the daemon with this binary"
        );
    }
}
