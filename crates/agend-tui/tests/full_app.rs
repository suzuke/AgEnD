//! App integration driven by the actual holder parser and dedicated sockets.
#![cfg(unix)]
#[path = "../../agend-testkit/tests/common/terminal_parser.rs"]
mod parser;
use agend_core::protocol::client::*;
use agend_core::protocol::terminal::*;
use agend_core::traits::TerminalProducer;
use agend_tui::{App, i18n::Language, source::client::ClientSource};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::sync::mpsc;
use std::time::{Duration, Instant};
// Each App opens three connections with cloned socket handles. Keep complete
// scenarios concurrent within macOS's default 256-fd limit; no case is skipped.
static LABS: (std::sync::Mutex<usize>, std::sync::Condvar) =
    (std::sync::Mutex::new(0), std::sync::Condvar::new());
struct Permit;
impl Permit {
    fn new() -> Self {
        let mut count = LABS.0.lock().unwrap_or_else(|error| error.into_inner());
        while *count >= 3 {
            count = LABS
                .1
                .wait(count)
                .unwrap_or_else(|error| error.into_inner());
        }
        *count += 1;
        Self
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        *LABS.0.lock().unwrap_or_else(|error| error.into_inner()) -= 1;
        LABS.1.notify_one();
    }
}
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn wait(app: &mut App, ready: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        app.tick();
        if ready(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "full App timed out: mode={:?}, typing={:?}, frame={:?}, message={:?}",
        app.term.as_ref().map(|term| term.mode),
        app.term.as_ref().map(|term| term.typing),
        app.term
            .as_ref()
            .and_then(|term| term.full.as_ref())
            .and_then(|full| full.data.as_ref())
            .map(|data| (
                &data.request_id,
                data.frame.revision,
                data.frame.size,
                data.frame.viewport_top,
                data.frame.live_top,
                data.frame.cells.len()
            )),
        app.message
    );
}
fn open(source: ClientSource, columns: u16, rows: u16) -> App {
    let mut app = App::new(Box::new(source), Language::En);
    app.resize(columns, rows);
    key(&mut app, KeyCode::Char('/'));
    for character in parser::ID.chars() {
        key(&mut app, KeyCode::Char(character));
    }
    key(&mut app, KeyCode::Enter);
    app
}
fn app(fake: &parser::Fake, columns: u16, rows: u16) -> App {
    let mut app = open(
        ClientSource::new(fake.daemon.socket_path(), None),
        columns,
        rows,
    );
    wait(&mut app, |app| {
        app.term
            .as_ref()
            .and_then(|term| term.full.as_ref())
            .is_some_and(|full| full.ready)
    });
    app
}
fn acquire(app: &mut App) {
    key(app, KeyCode::Char('i'));
    wait(app, |app| app.term.as_ref().is_some_and(|term| term.typing));
}
struct Held {
    parser: parser::Parser,
    entered: mpsc::SyncSender<()>,
    release: mpsc::Receiver<()>,
    held: bool,
}
impl TerminalProducer for Held {
    fn frame(
        &mut self,
        data: TerminalFrameRequest,
    ) -> Result<TerminalFrameData, TerminalOperationError> {
        self.parser.frame(data)
    }
    fn legacy_input(&mut self, bytes: String) -> Result<(), TerminalOperationError> {
        self.parser.legacy_input(bytes)
    }
    fn control(
        &mut self,
        data: TerminalControlRequest,
    ) -> Result<TerminalControlData, TerminalOperationError> {
        if matches!(data.operation, TerminalControlOperation::Acquire { .. }) && !self.held {
            self.held = true;
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        self.parser.control(data)
    }
}
#[test]
fn acquire_and_latest_resize_complete_before_any_key_can_reach_the_consumer() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    fake.parser.output();
    let (entered, observed) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    fake.daemon
        .set_terminal_producer(
            parser::ID,
            Held {
                parser: fake.parser.clone(),
                entered,
                release: gate,
                held: false,
            },
        )
        .unwrap();
    let mut app = app(&fake, 31, 9);
    key(&mut app, KeyCode::Char('i'));
    observed.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(app.full_mode());
    assert!(!app.term.as_ref().unwrap().typing);
    key(&mut app, KeyCode::Char('q'));
    assert!(!app.quit);
    assert!(fake.parser.received().is_empty());
    app.event(mouse_event(
        ratatui::crossterm::event::MouseEventKind::ScrollUp,
        0,
        0,
        KeyModifiers::SHIFT,
    ));
    assert!(
        !fake
            .daemon
            .requests()
            .iter()
            .any(|request| matches!(request, ClientRequest::SetTerminalViewport { .. })),
        "viewport replaced the pending grant selection"
    );
    app.resize(40, 12);
    key(&mut app, KeyCode::Char('x'));
    release.send(()).unwrap();
    wait(&mut app, |app| {
        app.term.as_ref().is_some_and(|term| term.typing)
    });
    let full = app.term.as_ref().unwrap().full.as_ref().unwrap();
    assert_eq!(
        full.data.as_ref().unwrap().frame.size,
        TerminalSize {
            rows: 11,
            columns: 40
        }
    );
    key(&mut app, KeyCode::Char('界'));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !fake.parser.received().contains('界') && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fake.parser.received(), "界");
    for exit in [
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('\u{1d}'), KeyModifiers::NONE),
    ] {
        app.key(exit);
        assert!(!app.full_mode());
        assert!(!app.term.as_ref().unwrap().typing);
        wait(&mut app, |app| {
            app.term
                .as_ref()
                .and_then(|term| term.full.as_ref())
                .is_some_and(|full| full.ready)
        });
        assert_eq!(fake.parser.received(), "界", "local exit leaked into PTY");
        acquire(&mut app);
    }
}
#[test]
fn the_renderer_preserves_real_colors_wide_combining_cells_modes_and_cursor() {
    let _permit = Permit::new();
    use ratatui::style::{Color, Modifier};
    let fake = parser::Fake::default();
    fake.parser.feed(
        "\x1b[2J\x1b[H\x1b[1;3;4;38;2;12;34;56;48;5;42m界e\u{301}\x1b[0m\x1b[?25h\x1b[6 q"
            .as_bytes(),
    );
    let mut app = app(&fake, 40, 12);
    let (buffer, _) = agend_tui::render_buffer(&mut app, 40, 12);
    assert_eq!(buffer[(0, 1)].symbol(), "界");
    assert_eq!(buffer[(2, 1)].symbol(), "e\u{301}");
    assert_eq!(buffer[(0, 1)].fg, Color::Rgb(12, 34, 56));
    assert_eq!(buffer[(0, 1)].bg, Color::Indexed(42));
    assert!(
        buffer[(0, 1)]
            .modifier
            .contains(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED)
    );
    assert_eq!(
        agend_tui::terminal::full::cursor_style(&app),
        ratatui::crossterm::cursor::SetCursorStyle::SteadyBar
    );
    let data = app
        .term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .data
        .as_ref()
        .unwrap();
    let mut clipped = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 1, 1));
    agend_tui::terminal::full::cells(
        &mut clipped,
        ratatui::layout::Rect::new(0, 0, 1, 1),
        &data.frame,
    );
    assert_eq!(
        clipped[(0, 0)].symbol(),
        " ",
        "clipped wide cell escaped the content area"
    );
    fake.parser.feed(b"\x1b[?1049h\x1b[H ALT");
    wait(&mut app, |app| {
        app.term
            .as_ref()
            .unwrap()
            .full
            .as_ref()
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .frame
            .alternate_screen
    });
    let data = &app
        .term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .data
        .as_ref()
        .unwrap()
        .frame;
    assert_eq!(
        (data.history_oldest, data.live_top, data.viewport_top),
        (0, 0, 0)
    );
    assert!(agend_tui::render_to_string(&mut app, 40, 12).contains("ALT"));
}
#[test]
fn another_window_revokes_input_and_only_an_explicit_i_restores_it() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    let mut a = app(&fake, 80, 24);
    acquire(&mut a);
    let mut b = app(&fake, 81, 25);
    acquire(&mut b);
    let deadline = Instant::now() + Duration::from_secs(3);
    while a.term.as_ref().unwrap().typing && Instant::now() < deadline {
        a.tick();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!a.full_mode());
    assert!(!a.term.as_ref().unwrap().typing);
    let rendered = agend_tui::render_to_string(&mut a, 200, 24);
    assert_eq!(rendered.matches("another window").count(), 1);
    key(&mut a, KeyCode::Char('x'));
    assert!(fake.parser.received().is_empty());
    acquire(&mut a);
    key(&mut a, KeyCode::Char('y'));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !fake.parser.received().contains('y') && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fake.parser.received(), "y");
}
#[test]
fn an_old_protocol_peer_keeps_the_plaintext_view_read_only_with_upgrade_guidance() {
    let _permit = Permit::new();
    use agend_testkit::fake_daemon::FakeDaemon;
    let fake = FakeDaemon::start().unwrap();
    fake.set_instance(InstanceView {
        program: None,
        instance_id: parser::ID.into(),
        team_id: "general".into(),
        backend: "claude".into(),
        state: AgentState::Idle,
        working_directory: None,
    });
    fake.set_screen(parser::ID, "LEGACY");
    let mut app = open(ClientSource::new(fake.socket_path(), None), 80, 24);
    assert!(app.term.as_ref().unwrap().upgrade_required);
    key(&mut app, KeyCode::Char('i'));
    assert!(!app.term.as_ref().unwrap().typing);
    assert!(agend_tui::render_to_string(&mut app, 80, 24).contains("requires client 1.4"));
    assert!(fake.terminal_inputs().is_empty());
}
#[test]
fn pinned_history_stays_pinned_and_bottom_resumes_following() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    fake.parser.output();
    let mut app = app(&fake, 80, 24);
    app.scroll_terminal(-5);
    wait(&mut app, |app| {
        frame(app).cells.len() == 22
            && !app
                .term
                .as_ref()
                .unwrap()
                .full
                .as_ref()
                .unwrap()
                .follows_live()
    });
    let pinned = app
        .term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .data
        .as_ref()
        .unwrap()
        .frame
        .clone();
    fake.parser.output();
    wait(&mut app, |app| {
        app.term
            .as_ref()
            .unwrap()
            .full
            .as_ref()
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .frame
            .live_top
            > pinned.live_top
    });
    let next = &app
        .term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .data
        .as_ref()
        .unwrap()
        .frame;
    assert_eq!(next.viewport_top, pinned.viewport_top);
    assert_eq!(next.cells, pinned.cells);
    app.scroll_terminal(i64::MAX);
    wait(&mut app, |app| {
        let f = &app
            .term
            .as_ref()
            .unwrap()
            .full
            .as_ref()
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .frame;
        f.viewport_top == f.live_top
    });
}

