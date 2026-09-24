//! Spawns the agent on a PTY (portable-pty), injects its environment (identity,
//! home, PATH with the shims), reads output, resizes, signals.
//!
//! Must NOT: write anything to the PTY other than a single control key.

use agend_core::protocol::holder::ControlKey;

/// Bytes written to the PTY for a control key.
pub fn control_key_bytes(key: ControlKey) -> Option<&'static [u8]> {
    Some(match key {
        ControlKey::Esc => b"\x1b",
        ControlKey::Enter => b"\r",
        ControlKey::Up => b"\x1b[A",
        ControlKey::Down => b"\x1b[B",
        ControlKey::Right => b"\x1b[C",
        ControlKey::Left => b"\x1b[D",
        ControlKey::Digit1 => b"1",
        ControlKey::Digit2 => b"2",
        ControlKey::Digit3 => b"3",
        ControlKey::Y => b"y",
        ControlKey::N => b"n",
        ControlKey::Unknown => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_keys_are_single_bytes() {
        assert_eq!(control_key_bytes(ControlKey::Esc), Some(&[0x1b][..]));
        assert_eq!(control_key_bytes(ControlKey::Digit1), Some(b"1".as_slice()));
        assert_eq!(control_key_bytes(ControlKey::Unknown), None);
    }
}
