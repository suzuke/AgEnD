//! The screen-layer owner-acceptance steps A2–A4 of docs/gates/gate-11-tui.md,
//! key for key, so the page cannot drift from the app. (A5 is covered by
//! `daemon_source::stopping_the_daemon_shows_disconnected_and_a_new_one_reconnects`.)

mod common;

use agend_tui::i18n::Language;
use common::*;
use ratatui::crossterm::event::KeyCode::{Down, Enter, Left, Right, Up};

/// What `F3` does in `examples/tui_fake.rs`.
const F3_FOLLOW_UP: &str = "Seed 42 hides the flake. Also run 20 times nightly?";

#[test]
fn owner_steps_a2_to_a4_in_traditional_chinese() {
    let (mut app, handle) = demo(Language::ZhTw);
    // A2
    let home = render(&mut app);
    assert!(home.lines().nth(2).unwrap().starts_with("━━ 需要你 · 3 ━━"));
    assert!(line_with(&home, "Regression suite fails 3 of 10 runs").starts_with("▌›"));
    assert!(line_with(&home, "Regression suite").ends_with("新  archfix · T-45"));
    for team in ["┏ archfix", "┏ research", "┏ general"] {
        line_with(&home, team);
    }
    assert!(home.contains("沒有進行中的目標"));

    // A3
    press(&mut app, &[Enter, Down, Down, Down, ch('h')]);
    let home = render(&mut app);
    assert!(home.contains("需要你 · 3"));
    assert!(!line_with(&home, "Regression suite").contains('新'));
    press(&mut app, &[Enter, ch('1')]);
    let list = render(&mut app);
    assert!(
        list.contains("已送出對 A-1 的回答：fixed seed 42"),
        "{list}"
    );
    assert!(list.contains("需要你 · 2 項待處理"));
    assert!(!list.contains("Regression suite"));
    handle.follow_up("A-1", "dev-2", F3_FOLLOW_UP, &["yes", "no"]);
    app.tick();
    press(&mut app, &[ch('h')]);
    let home = render(&mut app);
    assert!(home.contains("需要你 · 3"));
    assert!(
        home.lines()
            .nth(3)
            .unwrap()
            .contains("Seed 42 hides the flake"),
        "{home}"
    );
    // The follow-up is a new question: unread again.
    assert!(line_with(&home, "Seed 42 hides").ends_with("新  archfix · T-45"));
    press(&mut app, &[Enter]);
    assert!(render(&mut app).contains("你（tui）：fixed seed 42"));

    // A4
    press(&mut app, &[ch('h')]);
    while !render(&mut app)
        .lines()
        .any(|l| l.starts_with("┃ ›● Fix lock-order"))
    {
        press(&mut app, &[Down]);
    }
    press(&mut app, &[ch('t')]);
    let term = render(&mut app);
    assert!(term.contains("dev-1 的終端 · 唯讀快照") && term.contains("cargo test --workspace"));
    press(&mut app, &[Left]);
    assert!(
        render(&mut app)
            .lines()
            .any(|l| l.starts_with("┃ ›● Fix lock-order"))
    );
    press(&mut app, &[Up, Up]);
    assert!(render(&mut app).lines().any(|l| l.starts_with("┏›archfix")));
    press(&mut app, &[Right]);
    let team = render(&mut app);
    assert_eq!(first_line(&team), "AgEnD › archfix");
    assert!(team.contains("[1 目標]"));
    press(&mut app, &[ch('2'), Down, Down, ch('t')]);
    assert!(render(&mut app).contains("qa-1 沒有終端輸出。"));
    press(&mut app, &[Left]);
    assert!(render(&mut app).lines().any(|l| l.starts_with("┏›archfix")));
    press(&mut app, &[ch('/'), ch('r'), ch('e'), ch('v'), Enter]);
    assert!(render(&mut app).contains("reviewer-1 的終端"));
    press(&mut app, &[Left]);
    assert_eq!(first_line(&render(&mut app)), "AgEnD");
    press(&mut app, &[ch('L')]);
    assert!(render(&mut app).contains("L 中文"));
    press(&mut app, &[ch('L'), ch('q')]);
    assert!(app.quit);
}