fn frame(app: &App) -> &TerminalFrame {
    &app.term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .data
        .as_ref()
        .unwrap()
        .frame
}
fn received(fake: &parser::Fake, expected: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let bytes = fake.parser.received_bytes();
        if bytes == expected {
            return;
        }
        assert!(
            expected.starts_with(&bytes),
            "unexpected PTY input: {bytes:?}; expected {expected:?}"
        );
        assert!(
            Instant::now() < deadline,
            "PTY input timed out: {bytes:?}; expected {expected:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn mouse_event(
    kind: ratatui::crossterm::event::MouseEventKind,
    column: u16,
    row: u16,
    modifiers: KeyModifiers,
) -> ratatui::crossterm::event::Event {
    ratatui::crossterm::event::Event::Mouse(ratatui::crossterm::event::MouseEvent {
        kind,
        column,
        row,
        modifiers,
    })
}
#[test]
fn keys_use_real_application_cursor_keypad_and_newline_modes() {
    let _permit = Permit::new();
    use ratatui::crossterm::event::{Event, KeyEventKind, KeyEventState};
    let fake = parser::Fake::default();
    let mut app = app(&fake, 80, 24);
    acquire(&mut app);
    let mut expected = Vec::new();
    for (code, modifiers, bytes) in [
        (KeyCode::Char('q'), KeyModifiers::NONE, "q"),
        (KeyCode::Esc, KeyModifiers::NONE, "\x1b"),
        (KeyCode::Char('c'), KeyModifiers::CONTROL, "\x03"),
        (KeyCode::Char('L'), KeyModifiers::NONE, "L"),
        (KeyCode::Char('界'), KeyModifiers::NONE, "界"),
        (KeyCode::Char('x'), KeyModifiers::ALT, "\x1bx"),
        (KeyCode::Left, KeyModifiers::NONE, "\x1b[D"),
        (
            KeyCode::Up,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            "\x1b[1;6A",
        ),
        (KeyCode::F(5), KeyModifiers::ALT, "\x1b[15;3~"),
        (KeyCode::Enter, KeyModifiers::NONE, "\r"),
    ] {
        app.event(Event::Key(KeyEvent::new(code, modifiers)));
        expected.extend_from_slice(bytes.as_bytes());
        received(&fake, &expected);
        assert!(app.full_mode() && !app.quit);
        assert_eq!(app.lang, Language::En);
    }
    fake.parser.feed(b"\x1b[?1h\x1b=\x1b[20h");
    wait(&mut app, |app| {
        let m = frame(app).modes;
        m.application_cursor && m.application_keypad && m.line_feed_new_line
    });
    for (code, state, bytes) in [
        (KeyCode::Up, KeyEventState::NONE, "\x1bOA"),
        (KeyCode::Home, KeyEventState::NONE, "\x1bOH"),
        (KeyCode::Char('3'), KeyEventState::KEYPAD, "\x1bOs"),
        (KeyCode::Enter, KeyEventState::KEYPAD, "\x1bOM"),
        (KeyCode::Enter, KeyEventState::NONE, "\r\n"),
    ] {
        app.event(Event::Key(KeyEvent {
            code,
            state,
            kind: KeyEventKind::Press,
            modifiers: KeyModifiers::NONE,
        }));
        expected.extend_from_slice(bytes.as_bytes());
        received(&fake, &expected);
    }
    app.event(Event::Key(KeyEvent {
        code: KeyCode::Char('z'),
        state: KeyEventState::NONE,
        kind: KeyEventKind::Release,
        modifiers: KeyModifiers::NONE,
    }));
    key(&mut app, KeyCode::Char('!'));
    expected.push(b'!');
    received(&fake, &expected);
}
#[test]
fn paste_stays_one_operation_and_local_exit_bytes_remain_data() {
    let _permit = Permit::new();
    use ratatui::crossterm::event::Event;
    let fake = parser::Fake::default();
    let mut app = app(&fake, 80, 24);
    app.event(Event::Paste("READONLY".into()));
    assert!(fake.parser.received_bytes().is_empty());
    acquire(&mut app);
    fake.parser.feed(b"\x1b[?2004h");
    wait(&mut app, |app| frame(app).modes.bracketed_paste);
    app.event(Event::Paste("繁中\nsecond\x1d".into()));
    received(&fake, "\x1b[200~繁中\nsecond\x1d\x1b[201~".as_bytes());
    assert!(app.full_mode() && app.term.as_ref().unwrap().typing);
    fake.parser.feed(b"\x1b[?2004l");
    wait(&mut app, |app| !frame(app).modes.bracketed_paste);
    app.event(Event::Paste("plain\x1d".into()));
    received(
        &fake,
        "\x1b[200~繁中\nsecond\x1d\x1b[201~plain\x1d".as_bytes(),
    );
    // At exactly 1 MiB raw text, base64+JSON is too large. It must be rejected
    // as a complete operation before the producer receives a single byte.
    let before = fake.parser.received_bytes();
    app.event(Event::Paste(
        "x".repeat(agend_core::protocol::holder::MAX_REQUEST_LINE),
    ));
    assert!(
        app.message
            .as_deref()
            .is_some_and(|message| message.contains("exceeds"))
    );
    assert!(app.term.as_ref().unwrap().typing);
    // Base64 alone fits, but exact JSON envelope does not. This rejection
    // arrives asynchronously from the writer and must also keep control.
    app.message = None;
    app.event(Event::Paste(
        "x".repeat(3 * agend_core::protocol::holder::MAX_REQUEST_LINE / 4),
    ));
    wait(&mut app, |app| {
        app.message
            .as_deref()
            .is_some_and(|message| message.contains("exceeds"))
    });
    assert!(app.term.as_ref().unwrap().typing);
    app.event(Event::Paste("END".into()));
    let mut expected = before;
    expected.extend_from_slice(b"END");
    received(&fake, &expected);
    assert_eq!(fake.daemon.requests().iter().filter(|request| matches!(request,ClientRequest::TerminalControl { data } if matches!(data.operation,ClientTerminalOperation::Input { .. }))).count(),3);
}
#[test]
fn mouse_tracking_filters_motion_and_excludes_status_and_outside_cells() {
    let _permit = Permit::new();
    use ratatui::crossterm::event::{MouseButton::*, MouseEventKind::*};
    let fake = parser::Fake::default();
    let mut app = app(&fake, 80, 24);
    acquire(&mut app);
    fake.parser.feed(b"\x1b[?1000h\x1b[?1006h");
    wait(&mut app, |app| {
        frame(app).modes.mouse_tracking == TerminalMouseTracking::Click
            && frame(app).modes.sgr_mouse
    });
    let mut expected = Vec::new();
    for (kind, col, row, modifiers, bytes) in [
        (Down(Left), 0, 0, KeyModifiers::NONE, "\x1b[<0;1;1M"),
        (
            Up(Right),
            79,
            22,
            KeyModifiers::ALT | KeyModifiers::CONTROL,
            "\x1b[<26;80;23m",
        ),
        (ScrollDown, 12, 10, KeyModifiers::NONE, "\x1b[<65;13;11M"),
    ] {
        app.event(mouse_event(kind, col, row, modifiers));
        expected.extend_from_slice(bytes.as_bytes());
        received(&fake, &expected);
    }
    for (kind, col, row) in [
        (Moved, 0, 0),
        (Drag(Left), 1, 1),
        (Down(Left), 0, 23),
        (Down(Left), 80, 0),
        (ScrollUp, 0, 24),
    ] {
        app.event(mouse_event(kind, col, row, KeyModifiers::NONE));
    }
    fake.parser.feed(b"\x1b[?1002h");
    wait(&mut app, |app| {
        frame(app).modes.mouse_tracking == TerminalMouseTracking::Drag
    });
    app.event(mouse_event(Drag(Middle), 3, 4, KeyModifiers::CONTROL));
    expected.extend_from_slice(b"\x1b[<49;4;5M");
    received(&fake, &expected);
    app.event(mouse_event(Moved, 1, 1, KeyModifiers::NONE));
    fake.parser.feed(b"\x1b[?1003h");
    wait(&mut app, |app| {
        frame(app).modes.mouse_tracking == TerminalMouseTracking::Motion
    });
    app.event(mouse_event(Moved, 5, 6, KeyModifiers::ALT));
    expected.extend_from_slice(b"\x1b[<43;6;7M");
    received(&fake, &expected);
}
#[test]
fn legacy_and_utf8_mouse_reports_keep_binary_coordinates_without_wrapping() {
    let _permit = Permit::new();
    use ratatui::crossterm::event::{MouseButton::*, MouseEventKind::*};
    let fake = parser::Fake::default();
    let mut app = app(&fake, 500, 24);
    acquire(&mut app);
    fake.parser.feed(b"\x1b[?1000h");
    wait(&mut app, |app| {
        frame(app).modes.mouse_tracking == TerminalMouseTracking::Click
    });
    app.event(mouse_event(Down(Left), 222, 0, KeyModifiers::NONE));
    let mut expected = b"\x1b[M".to_vec();
    expected.extend_from_slice(&[32, 255, 33]);
    received(&fake, &expected);
    app.event(mouse_event(Down(Left), 223, 0, KeyModifiers::NONE));
    app.event(mouse_event(Up(Left), 0, 0, KeyModifiers::NONE));
    expected.extend_from_slice(b"\x1b[M#!!");
    received(&fake, &expected);
    fake.parser.feed(b"\x1b[?1005h");
    wait(&mut app, |app| frame(app).modes.utf8_mouse);
    app.event(mouse_event(Down(Right), 499, 0, KeyModifiers::ALT));
    expected.extend_from_slice("\x1b[M*Ȕ!".as_bytes());
    received(&fake, &expected);
}
#[test]
fn wheel_uses_history_without_tracking_and_shift_keeps_history_local() {
    let _permit = Permit::new();
    use ratatui::crossterm::event::MouseEventKind::*;
    let fake = parser::Fake::default();
    fake.parser.output();
    let mut app = app(&fake, 80, 24);
    // Header and footer wheel events must not change selection.
    let live = frame(&app).live_top;
    app.event(mouse_event(ScrollUp, 0, 0, KeyModifiers::NONE));
    app.event(mouse_event(ScrollUp, 0, 23, KeyModifiers::NONE));
    app.tick();
    assert_eq!(frame(&app).viewport_top, live);
    app.event(mouse_event(ScrollUp, 0, 1, KeyModifiers::NONE));
    wait(&mut app, |app| {
        frame(app).cells.len() == 22
            && !app
                .term
                .as_ref()
                .unwrap()
                .full
                .as_ref()
                .unwrap()
                .follows_live()
    });
    assert!(fake.parser.received_bytes().is_empty());
    acquire(&mut app);
    fake.parser.output();
    wait(&mut app, |app| frame(app).live_top > live);
    let live = frame(&app).live_top;
    app.event(mouse_event(ScrollUp, 0, 0, KeyModifiers::NONE));
    wait(&mut app, |app| frame(app).viewport_top < live);
    app.scroll_terminal(i64::MAX);
    wait(&mut app, |app| {
        frame(app).viewport_top == frame(app).live_top
    });
    fake.parser.feed(b"\x1b[?1000h\x1b[?1006h");
    wait(&mut app, |app| frame(app).modes.sgr_mouse);
    let live = frame(&app).live_top;
    app.event(mouse_event(ScrollUp, 3, 4, KeyModifiers::SHIFT));
    wait(&mut app, |app| frame(app).viewport_top < live);
    assert!(fake.parser.received_bytes().is_empty());
    app.scroll_terminal(i64::MAX);
    wait(&mut app, |app| {
        frame(app).viewport_top == frame(app).live_top
    });
    app.event(mouse_event(ScrollUp, 3, 4, KeyModifiers::NONE));
    received(&fake, b"\x1b[<64;4;5M");
    fake.parser.feed(b"\x1b[?1049h");
    wait(&mut app, |app| frame(app).alternate_screen);
    app.event(mouse_event(ScrollUp, 3, 4, KeyModifiers::SHIFT));
    assert_eq!(frame(&app).viewport_top, 0);
    assert_eq!(fake.parser.received_bytes(), b"\x1b[<64;4;5M");
}
#[test]
fn failure_and_reconnect_never_restore_control_and_focus_is_mode_aware() {
    let _permit = Permit::new();
    use ratatui::crossterm::event::Event;
    let dir = agend_testkit::tempdir::TempDir::new("tui-full-reconnect").unwrap();
    let daemon =
        agend_testkit::fake_daemon::FakeDaemon::start_at(&dir.path().join("daemon.sock")).unwrap();
    let parser = parser::Parser::default();
    daemon.set_instance(InstanceView {
        program: None,
        instance_id: parser::ID.into(),
        team_id: "general".into(),
        backend: "claude".into(),
        state: AgentState::Idle,
        working_directory: None,
    });
    daemon
        .set_terminal_producer(parser::ID, parser.clone())
        .unwrap();
    let fake = parser::Fake { daemon, parser };
    let mut app = app(&fake, 80, 24);
    acquire(&mut app);
    app.event(Event::FocusGained);
    assert!(fake.parser.received_bytes().is_empty());
    fake.parser.feed(b"\x1b[?1004h");
    wait(&mut app, |app| frame(app).modes.focus_reporting);
    app.event(Event::FocusGained);
    app.event(Event::FocusLost);
    received(&fake, b"\x1b[I\x1b[O");
    fake.daemon.set_instance(InstanceView {
        program: None,
        instance_id: parser::ID.into(),
        team_id: "general".into(),
        backend: "claude".into(),
        state: AgentState::Failed,
        working_directory: None,
    });
    wait(&mut app, |app| {
        app.fleet
            .agent(parser::ID)
            .is_some_and(|agent| agent.state == agend_tui::source::AgentState::Failed)
    });
    app.event(Event::Paste("FAILED".into()));
    assert!(!app.full_mode() && !app.term.as_ref().unwrap().typing);
    fake.daemon.set_instance(InstanceView {
        program: None,
        instance_id: parser::ID.into(),
        team_id: "general".into(),
        backend: "claude".into(),
        state: AgentState::Idle,
        working_directory: None,
    });
    wait(&mut app, |app| {
        app.term.as_ref().is_some_and(|term| {
            term.mode == agend_tui::app::TermMode::Live
                && term.full.as_ref().is_some_and(|full| full.ready)
        })
    });
    app.event(Event::Paste("NO-RESTORE".into()));
    assert!(!app.term.as_ref().unwrap().typing);
    acquire(&mut app);
    let socket = fake.daemon.socket_path().to_path_buf();
    let parser = fake.parser.clone();
    drop(fake.daemon);
    wait(&mut app, |app| !app.is_connected());
    app.event(Event::Paste("OFFLINE".into()));
    let daemon = agend_testkit::fake_daemon::FakeDaemon::start_at(&socket).unwrap();
    daemon.set_instance(InstanceView {
        program: None,
        instance_id: parser::ID.into(),
        team_id: "general".into(),
        backend: "claude".into(),
        state: AgentState::Idle,
        working_directory: None,
    });
    daemon
        .set_terminal_producer(parser::ID, parser.clone())
        .unwrap();
    let fake = parser::Fake { daemon, parser };
    wait(&mut app, |app| {
        app.is_connected()
            && app
                .term
                .as_ref()
                .is_some_and(|term| term.full.as_ref().is_some_and(|full| full.ready))
    });
    app.event(Event::Paste("RECONNECTED".into()));
    assert!(!app.full_mode() && !app.term.as_ref().unwrap().typing);
    acquire(&mut app);
    app.event(Event::Paste("FRESH".into()));
    received(&fake, b"\x1b[I\x1b[OFRESH");
}

#[test]
fn expired_pinned_history_clamps_once_and_reports_the_retained_boundary() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    fake.parser.output();
    let mut app = app(&fake, 80, 24);
    app.scroll_terminal(i64::MIN);
    wait(&mut app, |app| {
        frame(app).viewport_top == frame(app).history_oldest
    });
    let pinned = frame(&app).viewport_top;
    for _ in 0..10 {
        fake.parser.output();
    }
    wait(&mut app, |app| {
        frame(app).history_oldest > pinned && frame(app).viewport_clamped
    });
    assert_eq!(frame(&app).viewport_top, frame(&app).history_oldest);
    assert!(
        app.message
            .as_deref()
            .is_some_and(|text| text.contains("History expired"))
    );
    let top = frame(&app).viewport_top;
    app.scroll_terminal(0);
    wait(&mut app, |app| !frame(app).viewport_clamped);
    assert_eq!(
        frame(&app).viewport_top,
        top,
        "already clamped selection was not retained"
    );
}
struct RefuseAcquire(parser::Parser);
impl TerminalProducer for RefuseAcquire {
    fn frame(
        &mut self,
        data: TerminalFrameRequest,
    ) -> Result<TerminalFrameData, TerminalOperationError> {
        self.0.frame(data)
    }
    fn legacy_input(&mut self, bytes: String) -> Result<(), TerminalOperationError> {
        self.0.legacy_input(bytes)
    }
    fn control(
        &mut self,
        data: TerminalControlRequest,
    ) -> Result<TerminalControlData, TerminalOperationError> {
        if matches!(data.operation, TerminalControlOperation::Acquire { .. }) {
            Err(TerminalOperationError {
                request_id: data.request_id,
                code: "not_supported".into(),
                message: "this backend waits for U17 verification".into(),
            })
        } else {
            self.0.control(data)
        }
    }
}
#[test]
fn unsupported_acquire_preserves_the_live_readonly_view_without_retrying_control() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    fake.daemon
        .set_terminal_producer(parser::ID, RefuseAcquire(fake.parser.clone()))
        .unwrap();
    let mut app = app(&fake, 80, 24);
    key(&mut app, KeyCode::Char('i'));
    wait(&mut app, |app| {
        app.message
            .as_deref()
            .is_some_and(|text| text.contains("not_supported"))
    });
    assert!(!app.full_mode() && !app.term.as_ref().unwrap().typing);
    assert_eq!(
        app.term.as_ref().unwrap().mode,
        agend_tui::app::TermMode::Live
    );
    fake.parser.feed(b"STILL-LIVE");
    wait(&mut app, |app| {
        frame(app).cells.iter().any(|row| {
            row.iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
                .contains("STILL-LIVE")
        })
    });
    assert_eq!(fake.daemon.requests().iter().filter(|request| matches!(request,ClientRequest::TerminalControl { data } if matches!(data.operation,ClientTerminalOperation::Acquire { .. }))).count(),1);
    app.paste("NO-INPUT");
    assert!(fake.parser.received_bytes().is_empty());
}

