//! Key-sequence tests: levels (`←`/`→`), shortcuts (`t`, `/`, `h`, `!`),
//! language, read vs resolved, answering, and the disconnected state.

mod common;

use agend_core::protocol::ask::AskReply;
use agend_tui::app::{Connection, Screen, Tab, Target};
use agend_tui::i18n::Language;
use common::*;
use ratatui::crossterm::event::KeyCode::{
    self, BackTab, Backspace, Down, Enter, Esc, Left, Right, Tab as TabKey, Up,
};

fn screen(app: &agend_tui::App) -> Screen {
    app.screen().clone()
}

fn selected(app: &agend_tui::App) -> Option<Target> {
    app.view().selected.clone()
}

#[test]
fn right_arrow_is_exactly_enter_on_every_screen() {
    let prefixes: &[&[KeyCode]] = &[
        &[],                                        // Home, needs-you row
        &[Down, Down, Down],                        // Home, team header
        &[Down, Down, Down, Down],                  // Home, goal row
        &[Enter],                                   // Needs you, item -> first option
        &[Enter, Down],                             // Needs you, option -> answer
        &[Down, Down, Down, Enter],                 // Team goals
        &[Down, Down, Down, Enter, ch('2')],        // Team agents
        &[Down, Down, Down, Enter, ch('3')],        // Team pipeline
        &[Down, Down, Down, Down, Enter],           // Task detail stage with a needs-you
        &[Down, Down, Down, Enter, ch('2'), Enter], // Agent detail
        &[ch('t')],                                 // Terminal: nothing to open
    ];
    for prefix in prefixes {
        let (mut with_enter, h1) = demo(Language::En);
        let (mut with_right, h2) = demo(Language::En);
        press(&mut with_enter, prefix);
        press(&mut with_right, prefix);
        press(&mut with_enter, &[Enter]);
        press(&mut with_right, &[Right]);
        assert_eq!(with_enter.stack(), with_right.stack(), "after {prefix:?}");
        assert_eq!(h1.answers(), h2.answers(), "after {prefix:?}");
        assert_eq!(render(&mut with_enter), render(&mut with_right));
    }
}

#[test]
fn left_goes_back_one_level_and_restores_the_selection() {
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[Left, Esc]);
    assert_eq!(screen(&app), Screen::Home, "← on Home does nothing");
    assert!(!app.quit);
    press(&mut app, &[Down, Down, Down, Right, Down, Right]);
    assert_eq!(
        screen(&app),
        Screen::Task {
            task: "T-52".into()
        }
    );
    press(&mut app, &[Left]);
    assert_eq!(
        screen(&app),
        Screen::Team {
            team: "archfix".into(),
            tab: Tab::Goals
        }
    );
    assert_eq!(selected(&app), Some(Target::Task("T-52".into())));
    press(&mut app, &[Left]);
    assert_eq!(screen(&app), Screen::Home);
    assert_eq!(selected(&app), Some(Target::Team("archfix".into())));
}

#[test]
fn tabs_switch_with_digits_and_tab_never_with_arrows() {
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[Down, Down, Down, Enter]);
    let tab = |app: &agend_tui::App| match app.screen() {
        Screen::Team { tab, .. } => *tab,
        other => panic!("not on a team page: {other:?}"),
    };
    press(&mut app, &[ch('3')]);
    assert_eq!(tab(&app), Tab::Pipeline);
    press(&mut app, &[TabKey]);
    assert_eq!(tab(&app), Tab::Goals);
    press(&mut app, &[BackTab]);
    assert_eq!(tab(&app), Tab::Pipeline);
    press(&mut app, &[ch('2'), Right]);
    assert_eq!(
        screen(&app),
        Screen::Agent {
            agent: "dev-1".into()
        }
    );
}

#[test]
fn t_opens_the_rows_agent_terminal_and_back_returns_to_the_same_row() {
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[ch('t')]);
    assert!(matches!(screen(&app), Screen::Terminal { agent, .. } if agent == "dev-2"));
    press(&mut app, &[Left]);
    assert_eq!(screen(&app), Screen::Home);
    assert_eq!(selected(&app), Some(Target::Item("A-1".into())));

    // Needs-you item without an asker: the task holder (T-88 is dev-3's).
    press(&mut app, &[Down, ch('t')]);
    assert!(matches!(screen(&app), Screen::Terminal { agent, .. } if agent == "dev-3"));
    press(&mut app, &[Left, Down, Down, Down, ch('t')]); // goal T-45 -> dev-2
    assert!(matches!(screen(&app), Screen::Terminal { agent, .. } if agent == "dev-2"));

    // A row without an agent: a message, nothing else.
    press(&mut app, &[Left, Up, ch('t')]);
    assert_eq!(screen(&app), Screen::Home);
    assert!(render(&mut app).contains("This row has no agent."));

    // An agent with no output (qa-1 is idle): a message, nothing else.
    press(&mut app, &[Enter, ch('2'), Down, Down, ch('t')]);
    assert!(matches!(screen(&app), Screen::Team { .. }));
    assert!(render(&mut app).contains("No terminal output for qa-1."));

    // Task Detail: the stage's agent.
    press(&mut app, &[ch('1'), Enter, ch('t')]);
    assert!(matches!(screen(&app), Screen::Terminal { agent, .. } if agent == "dev-2"));
}

