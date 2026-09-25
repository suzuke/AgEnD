//! Shared helpers for the TUI tests.

#![allow(dead_code)]

use agend_tui::i18n::Language;
use agend_tui::source::scripted::{ScriptHandle, ScriptedSource};
use agend_tui::{App, render_to_string};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn demo(lang: Language) -> (App, ScriptHandle) {
    let (source, handle) = ScriptedSource::demo();
    (App::new(Box::new(source), lang), handle)
}

pub fn press(app: &mut App, keys: &[KeyCode]) {
    for key in keys {
        app.key(KeyEvent::from(*key));
    }
}

pub fn ch(c: char) -> KeyCode {
    KeyCode::Char(c)
}

pub fn ctrl_c(app: &mut App) {
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
}

pub fn render(app: &mut App) -> String {
    render_to_string(app, 100, 30)
}

/// The rendered line containing `needle`, for key-line assertions.
pub fn line_with<'a>(screen: &'a str, needle: &str) -> &'a str {
    screen
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no line with {needle:?} in:\n{screen}"))
}

pub fn first_line(screen: &str) -> &str {
    screen.lines().next().unwrap_or("")
}
