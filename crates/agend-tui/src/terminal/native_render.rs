//! Preserve underline shapes that Ratatui's Modifier cannot represent.
//! This adapter emits typed commands, never replays agent ANSI. Its shadow
//! follows both the protocol cells and Ratatui's actual diff, including wide
//! cell invalidation; style-only transitions must not depend on a text change.
use super::full;
use crate::App;
use agend_core::protocol::terminal::{TerminalCell, TerminalColor, style};
use ratatui::{
    buffer::Buffer,
    crossterm::{
        cursor::{Hide, MoveTo, SetCursorStyle, Show},
        queue,
        style::{
            Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor,
            SetUnderlineColor,
        },
    },
    layout::Position,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
};

const UNDERLINES: u16 = style::UNDERLINE
    | style::DOUBLE_UNDERLINE
    | style::UNDERCURL
    | style::DOTTED_UNDERLINE
    | style::DASHED_UNDERLINE;

#[derive(Default)]
pub struct CellRenderer {
    previous: Option<Buffer>,
    styled: BTreeMap<(u16, u16), TerminalCell>,
    cursor_style: Option<SetCursorStyle>,
}
struct PaintedCell {
    x: u16,
    y: u16,
    source: TerminalCell,
    text: String,
}
/// Prepared before releasing the completed frame's borrow of Terminal.
pub struct Paint {
    cells: Vec<PaintedCell>,
    cursor: Option<Position>,
    cursor_style: Option<SetCursorStyle>,
}
impl CellRenderer {
    pub fn prepare(&mut self, app: &App, buffer: &Buffer) -> Paint {
        let same_area = self
            .previous
            .as_ref()
            .is_some_and(|old| old.area == buffer.area);
        let overwritten: BTreeSet<_> = self
            .previous
            .as_ref()
            .filter(|_| same_area)
            .map(|old| {
                old.diff(buffer)
                    .into_iter()
                    .map(|(x, y, _)| (x, y))
                    .collect()
            })
            .unwrap_or_default();
        let mut styled = BTreeMap::new();
        let mut cells = Vec::new();
        if let Some((frame, content, offset)) = full::view(app, buffer.area) {
            for (row, line) in frame
                .cells
                .iter()
                .skip(offset)
                .take(usize::from(content.height))
                .enumerate()
            {
                for (column, source) in line.iter().take(usize::from(content.width)).enumerate() {
                    if source.width == 0 || source.style & UNDERLINES == 0 {
                        continue;
                    }
                    let position = (content.x + column as u16, content.y + row as u16);
                    if !same_area
                        || self.styled.get(&position) != Some(source)
                        || overwritten.contains(&position)
                    {
                        cells.push(PaintedCell {
                            x: position.0,
                            y: position.1,
                            source: source.clone(),
                            // The buffer has already replaced clipped wide leads with spaces.
                            text: buffer[position].symbol().into(),
                        });
                    }
                    styled.insert(position, source.clone());
                }
            }
        }
        // No extra full-screen shadow when the native extension is inactive.
        self.previous = (!styled.is_empty()).then(|| buffer.clone());
        self.styled = styled;
        let selected = full::cursor_style(app);
        let cursor_style = (self.cursor_style != Some(selected)).then_some(selected);
        self.cursor_style = Some(selected);
        Paint {
            cells,
            cursor: full::cursor_position(app, buffer.area),
            cursor_style,
        }
    }
}
impl Paint {
    pub fn write(self, mut writer: impl Write) -> io::Result<()> {
        if let Some(shape) = self.cursor_style {
            queue!(writer, shape)?;
        }
        if !self.cells.is_empty() {
            queue!(writer, Hide)?;
            for cell in self.cells {
                queue!(
                    writer,
                    MoveTo(cell.x, cell.y),
                    SetAttribute(Attribute::Reset),
                    SetForegroundColor(color(cell.source.foreground)),
                    SetBackgroundColor(color(cell.source.background)),
                    SetUnderlineColor(cell.source.underline_color.map_or(Color::Reset, color))
                )?;
                for (flag, attribute) in [
                    (style::BOLD, Attribute::Bold),
                    (style::DIM, Attribute::Dim),
                    (style::ITALIC, Attribute::Italic),
                    (style::INVERSE, Attribute::Reverse),
                    (style::HIDDEN, Attribute::Hidden),
                    (style::STRIKEOUT, Attribute::CrossedOut),
                ] {
                    if cell.source.style & flag != 0 {
                        queue!(writer, SetAttribute(attribute))?;
                    }
                }
                let underline = if cell.source.style & style::DOUBLE_UNDERLINE != 0 {
                    Attribute::DoubleUnderlined
                } else if cell.source.style & style::UNDERCURL != 0 {
                    Attribute::Undercurled
                } else if cell.source.style & style::DOTTED_UNDERLINE != 0 {
                    Attribute::Underdotted
                } else if cell.source.style & style::DASHED_UNDERLINE != 0 {
                    Attribute::Underdashed
                } else {
                    Attribute::Underlined
                };
                queue!(writer, SetAttribute(underline), Print(cell.text))?;
            }
            // Keep the next Ratatui diff's initial style and its tracked cursor valid.
            queue!(
                writer,
                SetAttribute(Attribute::Reset),
                SetForegroundColor(Color::Reset),
                SetBackgroundColor(Color::Reset),
                SetUnderlineColor(Color::Reset)
            )?;
            if let Some(Position { x, y }) = self.cursor {
                queue!(writer, MoveTo(x, y), Show)?;
            }
        }
        writer.flush()
    }
}
fn color(color: TerminalColor) -> Color {
    match color {
        TerminalColor::Foreground | TerminalColor::Background => Color::Reset,
        TerminalColor::Indexed { index } => Color::AnsiValue(index),
        TerminalColor::Rgb { r, g, b } => Color::Rgb { r, g, b },
    }
}
