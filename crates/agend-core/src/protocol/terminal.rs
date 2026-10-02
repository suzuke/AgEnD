//! Shared holder/client terminal data (D39). Transport and ANSI parsing stay
//! outside core. Coordinates are zero-based, sizes exclude the status row.

use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

/// Includes the terminating newline. Adapters reject the whole frame above it.
pub const MAX_FRAME_LINE: usize = 8 << 20;
pub const MAX_SCREEN_SIDE: u16 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub rows: u16,
    pub columns: u16,
}

impl TerminalSize {
    pub fn is_valid(self) -> bool {
        (1..=MAX_SCREEN_SIDE).contains(&self.rows) && (1..=MAX_SCREEN_SIDE).contains(&self.columns)
    }
}

/// None follows the live screen. Some pins an absolute normal-screen row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalViewport {
    pub top: Option<u64>,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalColor {
    Foreground,
    Background,
    Indexed { index: u8 },
    Rgb { r: u8, g: u8, b: u8 },
}

/// Protocol bits, independent of the parser's internal bit representation.
pub mod style {
    pub const BOLD: u16 = 1;
    pub const DIM: u16 = 1 << 1;
    pub const ITALIC: u16 = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const INVERSE: u16 = 1 << 4;
    pub const HIDDEN: u16 = 1 << 5;
    pub const STRIKEOUT: u16 = 1 << 6;
    pub const DOUBLE_UNDERLINE: u16 = 1 << 7;
    pub const UNDERCURL: u16 = 1 << 8;
    pub const DOTTED_UNDERLINE: u16 = 1 << 9;
    pub const DASHED_UNDERLINE: u16 = 1 << 10;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCell {
    /// The base character and its combining characters; empty for a spacer.
    pub text: String,
    /// 0: wide-character spacer; 1: ordinary cell; 2: wide-character lead.
    pub width: u8,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    pub underline_color: Option<TerminalColor>,
    pub style: u16,
    /// A wide character wrapped before this otherwise blank edge cell.
    pub leading_spacer: bool,
    pub wrap: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCursorShape {
    Block,
    Beam,
    Underline,
    HollowBlock,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCursor {
    pub row: u16,
    pub column: u16,
    pub visible: bool,
    pub shape: TerminalCursorShape,
    pub blinking: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalMouseTracking {
    #[default]
    None,
    Click,
    Drag,
    Motion,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalModes {
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub mouse_tracking: TerminalMouseTracking,
    pub sgr_mouse: bool,
    pub utf8_mouse: bool,
    pub focus_reporting: bool,
    pub line_feed_new_line: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalFrame {
    /// Identity of this holder process, unchanged across daemon reconnects.
    pub generation: String,
    pub revision: u64,
    pub size: TerminalSize,
    pub alternate_screen: bool,
    pub modes: TerminalModes,
    /// Normal-screen absolute row IDs. On alt screen all three are zero.
    pub history_oldest: u64,
    pub live_top: u64,
    pub viewport_top: u64,
    pub viewport_clamped: bool,
    /// Full rows in this viewport; no history outside the requested range.
    pub cells: Vec<Vec<TerminalCell>>,
    /// Relative to the viewport; hidden when the live cursor lies outside it.
    pub cursor: TerminalCursor,
}

/// Request/reply correlation is separate from frame revision and generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalFrameRequest {
    pub request_id: String,
    pub viewport: TerminalViewport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalFrameData {
    pub request_id: String,
    pub frame: TerminalFrame,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOperationError {
    pub request_id: String,
    pub code: String,
    pub message: String,
}

/// A holder operation on the one PTY queue. The daemon owns caller identity
/// and connection-scoped attach IDs; the holder verifies its generation and
/// owner again when the operation reaches the writer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalControlRequest {
    pub request_id: String,
    pub generation: String,
    pub operation: TerminalControlOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum TerminalControlOperation {
    Acquire {
        attach_id: String,
        size: TerminalSize,
    },
    Resize {
        attach_id: String,
        size: TerminalSize,
    },
    Input {
        attach_id: String,
        bytes_base64: String,
    },
    Release {
        attach_id: String,
    },
}

impl TerminalControlOperation {
    pub fn attach_id(&self) -> &str {
        match self {
            Self::Acquire { attach_id, .. }
            | Self::Resize { attach_id, .. }
            | Self::Input { attach_id, .. }
            | Self::Release { attach_id } => attach_id,
        }
    }
}

/// Sent after the actual queue operation finishes, never just after enqueue.
/// Acquire/Resize include the complete frame at the acknowledged PTY size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalControlData {
    pub request_id: String,
    pub generation: String,
    pub attach_id: Option<String>,
    pub frame: Option<TerminalFrame>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_exclude_empty_and_oversized_pty_sizes() {
        for (rows, columns) in [(0, 80), (24, 0), (1001, 80), (24, u16::MAX)] {
            assert!(!TerminalSize { rows, columns }.is_valid());
        }
        for (rows, columns) in [(1, 1), (24, 80), (1000, 1000)] {
            assert!(TerminalSize { rows, columns }.is_valid());
        }
    }
}

/// Coalesced runtime link notice, separate from the holder's frame revision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalNotice {
    pub connection_epoch: u64,
    pub output_sequence: u64,
    pub connected: bool,
}