#[test]
fn finder_jumps_to_agents_tasks_and_teams_and_esc_closes_without_moving() {
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[Down, ch('/'), ch('q'), ch('L'), Esc]);
    assert!(app.finder.is_none());
    assert!(!app.quit, "q types into the finder");
    assert_eq!(app.lang, Language::En, "L types into the finder");
    assert_eq!(selected(&app), Some(Target::Item("event-5".into())));

    press(&mut app, &[ch('/'), ch('r'), ch('e'), ch('v'), Enter]);
    assert!(matches!(screen(&app), Screen::Terminal { agent, .. } if agent == "reviewer-1"));
    press(&mut app, &[Left]);
    assert_eq!(
        screen(&app),
        Screen::Home,
        "← returns to where / was pressed"
    );

    press(
        &mut app,
        &[ch('/'), ch('T'), ch('-'), ch('8'), ch('8'), Right],
    );
    assert_eq!(
        screen(&app),
        Screen::Task {
            task: "T-88".into()
        }
    );
    press(
        &mut app,
        &[
            ch('/'),
            ch('g'),
            ch('e'),
            ch('n'),
            ch('x'),
            Backspace,
            Down,
            Enter,
        ],
    );
    assert_eq!(
        screen(&app),
        Screen::Team {
            team: "general".into(),
            tab: Tab::Goals
        }
    );
    assert_eq!(app.stack().len(), 3);
}

#[test]
fn language_toggle_keeps_screen_selection_and_state() {
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[Down, Down, Down, Enter, ch('2'), Down]);
    let before = app.stack().to_vec();
    press(&mut app, &[ch('l')]);
    assert_eq!(app.lang, Language::En, "lowercase l does nothing");
    press(&mut app, &[ch('L')]);
    assert_eq!(app.lang, Language::ZhTw);
    assert_eq!(app.stack(), before.as_slice());
    assert!(render(&mut app).contains("[2 Agent]"));
    press(&mut app, &[ch('L')]);
    assert_eq!(app.lang, Language::En);
}

#[test]
fn reading_an_item_is_not_resolving_it() {
    let (mut app, handle) = demo(Language::En);
    press(&mut app, &[Enter, Down, Down, Down]); // A-1, its two options, the usage-limit item
    press(&mut app, &[ch('h')]);
    let home = render(&mut app);
    assert!(home.contains("Needs you · 3"), "viewing resolves nothing");
    assert!(
        line_with(&home, "Regression suite").ends_with("   archfix · T-45"),
        "read: no new marker"
    );
    assert!(
        line_with(&home, "Which fuzzer").ends_with("new  research · T-90"),
        "not viewed yet"
    );
    assert!(handle.answers().is_empty());
}

#[test]
fn choosing_an_option_answers_and_removes_the_item_until_a_follow_up() {
    let (mut app, handle) = demo(Language::En);
    press(&mut app, &[Enter, Enter]);
    assert_eq!(selected(&app), Some(Target::Choice("A-1".into(), 0)));
    press(&mut app, &[Enter]);
    assert_eq!(
        handle.answers(),
        [(
            "A-1".to_owned(),
            AskReply::Choice {
                option: "fixed seed 42".into()
            }
        )]
    );
    let screen_text = render(&mut app);
    assert!(screen_text.contains("Needs you · 2 open"), "{screen_text}");
    assert!(screen_text.contains("Answer sent to A-1: fixed seed 42"));
    assert!(!screen_text.contains("Regression suite"));
    assert_eq!(
        selected(&app),
        Some(Target::Item("event-5".into())),
        "moves to the next item"
    );

    // The agent follows up: the conversation comes back with its history.
    handle.follow_up("A-1", "dev-2", "Also run 20 times nightly?", &["yes", "no"]);
    app.tick();
    press(&mut app, &[ch('h')]);
    let home = render(&mut app);
    assert!(home.contains("Needs you · 3"));
    assert!(line_with(&home, "Also run 20 times nightly?").ends_with("archfix · T-45"));
    press(&mut app, &[Enter]);
    let thread = render(&mut app);
    assert!(thread.contains("┃    you (tui): fixed seed 42"), "{thread}");
    assert!(thread.contains("┃    [1] yes"));
}