#[test]
fn readonly_view_follows_the_last_live_output_without_resizing_the_pty() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    let mut app = app(&fake, 40, 12);
    let original = frame(&app).size;
    // Output still in the lower part of the 50-row screen, before any scroll.
    // Clipping the first 10 rows would lose every subsequent update here.
    for i in 0..35 {
        fake.parser.feed(format!("fresh-{i:02}\r\n").as_bytes());
    }
    wait(&mut app, |app| {
        frame(app).cells.iter().any(|row| {
            row.iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
                .contains("fresh-34")
        })
    });
    let text = agend_tui::render_to_string(&mut app, 40, 12);
    assert!(
        text.contains("fresh-34"),
        "last live output clipped: {text}"
    );
    assert_eq!(frame(&app).size, original);
    assert!(
        !fake
            .daemon
            .requests()
            .iter()
            .any(|request| matches!(request, ClientRequest::TerminalControl { .. })),
        "read-only view changed another window's PTY"
    );
    app.scroll_terminal(-3);
    wait(&mut app, |app| {
        frame(app).viewport_top > frame(app).live_top && frame(app).cells.len() == 10
    });
    let pinned = frame(&app).viewport_top;
    fake.parser.feed(b"fresh-35\r\n");
    wait(&mut app, |app| frame(app).revision > 36);
    assert_eq!(frame(&app).viewport_top, pinned);
    assert!(!agend_tui::render_to_string(&mut app, 40, 12).contains("fresh-35"));
    app.scroll_terminal(i64::MAX);
    wait(&mut app, |app| {
        frame(app).cells.len() == usize::from(original.rows)
    });
    assert!(agend_tui::render_to_string(&mut app, 40, 12).contains("fresh-35"));
}

