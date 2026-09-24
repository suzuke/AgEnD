//! Spawns the agent on a PTY (portable-pty), injects its environment (identity,
//! home, PATH with the shims), reads output, resizes, signals.
//!
//! Must NOT: write anything to the PTY other than a single control key.

use agend_core::protocol::holder::ControlKey;

/// Bytes written to the PTY for a control key.
pub fn control_key_bytes(key: ControlKey) -> &'static [u8] {
    match key {
        ControlKey::Esc => b"\x1b",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_keys_are_single_bytes() {
        assert_eq!(control_key_bytes(ControlKey::Esc), [0x1b]);
    }
}
