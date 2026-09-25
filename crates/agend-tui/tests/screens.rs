//! Headless render tests (ratatui TestBackend): key lines of every screen in
//! both languages, content-only highlight, CJK alignment, small terminals.

mod common;

use agend_tui::i18n::Language;
use agend_tui::render_buffer;
use common::*;
use ratatui::crossterm::event::KeyCode::{self, Down, Enter, Esc};
use ratatui::style::Modifier;
use unicode_width::UnicodeWidthStr;

#[test]
fn home_puts_needs_you_on_top_then_one_block_per_team() {
    let (mut app, _) = demo(Language::En);
    let screen = render(&mut app);
    let lines: Vec<&str> = screen.lines().collect();
    assert_eq!(lines[0], "AgEnD");
    assert!(lines[2].starts_with("━━ Needs you · 3 ━━"), "{screen}");
    assert!(lines[3].starts_with("▌›! Regression suite fails 3 of 10 runs."));
    assert!(lines[3].ends_with("new  archfix · T-45"));
    assert!(lines[4].contains("reviewer-1 hit its usage limit"));
    assert!(lines[5].ends_with("research · T-90"));
    let archfix = line_with(&screen, "┏ archfix");
    assert!(
        archfix.ends_with("● 1 working  ! 1 needs you  ○ 1 idle"),
        "{archfix}"
    );
    assert!(
        line_with(&screen, "Restructure state boundary")
            .contains("[□□□□□] 0/5  waiting for you  T-45")
    );
    assert!(line_with(&screen, "Fix lock-order").contains("[■■□□□] 2/5  running: checks  T-52"));
    assert!(line_with(&screen, "✓ T-37 merged").contains("Tidy config loader"));
    assert!(line_with(&screen, "┏ research").ends_with("! 1 needs you  ⚠ 1 stuck"));
    assert!(line_with(&screen, "┏ general").ends_with("○ 1 idle"));
    assert!(screen.contains("┃    no active goals"));
    // The repo appears only in Task Detail.
    assert!(!screen.contains("agend-terminal") && !screen.contains("nulls-sim"));
    // Needs you comes before every team block.
    assert!(screen.find("Needs you").unwrap() < screen.find("┏ archfix").unwrap());
}

#[test]
fn home_in_traditional_chinese_keeps_ids_and_aligns_wide_text() {
    let (mut app, _) = demo(Language::ZhTw);
    for (width, height) in [(80, 24), (100, 30), (140, 40)] {
        let screen = agend_tui::render_to_string(&mut app, width, height);
        assert!(screen.contains("━━ 需要你 · 3 ━━"), "{screen}");
        assert!(line_with(&screen, "┏ archfix").ends_with("● 1 工作中  ! 1 需要你  ○ 1 閒置"));
        let cjk = line_with(&screen, "完成模擬器");
        let ascii = line_with(&screen, "Survey fuzzers");
        // Right-aligned metadata ends in the same column despite CJK width.
        assert_eq!(cjk.width(), ascii.width(), "{cjk}\n{ascii}");
        assert_eq!(cjk.width(), width as usize);
        assert!(cjk.ends_with("T-88") && ascii.ends_with("T-90"));
        assert!(screen.contains("L English"));
    }
}

#[test]
fn team_page_tabs_show_goals_agents_and_pipeline() {
    let (mut app, _) = demo(Language::En);
    // Home: 3 needs-you rows, then the archfix header.
    press(&mut app, &[Down, Down, Down, Enter]);
    let goals = render(&mut app);
    assert_eq!(first_line(&goals), "AgEnD › archfix");
    assert!(line_with(&goals, "[1 Goals]").contains("2 Agents"));
    assert!(line_with(&goals, "›! Restructure state boundary").ends_with("T-45"));
    assert!(line_with(&goals, "✓ Tidy config loader").contains("done  T-37"));

    press(&mut app, &[ch('2')]);
    let agents = render(&mut app);
    assert!(line_with(&agents, "[2 Agents]").contains("1 Goals"));
    assert!(line_with(&agents, "State").contains("Backend"));
    assert!(
        line_with(&agents, "›● working")
            .contains("dev-1       claude    T-52 Fix lock-order inversion")
    );
    assert!(line_with(&agents, "dev-2").contains("! needs you"));
    assert!(line_with(&agents, "qa-1").contains("opencode  —"));
    assert!(
        !agents.contains("reviewer-1"),
        "other teams' members stay out"
    );

    press(&mut app, &[ch('3')]);
    let pipeline = render(&mut app);
    assert!(pipeline.contains("── work (1)"));
    assert!(pipeline.contains("── command (1)"));
    assert!(pipeline.contains("── done (1)"));
    assert!(line_with(&pipeline, "T-52 Fix lock-order").ends_with("dev-1"));
}

