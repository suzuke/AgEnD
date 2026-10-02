//! Structured viewport extraction from the authoritative alacritty grid.

use agend_core::protocol::terminal::*;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};

use super::{FrameError, Screen};

impl Screen {
    pub fn validate_frame_size(size: TerminalSize) -> Result<(), FrameError> {
        if !size.is_valid() {
            return Err(FrameError::InvalidSize);
        }
        // A conservative lower bound prevents allocating a million cells
        // for a frame which could never fit on the wire. Serialization still
        // checks the actual line (combining characters can exceed this bound).
        static MIN_CELL_BYTES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        let minimum = MIN_CELL_BYTES.get_or_init(|| {
            serde_json::to_vec(&TerminalCell {
                text: String::new(),
                width: 0,
                foreground: TerminalColor::Foreground,
                background: TerminalColor::Background,
                underline_color: None,
                style: 0,
                leading_spacer: true,
                wrap: true,
            })
            .expect("terminal cell serializes")
            .len()
        });
        if usize::from(size.rows) * usize::from(size.columns) * minimum > MAX_FRAME_LINE {
            return Err(FrameError::TooLarge);
        }
        Ok(())
    }

    /// Does not scroll the parser or change its classifier's visible screen.
    /// The caller holds the screen lock while extracting the whole frame.
    pub fn frame(&self, viewport: TerminalViewport) -> Result<TerminalFrame, FrameError> {
        if viewport.rows == 0 || viewport.rows > self.rows {
            return Err(FrameError::InvalidSize);
        }
        Self::validate_frame_size(TerminalSize {
            rows: viewport.rows,
            columns: self.columns,
        })?;
        let grid = self.term.grid();
        let mode = *self.term.mode();
        let alternate_screen = mode.contains(TermMode::ALT_SCREEN);
        let live_top = if alternate_screen {
            0
        } else {
            self.history.live_top
        };
        let history_oldest = live_top.saturating_sub(grid.history_size() as u64);
        let requested = viewport.top.unwrap_or(live_top);
        let viewport_top = if alternate_screen {
            0
        } else {
            requested.clamp(history_oldest, live_top)
        };
        let first = (viewport_top as i128 - live_top as i128) as i32;
        let cells = (first..first + i32::from(viewport.rows))
            .map(|line| {
                (0..grid.columns())
                    .map(|column| self.cell(&grid[Line(line)][Column(column)]))
                    .collect()
            })
            .collect();
        let cursor = grid.cursor.point;
        let cursor_row = cursor.line.0 - first;
        let cursor_style = self.term.cursor_style();
        Ok(TerminalFrame {
            generation: self.generation.clone(),
            revision: self.revision,
            size: TerminalSize {
                rows: self.rows,
                columns: self.columns,
            },
            alternate_screen,
            modes: TerminalModes {
                application_cursor: mode.contains(TermMode::APP_CURSOR),
                application_keypad: mode.contains(TermMode::APP_KEYPAD),
                bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
                mouse_tracking: if mode.contains(TermMode::MOUSE_MOTION) {
                    TerminalMouseTracking::Motion
                } else if mode.contains(TermMode::MOUSE_DRAG) {
                    TerminalMouseTracking::Drag
                } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
                    TerminalMouseTracking::Click
                } else {
                    TerminalMouseTracking::None
                },
                sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
                utf8_mouse: mode.contains(TermMode::UTF8_MOUSE),
                focus_reporting: mode.contains(TermMode::FOCUS_IN_OUT),
                line_feed_new_line: mode.contains(TermMode::LINE_FEED_NEW_LINE),
            },
            history_oldest,
            live_top,
            viewport_top,
            viewport_clamped: viewport.top.is_some() && viewport_top != requested,
            cells,
            cursor: TerminalCursor {
                row: cursor_row.clamp(0, i32::from(viewport.rows) - 1) as u16,
                column: cursor.column.0 as u16,
                visible: cursor_row >= 0
                    && cursor_row < i32::from(viewport.rows)
                    && mode.contains(TermMode::SHOW_CURSOR)
                    && cursor_style.shape != CursorShape::Hidden,
                shape: match cursor_style.shape {
                    CursorShape::Block => TerminalCursorShape::Block,
                    CursorShape::Beam => TerminalCursorShape::Beam,
                    CursorShape::Underline => TerminalCursorShape::Underline,
                    CursorShape::HollowBlock => TerminalCursorShape::HollowBlock,
                    CursorShape::Hidden => TerminalCursorShape::Hidden,
                },
                blinking: cursor_style.blinking,
            },
        })
    }

    fn cell(&self, cell: &Cell) -> TerminalCell {
        let flags = cell.flags;
        let spacer = flags.contains(Flags::WIDE_CHAR_SPACER);
        let mut text = if spacer {
            String::new()
        } else {
            if cell.c == '\t' {
                " ".into()
            } else {
                cell.c.to_string()
            }
        };
        if !spacer && let Some(extra) = cell.zerowidth() {
            text.extend(extra);
        }
        let mut style = 0;
        for (flag, bit) in [
            (Flags::BOLD, style::BOLD),
            (Flags::DIM, style::DIM),
            (Flags::ITALIC, style::ITALIC),
            (Flags::UNDERLINE, style::UNDERLINE),
            (Flags::INVERSE, style::INVERSE),
            (Flags::HIDDEN, style::HIDDEN),
            (Flags::STRIKEOUT, style::STRIKEOUT),
            (Flags::DOUBLE_UNDERLINE, style::DOUBLE_UNDERLINE),
            (Flags::UNDERCURL, style::UNDERCURL),
            (Flags::DOTTED_UNDERLINE, style::DOTTED_UNDERLINE),
            (Flags::DASHED_UNDERLINE, style::DASHED_UNDERLINE),
        ] {
            if flags.contains(flag) {
                style |= bit;
            }
        }
        TerminalCell {
            text,
            width: if spacer {
                0
            } else if flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            },
            foreground: self.color(cell.fg),
            background: self.color(cell.bg),
            underline_color: cell.underline_color().map(|color| self.color(color)),
            style,
            leading_spacer: flags.contains(Flags::LEADING_WIDE_CHAR_SPACER),
            wrap: flags.contains(Flags::WRAPLINE),
        }
    }

    fn color(&self, color: Color) -> TerminalColor {
        let rgb = match color {
            Color::Spec(rgb) => Some(rgb),
            Color::Indexed(index) => self.term.colors()[index as usize],
            Color::Named(name) => self.term.colors()[name],
        };
        if let Some(rgb) = rgb {
            return TerminalColor::Rgb {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
            };
        }
        match color {
            Color::Spec(_) => unreachable!("RGB handled above"),
            Color::Indexed(index) => TerminalColor::Indexed { index },
            Color::Named(NamedColor::Background) => TerminalColor::Background,
            Color::Named(name) if (name as usize) < 16 => {
                TerminalColor::Indexed { index: name as u8 }
            }
            Color::Named(name) if (259..267).contains(&(name as usize)) => TerminalColor::Indexed {
                index: (name as usize - 259) as u8,
            },
            Color::Named(_) => TerminalColor::Foreground,
        }
    }
}