#[test]
fn an_exit_with_a_failed_resubscribe_stays_readonly_and_retries_without_old_control() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    let mut app = app(&fake, 80, 24);
    acquire(&mut app);
    fake.daemon.set_supported_versions(&[V1_1]);
    app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL));
    assert!(
        !app.full_mode(),
        "failed exit resubscribe kept the old full mode"
    );
    assert!(!app.term.as_ref().unwrap().typing);
    assert_eq!(
        app.term.as_ref().unwrap().mode,
        agend_tui::app::TermMode::Ended
    );
    app.paste("OLD-CONTROL");
    assert!(fake.parser.received_bytes().is_empty());
    fake.daemon.set_supported_versions(&[V1_4]);
    wait(&mut app, |app| {
        app.term.as_ref().is_some_and(|term| {
            term.mode == agend_tui::app::TermMode::Live
                && term.full.as_ref().is_some_and(|full| full.ready)
        })
    });
    assert!(!app.full_mode() && !app.term.as_ref().unwrap().typing);
    acquire(&mut app);
    app.paste("FRESH");
    received(&fake, b"FRESH");
}

// Emit the same Ratatui diff and native extension as the interactive loop, then
// let a second real holder parser consume the bytes as an outer terminal.
fn native_frame(
    app: &mut App,
    previous: &mut ratatui::buffer::Buffer,
    native: &mut agend_tui::terminal::native_render::CellRenderer,
    consumer: &mut agend_holder::screen::Screen,
    columns: u16,
    rows: u16,
) -> Vec<u8> {
    use ratatui::backend::{Backend, CrosstermBackend};
    // Configure this byte collector like OuterModes configures the native TUI;
    // no environment mutation, and no concurrent color-disabled cases here.
    static COLORS: std::sync::Once = std::sync::Once::new();
    COLORS.call_once(|| ratatui::crossterm::style::force_color_output(true));
    let (buffer, _) = agend_tui::render_buffer(app, columns, rows);
    if previous.area != buffer.area {
        *previous = ratatui::buffer::Buffer::empty(buffer.area);
        consumer.resize(rows, columns);
    }
    let mut bytes = Vec::new();
    {
        let mut backend = CrosstermBackend::new(&mut bytes);
        backend.draw(previous.diff(&buffer).into_iter()).unwrap();
        if let Some(position) = agend_tui::terminal::full::cursor_position(app, buffer.area) {
            backend.set_cursor_position(position).unwrap();
            backend.show_cursor().unwrap();
        } else {
            backend.hide_cursor().unwrap();
        }
        native.prepare(app, &buffer).write(&mut backend).unwrap();
    }
    consumer.process(&bytes);
    *previous = buffer;
    bytes
}
fn update_frame(app: &mut App, parser: &parser::Parser, bytes: &[u8]) {
    let revision = app
        .term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .data
        .as_ref()
        .unwrap()
        .frame
        .revision;
    parser.feed(bytes);
    wait(app, |app| {
        app.term
            .as_ref()
            .unwrap()
            .full
            .as_ref()
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .frame
            .revision
            > revision
    });
}
#[test]
fn native_underline_shapes_and_style_only_transitions_survive_the_real_backend() {
    use agend_holder::screen::{ReplySink, Screen};
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    let mut app = app(&fake, 40, 12);
    acquire(&mut app);
    let mut native = agend_tui::terminal::native_render::CellRenderer::default();
    let mut previous = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 40, 12));
    let mut consumer = Screen::new(12, 40, ReplySink::default());
    // Each iteration overwrites exactly the same glyphs. Ratatui alone cannot
    // distinguish any of these five underlined modifiers.
    for underline in [2, 3, 4, 5, 1, 0, 3] {
        let stimulus = format!(
            "\x1b[H\x1b[0;1;2;3;7;8;9;38;2;12;34;56;48;5;42;58:2::90:80:70;4:{underline}m界e\u{301}\x1b[0m\x1b[2;7H\x1b[6 q"
        );
        update_frame(&mut app, &fake.parser, stimulus.as_bytes());
        native_frame(&mut app, &mut previous, &mut native, &mut consumer, 40, 12);
        let original = &app
            .term
            .as_ref()
            .unwrap()
            .full
            .as_ref()
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .frame;
        let rendered = consumer
            .frame(TerminalViewport {
                top: None,
                rows: 12,
            })
            .unwrap();
        for column in 0..3 {
            let actual = &rendered.cells[0][column];
            let expected = &original.cells[0][column];
            assert_eq!(
                actual.text, expected.text,
                "underline={underline}, column={column}"
            );
            assert_eq!(actual.width, expected.width);
            // A spacer is occupied by its lead; no independent glyph/SGR is
            // emitted for it. Its physical appearance belongs to the lead.
            if expected.width == 0 {
                continue;
            }
            assert_eq!(
                actual.style, expected.style,
                "underline={underline}, column={column}"
            );
            assert_eq!(actual.foreground, expected.foreground);
            assert_eq!(actual.background, expected.background);
            assert_eq!(actual.underline_color, expected.underline_color);
        }
        assert_eq!(rendered.cursor.row, 1);
        assert_eq!(
            rendered.cursor.column, 6,
            "overlay moved the hardware cursor"
        );
        assert_eq!(rendered.cursor.shape, TerminalCursorShape::Beam);
        assert!(!rendered.cursor.blinking);
        assert!(rendered.cursor.visible);
        // The native layer must not continually repaint an unchanged grid.
        let idle = native_frame(&mut app, &mut previous, &mut native, &mut consumer, 40, 12);
        assert!(
            !String::from_utf8_lossy(&idle).contains("界"),
            "idle overlay rewrote cells"
        );
    }
    // Cursor-positioned wide text at the edge is clipped by the same buffer
    // rules used by the adapter; its spacer is never independently printed.
    update_frame(
        &mut app,
        &fake.parser,
        "\x1b[H\x1b[4:5m界\x1b[0m".as_bytes(),
    );
    app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL));
    wait(&mut app, |app| {
        app.term.as_ref().unwrap().full.as_ref().unwrap().ready
    });
    let before_size = app
        .term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .data
        .as_ref()
        .unwrap()
        .frame
        .size;
    native_frame(&mut app, &mut previous, &mut native, &mut consumer, 1, 4);
    let rendered = consumer
        .frame(TerminalViewport { top: None, rows: 4 })
        .unwrap();
    assert_eq!(rendered.cells[1][0].text, " ");
    assert_eq!(
        rendered.cells[1][0].style & style::DASHED_UNDERLINE,
        style::DASHED_UNDERLINE
    );
    assert_eq!(
        app.term
            .as_ref()
            .unwrap()
            .full
            .as_ref()
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .frame
            .size,
        before_size,
        "readonly crop resized the producer PTY"
    );
}
#[test]
fn native_cursor_shapes_visibility_and_readonly_crop_match_the_displayed_cells() {
    use agend_holder::screen::{ReplySink, Screen};
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    let mut app = app(&fake, 30, 8);
    acquire(&mut app);
    let mut native = agend_tui::terminal::native_render::CellRenderer::default();
    let mut previous = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 30, 8));
    let mut consumer = Screen::new(8, 30, ReplySink::default());
    for (shape, expected, blinking) in [
        (1, TerminalCursorShape::Block, true),
        (2, TerminalCursorShape::Block, false),
        (3, TerminalCursorShape::Underline, true),
        (4, TerminalCursorShape::Underline, false),
        (5, TerminalCursorShape::Beam, true),
        (6, TerminalCursorShape::Beam, false),
    ] {
        update_frame(
            &mut app,
            &fake.parser,
            format!("\x1b[H\x1b[4:3mX\x1b[0m\x1b[7;30H\x1b[{shape} q").as_bytes(),
        );
        native_frame(&mut app, &mut previous, &mut native, &mut consumer, 30, 8);
        let frame = consumer
            .frame(TerminalViewport { top: None, rows: 8 })
            .unwrap();
        assert_eq!((frame.cursor.row, frame.cursor.column), (6, 29));
        assert_eq!(
            (frame.cursor.shape, frame.cursor.blinking),
            (expected, blinking)
        );
        assert!(frame.cursor.visible);
    }
    update_frame(&mut app, &fake.parser, b"\x1b[?25l\x1b[H\x1b[4:2mX\x1b[0m");
    native_frame(&mut app, &mut previous, &mut native, &mut consumer, 30, 8);
    assert!(
        !consumer
            .frame(TerminalViewport { top: None, rows: 8 })
            .unwrap()
            .cursor
            .visible
    );
    update_frame(
        &mut app,
        &fake.parser,
        b"\x1b[?25h\x1b[7;9H\x1b[4:4mY\x1b[0m",
    );
    app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL));
    wait(&mut app, |app| {
        app.term.as_ref().unwrap().full.as_ref().unwrap().ready
    });
    native_frame(&mut app, &mut previous, &mut native, &mut consumer, 30, 5);
    let rendered = consumer
        .frame(TerminalViewport { top: None, rows: 5 })
        .unwrap();
    assert_eq!((rendered.cursor.row, rendered.cursor.column), (3, 9));
    assert!(rendered.cursor.visible);
    assert_eq!(rendered.cells[3][8].text, "Y");
    assert_eq!(
        rendered.cells[3][8].style & style::DOTTED_UNDERLINE,
        style::DOTTED_UNDERLINE
    );
    // A finder replaces terminal content and hides its cursor. Returning to the
    // terminal must repaint the native extension even if source cells did not change.
    key(&mut app, KeyCode::Char('/'));
    native_frame(&mut app, &mut previous, &mut native, &mut consumer, 30, 5);
    assert!(
        !consumer
            .frame(TerminalViewport { top: None, rows: 5 })
            .unwrap()
            .cursor
            .visible
    );
    key(&mut app, KeyCode::Esc);
    native_frame(&mut app, &mut previous, &mut native, &mut consumer, 30, 5);
    let frame = consumer
        .frame(TerminalViewport { top: None, rows: 5 })
        .unwrap();
    assert_eq!(
        frame.cells[3][8].style & style::DOTTED_UNDERLINE,
        style::DOTTED_UNDERLINE
    );
    assert!(frame.cursor.visible);
}

