//! Wide layout interactions use the same scripted protocol producer as the app.
mod common;
use agend_tui::{
    app::{Screen, Target},
    i18n::Language,
    render_to_string,
};
use common::*;
use ratatui::crossterm::event::{
    Event, KeyCode::*, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

#[test]
fn wide_dashboard_has_navigation_and_cross_team_tasks_at_both_sizes() {
    for lang in [Language::En, Language::ZhTw] {
        for width in [110, 140] {
            let (mut app, _) = demo(lang);
            let text = render_to_string(&mut app, width, 34);
            assert!(
                text.contains("archfix") && text.contains("research") && text.contains("general"),
                "{text}"
            );
            assert!(text.contains("dev-1") && text.contains("T-45"), "{text}");
            assert!(!text.contains('┏') && !text.contains('┗'), "{text}");
            assert!(text.contains("Tab") && text.contains("Esc"));
            for line in text.lines() {
                assert!(unicode_width::UnicodeWidthStr::width(line) <= width as usize);
            }
        }
    }
}

#[test]
fn attention_can_go_next_previous_without_answering_and_keep_selection_on_return() {
    let (mut app, handle) = demo(Language::En);
    render_to_string(&mut app, 140, 34);
    press(&mut app, &[Char('!')]);
    let first = app.view().selected.clone();
    let text = render_to_string(&mut app, 140, 34);
    assert!(
        text.contains("1 / 3") && text.contains("fixed seed 42"),
        "{text}"
    );
    press(&mut app, &[Char(']')]);
    let second = app.view().selected.clone();
    assert_ne!(first, second);
    assert!(render_to_string(&mut app, 140, 34).contains("2 / 3"));
    press(&mut app, &[Char('[')]);
    assert_eq!(app.view().selected, first);
    press(&mut app, &[Char('[')]);
    assert_eq!(app.view().selected, first, "previous clamps at the start");
    assert!(handle.answers().is_empty() && handle.resolved().is_empty());
    press(&mut app, &[Char('t'), Esc]);
    assert_eq!(app.screen(), &Screen::NeedsYou);
    assert_eq!(app.view().selected, first);
    render_to_string(&mut app, 80, 24);
    press(&mut app, &[Char(']'), Char('[')]);
    assert_eq!(
        app.view().selected,
        first,
        "navigation also works in narrow layout"
    );
}

#[test]
fn sidebar_collapses_teams_and_opens_agent_without_losing_body_position() {
    let (mut app, _) = demo(Language::En);
    render_to_string(&mut app, 140, 34);
    let before = app.view().selected.clone();
    press(&mut app, &[Tab, Down, Down, Left]);
    let text = render_to_string(&mut app, 140, 34);
    assert!(
        !text
            .lines()
            .any(|l| l.chars().take(25).collect::<String>().contains("dev-1")),
        "{text}"
    );
    press(&mut app, &[Right, Down, Enter]);
    assert_eq!(
        app.screen(),
        &Screen::Agent {
            agent: "dev-1".into()
        }
    );
    press(&mut app, &[Esc]);
    assert_eq!(app.screen(), &Screen::Home);
    assert_eq!(app.view().selected, before);
}

#[test]
fn orphan_agents_are_listed_separately_without_relabeling_general() {
    let (mut app, _) = demo(Language::En);
    app.fleet.catalog.agents[0].team_id = "missing-team".into();
    let text = render_to_string(&mut app, 140, 40);
    assert!(
        text.contains("Ungrouped") && text.contains("general"),
        "{text}"
    );
}

#[test]
fn mouse_selects_queue_item_without_sending_an_answer() {
    let (mut app, handle) = demo(Language::En);
    render_to_string(&mut app, 140, 34);
    press(&mut app, &[Char('!')]);
    render_to_string(&mut app, 140, 34);
    app.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 30,
        row: 7,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(matches!(app.view().selected, Some(Target::Item(_))));
    assert!(render_to_string(&mut app, 140, 34).contains("2 / 3"));
    assert!(handle.answers().is_empty() && handle.resolved().is_empty());
}

#[test]
fn stale_mouse_regions_after_resize_cannot_answer_a_question() {
    let (mut app, handle) = demo(Language::En);
    render_to_string(&mut app, 140, 34);
    press(&mut app, &[Char('!')]);
    let (buffer, _) = agend_tui::render_buffer(&mut app, 140, 34);
    let row = (2..31)
        .find(|y| {
            (55..140)
                .any(|x| buffer[(x, *y)].symbol() == "[" && buffer[(x + 1, *y)].symbol() == "1")
        })
        .expect("visible option");
    app.resize(110, 34);
    app.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 57,
        row,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(handle.answers().is_empty());
    let (buffer, _) = agend_tui::render_buffer(&mut app, 140, 34);
    let row = (2..31)
        .find(|y| {
            (55..139)
                .any(|x| buffer[(x, *y)].symbol() == "[" && buffer[(x + 1, *y)].symbol() == "1")
        })
        .unwrap();
    app.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 57,
        row,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(
        handle.answers().len(),
        1,
        "a current painted choice sends one answer"
    );
}

#[test]
fn long_context_can_be_scrolled_without_leaving_the_selected_item() {
    let (mut app, handle) = demo(Language::En);
    handle.follow_up(
        "A-1",
        "dev-2",
        &"Long context 寬字元 ".repeat(300),
        &["continue", "stop"],
    );
    app.tick();
    render_to_string(&mut app, 110, 20);
    press(&mut app, &[Char('!')]);
    let before = app.view().selected.clone();
    let top = render_to_string(&mut app, 110, 20);
    press(&mut app, &[PageDown]);
    let scrolled = render_to_string(&mut app, 110, 20);
    assert_ne!(top, scrolled);
    assert_eq!(app.view().selected, before);
    press(&mut app, &[Enter]);
    let choices = render_to_string(&mut app, 110, 20);
    assert!(choices.contains("[1] continue"), "{choices}");
    assert!(handle.answers().is_empty());
}

#[test]
fn resizing_a_long_detail_keeps_the_selected_action_visible() {
    let (mut app, handle) = demo(Language::En);
    handle.follow_up(
        "A-1",
        "dev-2",
        &"long context ".repeat(120),
        &["VERIFY_VISIBLE_OPTION", "stop"],
    );
    app.tick();
    render_to_string(&mut app, 140, 34);
    press(&mut app, &[Char('!'), Enter]);
    assert!(render_to_string(&mut app, 140, 34).contains("VERIFY_VISIBLE_OPTION"));
    let small = render_to_string(&mut app, 110, 20);
    assert!(small.contains("VERIFY_VISIBLE_OPTION"), "{small}");
    assert!(handle.answers().is_empty());
    press(&mut app, &[Enter]);
    assert_eq!(handle.answers().len(), 1);
}
