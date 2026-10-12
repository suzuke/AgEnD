mod common;
use agend_tui::{
    i18n::Language,
    render_buffer,
    theme::{Theme, preferences},
};
use common::*;
use ratatui::{
    crossterm::event::{Event, KeyCode::*, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    style::Color,
};

#[test]
fn live_preview_can_be_cancelled_and_preserves_selection() {
    let (mut app, handle) = demo(Language::ZhTw);
    press(&mut app, &[Char('!')]);
    let selected = app.view().selected.clone();
    press(&mut app, &[Char('P'), Char('3')]);
    let (light, text) = render_buffer(&mut app, 140, 34);
    assert_eq!(app.theme, Theme::Light);
    assert!(text.contains("即時預覽"));
    assert_eq!(light[(0, 0)].bg, Color::Rgb(233, 237, 245));
    app.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 57,
        row: 15,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(handle.answers().is_empty());
    press(&mut app, &[Esc]);
    assert_eq!(app.theme, Theme::Dark);
    assert_eq!(app.view().selected, selected);
    press(&mut app, &[Char('P'), Char('4'), Enter]);
    assert_eq!(app.theme, Theme::HighContrast);
    assert!(!render_buffer(&mut app, 140, 34).1.contains("即時預覽"));
}
#[test]
fn terminal_defaults_have_no_forced_rgb_background_and_selection_remains_visible() {
    let (mut app, _) = demo(Language::En);
    app.theme = Theme::Terminal;
    let (buf, _) = render_buffer(&mut app, 140, 34);
    assert!(buf.content.iter().all(|c| c.bg == Color::Reset));
    assert!(
        buf.content
            .iter()
            .any(|c| c.modifier.contains(ratatui::style::Modifier::REVERSED))
    );
    for width in [70, 80, 110, 140] {
        for theme in Theme::ALL {
            app.theme = theme;
            render_buffer(&mut app, width, 20);
        }
    }
}
#[test]
fn preferences_roundtrip_and_failed_replacement_leave_no_temporary_file() {
    let dir = agend_testkit::tempdir::TempDir::new("theme-preferences").unwrap();
    let path = dir.path().join("tui-theme");
    assert_eq!(preferences::load(&path).unwrap(), Theme::Dark);
    assert!(!path.exists());
    for theme in Theme::ALL {
        preferences::save(&path, theme).unwrap();
        assert_eq!(preferences::load(&path).unwrap(), theme);
    }
    std::fs::write(&path, "not-a-theme").unwrap();
    assert!(preferences::load(&path).is_err());
    std::fs::write(&path, "x".repeat(65)).unwrap();
    assert!(preferences::load(&path).is_err());
    let destination = dir.path().join("directory");
    std::fs::create_dir(&destination).unwrap();
    assert!(preferences::save(&destination, Theme::Light).is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[test]
fn native_typing_keeps_the_theme_shortcut_as_agent_input() {
    let (mut app, handle) = demo(Language::En);
    press(&mut app, &[Char('t'), Char('i'), Char('P')]);
    assert_eq!(handle.typed(), b"P");
    assert_eq!(app.theme, Theme::Dark);
}