#[test]
fn task_detail_is_the_only_screen_with_the_repo() {
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[Down, Down, Down, Down, Enter]);
    let screen = render(&mut app);
    assert_eq!(first_line(&screen), "AgEnD › T-45");
    assert!(screen.contains("team archfix · repo agend-terminal · T-45 · agent: dev-2"));
    assert!(line_with(&screen, "Status: ! waiting for you").ends_with("[□□□□□] 0/5"));
    assert!(line_with(&screen, "├─›● 1. implement (work)").ends_with("running  dev-2"));
    assert!(line_with(&screen, "└─ · 5. merge (merge)").contains("not started"));
    assert!(line_with(&screen, "Regression suite fails").starts_with("▌ !"));
    assert!(screen.contains("━━ Recent events"));
    assert!(screen.contains("· #3 ask A-1 opened"));
}

#[test]
fn agent_detail_shows_reported_state_task_and_events() {
    let (mut app, _) = demo(Language::ZhTw);
    press(&mut app, &[Down, Down, Down, Enter, ch('2'), Down, Enter]);
    let screen = render(&mut app);
    assert_eq!(first_line(&screen), "AgEnD › archfix › dev-2");
    assert!(screen.contains("team archfix · backend codex"));
    assert!(screen.contains("狀態：! 需要你"));
    assert!(line_with(&screen, "›目前任務：T-45 Restructure state boundary").starts_with('›'));
    assert!(screen.contains("[t] 看終端"));
    assert!(screen.contains("· #3 ask A-1 opened"));
}

#[test]
fn terminal_view_shows_the_snapshot_read_only() {
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[ch('t')]);
    let screen = render(&mut app);
    assert_eq!(first_line(&screen), "AgEnD › dev-2 Terminal");
    assert!(screen.contains("━━ Terminal of dev-2 · read-only snapshot"));
    assert!(screen.contains("│ run 3: FAIL state_boundary::drop_order"));
    assert!(screen.contains("↑↓ scroll · ←/Esc back"));
}

#[test]
fn needs_you_expands_the_selected_item_with_recap_thread_and_options() {
    let (mut app, _) = demo(Language::ZhTw);
    press(&mut app, &[Enter]);
    let screen = render(&mut app);
    assert!(screen.contains("━━ 需要你 · 3 項待處理"));
    assert!(screen.contains("┃    來自 dev-2 · 任務 T-45 · team archfix"));
    // What happens if it is left alone (DEMO-01 §4B), right under who/which task.
    let lines: Vec<&str> = screen.lines().collect();
    let from = lines.iter().position(|l| l.contains("來自 dev-2")).unwrap();
    assert!(
        lines[from + 1].starts_with("┃    不處理的話：T-45 stays blocked at implement"),
        "{screen}"
    );
    assert!(screen.contains("┃    目標：Split the state module"));
    assert!(screen.contains("┃    目前已決定：Keep the public API unchanged."));
    assert!(screen.contains("┃    [1] fixed seed 42"));
    assert!(screen.contains("┃    [a] 用自己的話回答"));
    // The item being viewed lost its "new" marker; the others keep it.
    assert!(line_with(&screen, "Regression suite").ends_with("   archfix · T-45"));
    assert!(line_with(&screen, "usage limit").ends_with("新  research · T-88"));
    // A non-ask item says there is no action yet (protocol gap).
    press(&mut app, &[Down, Down, Down]);
    let screen = render(&mut app);
    assert!(
        screen.contains("client protocol v1 還沒有處理這一項的操作（第 8 施工關）"),
        "{screen}"
    );
    assert!(screen.contains("┃    不處理的話：T-88 review stays stopped until reviewer-1"));
}

#[test]
fn finder_lists_matches_by_kind() {
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[ch('/'), ch('d'), ch('e'), ch('v')]);
    let screen = render(&mut app);
    assert!(screen.contains("━━ Find · agents, tasks, teams"));
    assert!(line_with(&screen, "/ dev▏").ends_with("3 found"));
    assert!(line_with(&screen, "›@ dev-1").ends_with("archfix · T-52"));
    assert!(screen.contains(" @ dev-3"));
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[ch('/'), ch('模'), ch('擬')]);
    let screen = render(&mut app);
    assert!(
        line_with(&screen, "# T-88").ends_with("research"),
        "CJK query matches titles"
    );
}

/// Frame glyphs: block borders, bars, tree connectors and border lines.
const FRAME: [&str; 9] = ["▌", "┃", "┏", "┗", "━", "│", "─", "├", "└"];

