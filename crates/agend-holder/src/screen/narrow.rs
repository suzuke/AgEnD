//! A two-cell glyph cannot be represented in a single-column terminal.
//! Alacritty 0.26 reflow repeatedly moves that glyph to the next row without
//! consuming it; incoming wide input also indexes a missing spacer column.
//! Clip unrepresentable glyphs to styled blanks before reflow, keeping the
//! actual one-column PTY size, parser modes and normal history. Resize rebases
//! row identities through the existing history adapter.
use super::QueryReplies;
use alacritty_terminal::{
    grid::{Dimensions, Grid},
    index::{Column, Line},
    term::{
        Term, TermMode,
        cell::{Cell, Flags},
    },
};

fn clip(grid: &mut Grid<Cell>) {
    for line in -(grid.history_size() as i32)..grid.screen_lines() as i32 {
        for column in 0..grid.columns() {
            let cell = &mut grid[Line(line)][Column(column)];
            if cell.flags.intersects(
                Flags::WIDE_CHAR | Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER,
            ) {
                cell.clear_wide();
                cell.flags
                    .remove(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
            }
        }
    }
}
pub(super) fn prepare(term: &mut Term<QueryReplies>) {
    if term.mode().contains(TermMode::ALT_SCREEN) {
        // The normal grid is inactive but still reflows during resize. Preserve
        // the actual alternate grid across the public swap's clear-on-entry.
        let mut alternate = term.grid().clone();
        clip(&mut alternate);
        term.swap_alt();
        clip(term.grid_mut());
        term.swap_alt();
        *term.grid_mut() = alternate;
    } else {
        clip(term.grid_mut());
    }
}