#[test]
fn digits_and_free_text_answer_only_the_expanded_ask() {
    let (mut app, handle) = demo(Language::En);
    press(&mut app, &[ch('2')]);
    assert!(handle.answers().is_empty(), "digits do nothing on Home");
    press(&mut app, &[ch('!'), Down, Down, Down]); // past A-1's options: the non-ask item
    press(&mut app, &[ch('1'), ch('a')]);
    assert!(
        handle.answers().is_empty() && app.input.is_none(),
        "no action on a non-ask item"
    );
    press(&mut app, &[Down, ch('2')]); // A-2, option 2
    assert_eq!(
        handle.answers()[0],
        (
            "A-2".into(),
            AskReply::Choice {
                option: "AFL++".into()
            }
        )
    );

    press(&mut app, &[ch('h'), Enter, ch('a')]);
    for c in "seed 7".chars() {
        press(&mut app, &[ch(c)]);
    }
    assert!(render(&mut app).contains("Answer A-1: seed 7▏"));
    press(&mut app, &[Enter]);
    assert_eq!(
        handle.answers()[1],
        (
            "A-1".into(),
            AskReply::Text {
                text: "seed 7".into()
            }
        )
    );
    let text = render(&mut app);
    assert!(text.contains("Needs you · 1 open"), "{text}");
    press(&mut app, &[ch('h')]);
    press(&mut app, &[Enter]);
    assert_eq!(app.input, None);
}

#[test]
fn empty_needs_you_offers_back_to_home() {
    use agend_tui::source::scripted::{DemoStep, ScriptedSource, demo_catalog, demo_script};
    let (source, handle) = ScriptedSource::new(demo_catalog());
    for step in demo_script() {
        if let DemoStep::Ask(thread, recap) = step {
            handle.open_ask(thread, recap);
        }
    }
    let mut app = agend_tui::App::new(Box::new(source), Language::En);
    press(&mut app, &[Enter, ch('1'), ch('2')]);
    let text = render(&mut app);
    assert!(text.contains("Needs you · 0 open"), "{text}");
    assert!(text.contains("Nothing needs you right now."));
    assert!(line_with(&text, "[ Back to Home ]").starts_with('›'));
    press(&mut app, &[Right]);
    assert_eq!(screen(&app), Screen::Home);
    assert!(render(&mut app).contains("Needs you · 0"));
}

#[test]
fn stage_with_a_needs_you_opens_it() {
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[Down, Down, Down, Down, Enter, Enter]);
    assert_eq!(screen(&app), Screen::NeedsYou);
    assert_eq!(selected(&app), Some(Target::Item("A-1".into())));
    press(&mut app, &[Left, Down, Enter]);
    assert_eq!(
        screen(&app),
        Screen::Task {
            task: "T-45".into()
        },
        "a stage without one opens nothing"
    );
}

#[test]
fn quit_with_q_or_ctrl_c() {
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[ch('q')]);
    assert!(app.quit);
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[ch('/')]);
    ctrl_c(&mut app);
    assert!(app.quit);
}

#[test]
fn disconnect_shows_a_clear_message_and_reconnect_restores_the_view() {
    let (mut app, handle) = demo(Language::En);
    press(&mut app, &[Down, Down, Down, Enter]);
    handle.set_online(false);
    app.tick();
    assert!(matches!(
        app.connection,
        Connection::Disconnected { attempts: 0, .. }
    ));
    let text = render(&mut app);
    assert!(text.contains("━━ Daemon disconnected"), "{text}");
    assert!(text.contains("Lost the connection to the daemon: connection closed"));
    assert!(text.contains("Reconnecting automatically…"));
    assert!(
        !text.contains("Restructure"),
        "no stale data while disconnected"
    );
    app.tick();
    let text = render(&mut app);
    assert!(
        text.contains("Reconnect attempt 1 failed: daemon is not running (connection refused)")
    );
    press(&mut app, &[ch('r')]);
    assert!(render(&mut app).contains("Reconnect attempt 2 failed"));
    press(&mut app, &[Enter, ch('L')]);
    assert!(render(&mut app).contains("daemon 已斷線"), "L still works");
    press(&mut app, &[ch('L')]);

    handle.set_online(true);
    app.tick();
    assert!(app.is_connected());
    let text = render(&mut app);
    assert!(text.contains("Reconnected to the daemon."));
    assert_eq!(first_line(&text), "AgEnD › archfix", "back where it was");
    assert!(text.contains("Restructure state boundary"));
    press(&mut app, &[ch('h')]);
    assert!(
        render(&mut app).contains("Needs you · 3"),
        "events replayed once, not twice"
    );
}

#[test]
fn starting_without_a_daemon_shows_the_disconnected_state() {
    let (source, handle) = agend_tui::source::scripted::ScriptedSource::demo();
    handle.set_online(false);
    let mut app = agend_tui::App::new(Box::new(source), Language::ZhTw);
    let text = render(&mut app);
    assert!(
        text.contains("與 daemon 的連線中斷：daemon is not running"),
        "{text}"
    );
    assert!(text.contains("r 立即重試"));
    handle.set_online(true);
    app.tick();
    assert!(render(&mut app).contains("需要你 · 3"));
}
