//! Tracks normal-screen row identities while forwarding every parser action.
//! There is one ANSI parser. Single-row scrolls are observed using a temporary
//! grid display offset; multi-row actions use alacritty's bounded row count.
//! The offset is restored before any snapshot/classification.

use super::QueryReplies;
use alacritty_terminal::grid::{Dimensions, GridCell, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::cursor_icon::CursorIcon;
use alacritty_terminal::vte::ansi::*;

#[derive(Default)]
pub(super) struct History {
    pub live_top: u64,
    top: usize,
    bottom: usize,
    rebase_on_normal: bool,
}

impl History {
    pub fn new(rows: u16) -> Self {
        Self {
            bottom: usize::from(rows),
            ..Self::default()
        }
    }
    pub fn resize(&mut self, term: &Term<QueryReplies>) {
        self.top = 0;
        self.bottom = term.grid().screen_lines();
        if term.mode().contains(TermMode::ALT_SCREEN) {
            self.rebase_on_normal = true;
        } else {
            self.rebase(term);
        }
    }
    fn rebase(&mut self, term: &Term<QueryReplies>) {
        // Reflow replaces row boundaries. Never reuse old viewport row IDs.
        self.live_top += (term.grid().screen_lines() + term.grid().history_size()) as u64;
        self.rebase_on_normal = false;
    }
}

pub(super) struct Tracked<'a> {
    pub term: &'a mut Term<QueryReplies>,
    pub history: &'a mut History,
}

impl Tracked<'_> {
    fn apply(
        &mut self,
        eligible: bool,
        exact: Option<usize>,
        f: impl FnOnce(&mut Term<QueryReplies>),
    ) {
        let normal = !self.term.mode().contains(TermMode::ALT_SCREEN);
        let before = self.term.grid().history_size();
        if normal && eligible && exact.is_none() && before > 0 {
            self.term.grid_mut().scroll_display(Scroll::Delta(1));
        }
        f(self.term);
        if normal && eligible {
            let count = exact.unwrap_or_else(|| {
                if before == 0 {
                    self.term.grid().history_size()
                } else {
                    self.term.grid().display_offset().saturating_sub(1)
                }
            });
            self.history.live_top += count as u64;
        }
        self.term.grid_mut().scroll_display(Scroll::Bottom);
        self.normal();
    }
    fn normal(&mut self) {
        if self.history.rebase_on_normal && !self.term.mode().contains(TermMode::ALT_SCREEN) {
            self.history.rebase(self.term);
        }
    }
}

// Keep every Handler method explicit: changes to the locked vte API are
// reviewed here, and no default no-op can silently consume an ANSI action.
macro_rules! forward {
    ($(fn $name:ident($($arg:ident: $ty:ty),*);)*) => {$ (
        fn $name(&mut self, $($arg: $ty),*) {
            self.term.$name($($arg),*);
            self.normal();
        }
    )*};
}