#[test]
fn drawing_the_backend_size_repairs_missing_and_stale_resize_events() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    let mut app = app(&fake, 31, 9);
    // No Resize event arrives: the actual draw backend is already smaller.
    agend_tui::render_buffer(&mut app, 20, 5);
    acquire(&mut app);
    let size = |app: &App| {
        app.term
            .as_ref()
            .unwrap()
            .full
            .as_ref()
            .unwrap()
            .data
            .as_ref()
            .unwrap()
            .frame
            .size
    };
    assert_eq!(
        size(&app),
        TerminalSize {
            rows: 4,
            columns: 20
        }
    );
    // A delayed native event must not override the current draw dimensions.
    app.event(ratatui::crossterm::event::Event::Resize(31, 9));
    agend_tui::render_buffer(&mut app, 20, 5);
    key(&mut app, KeyCode::Char('x'));
    assert!(
        fake.parser.received().is_empty(),
        "a key passed before the current physical size was confirmed"
    );
    wait(&mut app, |app| {
        app.term.as_ref().is_some_and(|term| term.typing)
            && size(app)
                == (TerminalSize {
                    rows: 4,
                    columns: 20,
                })
    });
    key(&mut app, KeyCode::Char('界'));
    let deadline = Instant::now() + Duration::from_secs(3);
    while fake.parser.received().is_empty() {
        assert!(
            Instant::now() < deadline,
            "confirmed physical size never enabled input"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fake.parser.received(), "界");
}

#[path = "support/delayed_frames.rs"]
mod delayed_frames;
