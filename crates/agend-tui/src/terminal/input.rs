//! xterm key encoding from observed key semantics and holder modes.
use agend_core::protocol::terminal::{TerminalModes, TerminalMouseTracking};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventState, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

pub fn key_bytes(key: &KeyEvent, modes: TerminalModes) -> Vec<u8> {
    let modifier = 1
        + u8::from(key.modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(key.modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(key.modifiers.contains(KeyModifiers::CONTROL));
    if modes.application_keypad && key.state.contains(KeyEventState::KEYPAD) {
        let suffix = match key.code {
            KeyCode::Char('0'..='9') => match key.code {
                KeyCode::Char(c) => Some(b'p' + (c as u8 - b'0')),
                _ => None,
            },
            KeyCode::Char('.') => Some(b'n'),
            KeyCode::Char('/') => Some(b'o'),
            KeyCode::Char('*') => Some(b'j'),
            KeyCode::Char('-') => Some(b'm'),
            KeyCode::Char('+') => Some(b'k'),
            KeyCode::Char('=') => Some(b'X'),
            KeyCode::Enter => Some(b'M'),
            _ => None,
        };
        if let Some(suffix) = suffix {
            return sequence(suffix as char, true, modifier);
        }
    }
    let cursor = match key.code {
        KeyCode::Up => Some('A'),
        KeyCode::Down => Some('B'),
        KeyCode::Right => Some('C'),
        KeyCode::Left => Some('D'),
        KeyCode::Home => Some('H'),
        KeyCode::End => Some('F'),
        KeyCode::KeypadBegin => Some('E'),
        _ => None,
    };
    if let Some(suffix) = cursor {
        return sequence(suffix, modes.application_cursor, modifier);
    }
    if let KeyCode::F(1..=4) = key.code {
        let KeyCode::F(n) = key.code else {
            unreachable!()
        };
        return sequence((b'P' + n - 1) as char, true, modifier);
    }
    let tilde = match key.code {
        KeyCode::Insert => Some(2),
        KeyCode::Delete => Some(3),
        KeyCode::PageUp => Some(5),
        KeyCode::PageDown => Some(6),
        KeyCode::F(n @ 5..=12) => Some([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        _ => None,
    };
    if let Some(number) = tilde {
        return if modifier == 1 {
            format!("\x1b[{number}~")
        } else {
            format!("\x1b[{number};{modifier}~")
        }
        .into_bytes();
    }
    if key.code == KeyCode::Enter && modes.line_feed_new_line {
        return b"\r\n".to_vec();
    }
    crate::app::key_bytes(key)
}
fn sequence(suffix: char, application: bool, modifier: u8) -> Vec<u8> {
    if modifier > 1 {
        format!("\x1b[1;{modifier}{suffix}")
    } else if application {
        format!("\x1bO{suffix}")
    } else {
        format!("\x1b[{suffix}")
    }
    .into_bytes()
}

/// Encode a content-relative, zero-based mouse event. Unrepresentable legacy
/// coordinates are ignored rather than truncated or wrapped into another cell.
pub fn mouse_bytes(event: MouseEvent, modes: TerminalModes) -> Vec<u8> {
    use MouseEventKind::*;
    use TerminalMouseTracking::{Motion, None};
    let button = |button| match button {
        MouseButton::Left => 0u8,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let (mut code, release) = match event.kind {
        _ if modes.mouse_tracking == None => return Vec::new(),
        Down(b) => (button(b), false),
        Up(b) => (if modes.sgr_mouse { button(b) } else { 3 }, true),
        Drag(b) if matches!(modes.mouse_tracking, TerminalMouseTracking::Drag | Motion) => {
            (button(b) + 32, false)
        }
        Moved if modes.mouse_tracking == Motion => (35, false),
        ScrollUp => (64, false),
        ScrollDown => (65, false),
        ScrollRight => (66, false),
        ScrollLeft => (67, false),
        _ => return Vec::new(),
    };
    code += 4 * u8::from(event.modifiers.contains(KeyModifiers::SHIFT))
        + 8 * u8::from(event.modifiers.contains(KeyModifiers::ALT))
        + 16 * u8::from(event.modifiers.contains(KeyModifiers::CONTROL));
    let x = u32::from(event.column) + 1;
    let y = u32::from(event.row) + 1;
    if modes.sgr_mouse {
        return format!("\x1b[<{code};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes();
    }
    let limit = if modes.utf8_mouse { 2015 } else { 223 };
    if x > limit || y > limit {
        return Vec::new();
    }
    let mut bytes = b"\x1b[M".to_vec();
    for value in [u32::from(code) + 32, x + 32, y + 32] {
        if modes.utf8_mouse {
            let mut buffer = [0; 4];
            bytes.extend_from_slice(
                char::from_u32(value)
                    .unwrap()
                    .encode_utf8(&mut buffer)
                    .as_bytes(),
            );
        } else {
            bytes.push(value as u8);
        }
    }
    bytes
}