#[test]
fn highlight_covers_content_but_never_border_glyphs() {
    // (keys from Home, what the selected row starts with)
    let cases: &[(&[KeyCode], &str)] = &[
        (&[], "▌"),
        // Team headers: `┏›archfix ` is highlighted, the `─` run is not.
        (&[Down, Down, Down], "┏"),
        (&[Down, Down, Down, Down, Down, Down], "┏"),
        (&[Down, Down, Down, Down], "┃ "),
        (&[Down, Down, Down, Down, Enter], "  ├─"),
        (&[Down, Down, Down, Enter], ""),
        (&[Down, Down, Down, Enter, ch('2')], ""),
        (&[Enter, Enter], "┃ "),
    ];
    for lang in [Language::En, Language::ZhTw] {
        for (keys, border) in cases {
            let (mut app, _) = demo(lang);
            press(&mut app, keys);
            assert_highlight(&mut app, border);
        }
    }
}

fn assert_highlight(app: &mut agend_tui::App, border: &str) {
    let (buffer, text) = render_buffer(app, 100, 30);
    let y = text
        .lines()
        .position(|l| {
            l.strip_prefix(border)
                .is_some_and(|rest| rest.starts_with('›'))
        })
        .unwrap_or_else(|| panic!("no selected row starting with {border:?}:\n{text}"))
        as u16;
    let reversed = |x: u16| {
        buffer[(x, y)]
            .style()
            .add_modifier
            .contains(Modifier::REVERSED)
    };
    for x in 0..border.width() as u16 {
        assert!(!reversed(x), "border column {x} highlighted:\n{text}");
    }
    let mut content = 0;
    for x in 0..buffer.area.width {
        let symbol = buffer[(x, y)].symbol();
        if FRAME.contains(&symbol) {
            assert!(
                !reversed(x),
                "frame glyph {symbol:?} at column {x} highlighted:\n{text}"
            );
        } else if symbol == "›" {
            assert!(reversed(x), "marker column {x} not highlighted:\n{text}");
            content += 1;
        } else if reversed(x) {
            content += 1;
        }
    }
    assert!(content > 5, "the selected row's content is highlighted");
}

#[test]
fn small_terminal_shows_a_hint_and_min_size_keeps_everything_reachable() {
    let (mut app, _) = demo(Language::En);
    let tiny = agend_tui::render_to_string(&mut app, 50, 12);
    assert!(tiny.starts_with("Terminal too small: 50×12 (need at least 70×20)."));
    let screen = agend_tui::render_to_string(&mut app, 70, 20);
    assert!(screen.contains("↓ "), "hint for cut-off lines:\n{screen}");
    // Keep pressing ↓: the last line of the page becomes visible.
    press(&mut app, &vec![KeyCode::Down; 30]);
    let end = agend_tui::render_to_string(&mut app, 70, 20);
    assert!(end.contains("┃    no active goals"), "{end}");
    assert!(!end.contains("more lines below"), "{end}");
    press(&mut app, &[Esc]);
}

#[test]
fn help_line_lists_only_the_keys_that_work_on_the_selected_row() {
    let help = |app: &mut agend_tui::App| render(app).lines().last().unwrap_or("").to_owned();
    let (mut app, _) = demo(Language::En);
    // Home, needs-you row with an asker: `t` works.
    assert_eq!(
        help(&mut app),
        "↑↓ move · →/Enter open · t terminal · ! needs you · / find · L 中文 · q quit"
    );
    // Home, team header: no agent, so no `t`.
    press(&mut app, &[Down, Down, Down]);
    assert_eq!(
        help(&mut app),
        "↑↓ move · →/Enter open · ! needs you · / find · L 中文 · q quit"
    );
    press(&mut app, &[ch('t')]);
    assert_eq!(
        render(&mut app).lines().nth(28),
        Some("This row has no agent.")
    );

    // Needs you, an ask with options: choose, option and answer all work.
    let (mut app, _) = demo(Language::En);
    press(&mut app, &[Enter]);
    assert_eq!(
        help(&mut app),
        "↑↓ move · →/Enter choose · 1-9 option · a answer · t terminal · ←/Esc back · L 中文"
    );
    // The non-ask item (reviewer-1's usage limit): none of them do anything.
    press(&mut app, &[Down, Down, Down]);
    assert!(render(&mut app).contains("›! reviewer-1 hit its usage limit"));
    assert_eq!(help(&mut app), "↑↓ move · t terminal · ←/Esc back · L 中文");
    let before = render(&mut app);
    press(&mut app, &[Enter, ch('1'), ch('a')]);
    assert_eq!(
        render(&mut app),
        before,
        "the keys left out really do nothing"
    );

    // Traditional Chinese uses the same rule.
    let (mut app, _) = demo(Language::ZhTw);
    press(&mut app, &[Down, Down, Down]);
    assert_eq!(
        help(&mut app),
        "↑↓ 移動 · →/Enter 開啟 · ! 需要你 · / 搜尋 · L English · q 離開"
    );
}