impl Handler for Tracked<'_> {
    fn input(&mut self, a0: char) {
        self.apply(self.history.top == 0, None, |term| term.input(a0));
    }

    fn put_tab(&mut self, a0: u16) {
        self.apply(self.history.top == 0, None, |term| term.put_tab(a0));
    }

    fn linefeed(&mut self) {
        self.apply(self.history.top == 0, None, |term| term.linefeed());
    }

    fn newline(&mut self) {
        self.apply(self.history.top == 0, None, |term| term.newline());
    }

    fn scroll_up(&mut self, a0: usize) {
        let count = a0.min(self.history.bottom.saturating_sub(self.history.top));
        self.apply(self.history.top == 0, Some(count), |term| {
            term.scroll_up(a0)
        });
    }

    fn delete_lines(&mut self, a0: usize) {
        let origin = self.term.grid().cursor.point.line.0 as usize;
        let count = a0.min(self.history.bottom.saturating_sub(origin));
        self.apply(origin == 0 && self.history.top == 0, Some(count), |term| {
            term.delete_lines(a0)
        });
    }

    fn clear_screen(&mut self, a0: ClearMode) {
        let count = if matches!(a0, ClearMode::All) {
            let grid = self.term.grid();
            (0..grid.screen_lines())
                .rev()
                .find(|&line| {
                    (0..grid.columns()).any(|col| !grid[Line(line as i32)][Column(col)].is_empty())
                })
                .map(|line| line + 1)
        } else {
            None
        };
        self.apply(count.is_some(), count, |term| term.clear_screen(a0));
    }

    fn reset_state(&mut self) {
        self.term.reset_state();
        self.history.top = 0;
        self.history.bottom = self.term.grid().screen_lines();
        self.history.rebase(self.term);
    }

    fn set_scrolling_region(&mut self, a0: usize, a1: Option<usize>) {
        let bottom = a1.unwrap_or(self.term.grid().screen_lines());
        if a0 < bottom {
            self.history.top = a0.saturating_sub(1);
            self.history.bottom = bottom.min(self.term.grid().screen_lines());
        }
        self.term.set_scrolling_region(a0, a1);
    }
    forward! {
        fn set_title(a0: Option<String>);
        fn set_cursor_style(a0: Option<CursorStyle>);
        fn set_cursor_shape(a0: CursorShape);
        fn goto(a0: i32, a1: usize);
        fn goto_line(a0: i32);
        fn goto_col(a0: usize);
        fn insert_blank(a0: usize);
        fn move_up(a0: usize);
        fn move_down(a0: usize);
        fn identify_terminal(a0: Option<char>);
        fn device_status(a0: usize);
        fn move_forward(a0: usize);
        fn move_backward(a0: usize);
        fn move_down_and_cr(a0: usize);
        fn move_up_and_cr(a0: usize);
        fn backspace();
        fn carriage_return();
        fn bell();
        fn substitute();
        fn set_horizontal_tabstop();
        fn scroll_down(a0: usize);
        fn insert_blank_lines(a0: usize);
        fn erase_chars(a0: usize);
        fn delete_chars(a0: usize);
        fn move_backward_tabs(a0: u16);
        fn move_forward_tabs(a0: u16);
        fn save_cursor_position();
        fn restore_cursor_position();
        fn clear_line(a0: LineClearMode);
        fn clear_tabs(a0: TabulationClearMode);
        fn set_tabs(a0: u16);
        fn reverse_index();
        fn terminal_attribute(a0: Attr);
        fn set_mode(a0: Mode);
        fn unset_mode(a0: Mode);
        fn report_mode(a0: Mode);
        fn set_private_mode(a0: PrivateMode);
        fn unset_private_mode(a0: PrivateMode);
        fn report_private_mode(a0: PrivateMode);
        fn set_keypad_application_mode();
        fn unset_keypad_application_mode();
        fn set_active_charset(a0: CharsetIndex);
        fn configure_charset(a0: CharsetIndex, a1: StandardCharset);
        fn set_color(a0: usize, a1: Rgb);
        fn dynamic_color_sequence(a0: String, a1: usize, a2: &str);
        fn reset_color(a0: usize);
        fn clipboard_store(a0: u8, a1: &[u8]);
        fn clipboard_load(a0: u8, a1: &str);
        fn decaln();
        fn push_title();
        fn pop_title();
        fn text_area_size_pixels();
        fn text_area_size_chars();
        fn set_hyperlink(a0: Option<Hyperlink>);
        fn set_mouse_cursor_icon(a0: CursorIcon);
        fn report_keyboard_mode();
        fn push_keyboard_mode(a0: KeyboardModes);
        fn pop_keyboard_modes(a0: u16);
        fn set_keyboard_mode(a0: KeyboardModes, a1: KeyboardModesApplyBehavior);
        fn set_modify_other_keys(a0: ModifyOtherKeys);
        fn report_modify_other_keys();
        fn set_scp(a0: ScpCharPath, a1: ScpUpdateMode);
    }
}
