//! Draw authoritative terminal cells without dashboard padding or ANSI replay.
use crate::{App, app::TermMode, i18n::Text};
use agend_core::protocol::terminal::{TerminalColor, TerminalCursorShape, TerminalFrame, style};
use ratatui::crossterm::cursor::SetCursorStyle;
use ratatui::{
    Frame,
    buffer::{Buffer, CellDiffOption},
    layout::Rect,
    style::{Color, Modifier, Style},
};

pub fn render(frame: &mut Frame, app: &App) -> bool {
    if app.finder.is_some() || !app.is_connected() {
        return false;
    }
    let Some(term) = &app.term else {
        return false;
    };
    let Some(full) = &term.full else {
        return false;
    };
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return true;
    }
    let top = u16::from(!full.expanded);
    let content = Rect::new(
        area.x,
        area.y + top,
        area.width,
        area.height.saturating_sub(top + 1),
    );
    if !full.expanded {
        frame.buffer_mut().set_stringn(
            area.x,
            area.y,
            format!("{} · {}", term.agent, app.lang.tr(Text::FullReadOnly)),
            usize::from(area.width),
            Style::default().add_modifier(Modifier::BOLD),
        );
    }
    if let Some(data) = &full.data {
        let offset = if !full.expanded && full.follows_live() {
            live_offset(&data.frame, content.height)
        } else {
            0
        };
        cells_from(frame.buffer_mut(), content, &data.frame, offset);
        let mut cursor = data.frame.cursor;
        let cursor_in_view = usize::from(cursor.row) >= offset;
        cursor.row = cursor.row.saturating_sub(offset as u16);
        if full.ready
            && term.mode == TermMode::Live
            && cursor.visible
            && cursor_in_view
            && cursor.shape != TerminalCursorShape::Hidden
            && cursor.row < content.height
            && cursor.column < content.width
        {
            frame.set_cursor_position((content.x + cursor.column, content.y + cursor.row));
        }
    }
    let state = if term.mode == TermMode::Stopped {
        Text::StoppedNoInput
    } else if term.mode == TermMode::Ended {
        Text::EndedNoInput
    } else if !full.expanded && full.lost_control {
        Text::FullControlLost
    } else if term.typing {
        Text::FullInput
    } else if full.expanded || !full.ready {
        Text::FullWaiting
    } else {
        Text::FullReadOnly
    };
    let status = match &app.message {
        Some(message) => format!("{} · {} · {message}", term.agent, app.lang.tr(state)),
        None => format!("{} · {}", term.agent, app.lang.tr(state)),
    };
    frame.buffer_mut().set_stringn(
        area.x,
        area.bottom() - 1,
        status,
        usize::from(area.width),
        Style::default().add_modifier(Modifier::REVERSED),
    );
    true
}
/// Read-only follow includes the cursor row and last output, preserving the
/// complete grid's cell coordinates and leaving alternate screens at the top.
pub fn live_offset(frame: &TerminalFrame, rows: u16) -> usize {
    if frame.alternate_screen {
        return 0;
    }
    let last_text = frame
        .cells
        .iter()
        .rposition(|row| row.iter().any(|cell| !cell.text.trim().is_empty()))
        .map_or(0, |row| row + 1);
    let last_cursor = if frame.cursor.visible {
        usize::from(frame.cursor.row) + 1
    } else {
        0
    };
    last_text.max(last_cursor).saturating_sub(usize::from(rows))
}
pub fn cells(buffer: &mut Buffer, area: Rect, frame: &TerminalFrame) {
    cells_from(buffer, area, frame, 0);
}
fn cells_from(buffer: &mut Buffer, area: Rect, frame: &TerminalFrame, offset: usize) {
    for (row, cells) in frame
        .cells
        .iter()
        .skip(offset)
        .take(usize::from(area.height))
        .enumerate()
    {
        for (column, source) in cells.iter().take(usize::from(area.width)).enumerate() {
            let cell = &mut buffer[(area.x + column as u16, area.y + row as u16)];
            cell.reset();
            cell.set_style(cell_style(source));
            if source.width == 0 {
                cell.set_diff_option(CellDiffOption::Skip);
                continue;
            }
            // Never let a clipped wide lead overwrite the status or next row.
            let text = if source.text.is_empty()
                || (source.width == 2 && column + 1 >= usize::from(area.width))
            {
                " "
            } else {
                &source.text
            };
            cell.set_symbol(text);
            cell.set_diff_option(CellDiffOption::ForcedWidth(
                std::num::NonZeroU16::new(if source.width == 2 && text != " " {
                    2
                } else {
                    1
                })
                .unwrap(),
            ));
        }
    }
}
fn cell_style(cell: &agend_core::protocol::terminal::TerminalCell) -> Style {
    let mut modifier = Modifier::empty();
    for (flag, attribute) in [
        (style::BOLD, Modifier::BOLD),
        (style::DIM, Modifier::DIM),
        (style::ITALIC, Modifier::ITALIC),
        (style::INVERSE, Modifier::REVERSED),
        (style::HIDDEN, Modifier::HIDDEN),
        (style::STRIKEOUT, Modifier::CROSSED_OUT),
    ] {
        if cell.style & flag != 0 {
            modifier |= attribute;
        }
    }
    if cell.style
        & (style::UNDERLINE
            | style::DOUBLE_UNDERLINE
            | style::UNDERCURL
            | style::DOTTED_UNDERLINE
            | style::DASHED_UNDERLINE)
        != 0
    {
        modifier |= Modifier::UNDERLINED;
    }
    let mut result = Style::default()
        .fg(color(cell.foreground))
        .bg(color(cell.background))
        .add_modifier(modifier);
    if let Some(underline) = cell.underline_color {
        result = result.underline_color(color(underline));
    }
    result
}
fn color(color: TerminalColor) -> Color {
    match color {
        TerminalColor::Foreground | TerminalColor::Background => Color::Reset,
        TerminalColor::Indexed { index } => Color::Indexed(index),
        TerminalColor::Rgb { r, g, b } => Color::Rgb(r, g, b),
    }
}
pub fn cursor_style(app: &App) -> SetCursorStyle {
    if app.finder.is_some() || !app.is_connected() {
        return SetCursorStyle::DefaultUserShape;
    }
    let Some(data) = app
        .term
        .as_ref()
        .filter(|term| term.mode == TermMode::Live)
        .and_then(|term| term.full.as_ref())
        .filter(|full| full.ready)
        .and_then(|full| full.data.as_ref())
    else {
        return SetCursorStyle::DefaultUserShape;
    };
    match (data.frame.cursor.shape, data.frame.cursor.blinking) {
        (TerminalCursorShape::Beam, true) => SetCursorStyle::BlinkingBar,
        (TerminalCursorShape::Beam, false) => SetCursorStyle::SteadyBar,
        (TerminalCursorShape::Underline, true) => SetCursorStyle::BlinkingUnderScore,
        (TerminalCursorShape::Underline, false) => SetCursorStyle::SteadyUnderScore,
        (TerminalCursorShape::Block, true) => SetCursorStyle::BlinkingBlock,
        _ => SetCursorStyle::SteadyBlock,
    }
}
