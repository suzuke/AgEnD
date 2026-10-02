//! Complete App events through real daemon/holder sockets to a raw PTY consumer.
//! Only this lab's children and holders are stopped; no live backend is launched.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/client_process.rs"]
mod clp;
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
#[path = "common/native_app.rs"]
mod native;
use agend_core::protocol::terminal::{TerminalMouseTracking, TerminalSize};
use native::*;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind,
};
use std::{fs, sync::atomic::Ordering};
#[test]
fn raw_pty_agent() {
    native::raw_agent();
}

#[test]
fn native_app_keys_mouse_paste_resize_and_control_transfer_reach_raw_pty() {
    let mut native = Native::new();
    let (mut a, threads_a) = native.app(40, 12);
    let initial = frame(&a).size;
    assert_eq!(native.actual_size(), initial);
    a.paste("READONLY");
    acquire(&mut a);
    assert_eq!(
        frame(&a).size,
        TerminalSize {
            rows: 11,
            columns: 40
        }
    );
    assert_ne!(frame(&a).size, initial);
    assert_eq!(native.actual_size(), frame(&a).size);
    native.output(b"\x1b[?1h\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?1004h");
    wait(&mut a, |app| {
        let m = frame(app).modes;
        m.application_cursor
            && m.sgr_mouse
            && m.bracketed_paste
            && m.focus_reporting
            && m.mouse_tracking == TerminalMouseTracking::Click
    });
    let mut expected = Vec::new();
    for (code, modifiers, bytes) in [
        (KeyCode::Char('q'), KeyModifiers::NONE, "q"),
        (KeyCode::Esc, KeyModifiers::NONE, "\x1b"),
        (KeyCode::Char('c'), KeyModifiers::CONTROL, "\x03"),
        (KeyCode::Up, KeyModifiers::NONE, "\x1bOA"),
        (KeyCode::Char('界'), KeyModifiers::NONE, "界"),
    ] {
        a.event(Event::Key(KeyEvent::new(code, modifiers)));
        expected.extend_from_slice(bytes.as_bytes());
        native.expect(&mut a, &expected);
        assert!(a.full_mode() && !a.quit);
    }
    a.paste("繁中\nsecond\x1d");
    expected.extend_from_slice("\x1b[200~繁中\nsecond\x1d\x1b[201~".as_bytes());
    mouse(
        &mut a,
        MouseEventKind::Down(MouseButton::Left),
        0,
        0,
        KeyModifiers::NONE,
    );
    mouse(
        &mut a,
        MouseEventKind::Up(MouseButton::Right),
        39,
        10,
        KeyModifiers::ALT | KeyModifiers::CONTROL,
    );
    expected.extend_from_slice(b"\x1b[<0;1;1M\x1b[<26;40;11m");
    // Status row, outside columns, unsupported motion and shifted click are local.
    mouse(
        &mut a,
        MouseEventKind::Down(MouseButton::Left),
        0,
        11,
        KeyModifiers::NONE,
    );
    mouse(
        &mut a,
        MouseEventKind::Down(MouseButton::Left),
        40,
        0,
        KeyModifiers::NONE,
    );
    mouse(&mut a, MouseEventKind::Moved, 1, 1, KeyModifiers::NONE);
    mouse(
        &mut a,
        MouseEventKind::Down(MouseButton::Left),
        1,
        1,
        KeyModifiers::SHIFT,
    );
    a.event(Event::FocusGained);
    a.event(Event::FocusLost);
    expected.extend_from_slice(b"\x1b[I\x1b[O");
    native.expect(&mut a, &expected);
    // Rejected paste must not leave a partial prefix at the real consumer.
    a.paste(&"x".repeat(agend_core::protocol::holder::MAX_REQUEST_LINE));
    assert!(
        a.message
            .as_deref()
            .is_some_and(|message| message.contains("exceeds"))
    );
    a.paste("AFTER-REFUSAL");
    expected.extend_from_slice(b"\x1b[200~AFTER-REFUSAL\x1b[201~");
    native.expect(&mut a, &expected);
    a.event(Event::Resize(31, 9));
    assert!(!a.term.as_ref().unwrap().typing);
    a.paste("BEFORE-SIZE-ACK");
    wait(&mut a, |app| {
        app.term.as_ref().unwrap().typing
            && frame(app).size
                == TerminalSize {
                    rows: 8,
                    columns: 31,
                }
    });
    assert_eq!(native.actual_size(), frame(&a).size);
    a.paste("SIZED");
    expected.extend_from_slice(b"\x1b[200~SIZED\x1b[201~");
    native.expect(&mut a, &expected);
    let (mut b, threads_b) = native.app(60, 16);
    acquire(&mut b);
    wait(&mut a, |app| !app.term.as_ref().unwrap().typing);
    assert!(!a.full_mode());
    a.paste("OLD-WINDOW");
    a.resize(20, 5);
    b.paste("NEW-WINDOW");
    expected.extend_from_slice(b"\x1b[200~NEW-WINDOW\x1b[201~");
    native.expect(&mut b, &expected);
    assert_eq!(
        frame(&b).size,
        TerminalSize {
            rows: 15,
            columns: 60
        }
    );
    assert_eq!(native.actual_size(), frame(&b).size);
    acquire(&mut a);
    wait(&mut b, |app| !app.term.as_ref().unwrap().typing);
    assert_eq!(
        frame(&a).size,
        TerminalSize {
            rows: 4,
            columns: 20
        }
    );
    assert_eq!(native.actual_size(), frame(&a).size);
    a.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL));
    assert!(!a.full_mode() && !a.term.as_ref().unwrap().typing);
    acquire(&mut b);
    b.paste("END");
    expected.extend_from_slice(b"\x1b[200~END\x1b[201~");
    native.expect(&mut b, &expected);
    drop(a);
    drop(b);
    assert_eq!(threads_a.load(Ordering::SeqCst), 0);
    assert_eq!(threads_b.load(Ordering::SeqCst), 0);
}

#[test]
fn native_app_history_alt_restart_and_repeated_close_keep_scopes_retired() {
    let mut native = Native::new();
    let (mut app, threads) = native.app(40, 12);
    acquire(&mut app);
    let output = (0..1100)
        .map(|n| format!("row-{n:04}\r\n"))
        .collect::<String>();
    native.output(output.as_bytes());
    wait(&mut app, |app| {
        frame(app).history_oldest > 0 && text(frame(app)).contains("row-1099")
    });
    mouse(&mut app, MouseEventKind::ScrollUp, 2, 2, KeyModifiers::NONE);
    wait(&mut app, |app| {
        !app.term
            .as_ref()
            .unwrap()
            .full
            .as_ref()
            .unwrap()
            .follows_live()
            && frame(app).viewport_top < frame(app).live_top
    });
    let pinned = frame(&app).clone();
    native.output(b"row-1100\r\n");
    wait(&mut app, |app| frame(app).live_top > pinned.live_top);
    assert_eq!(frame(&app).viewport_top, pinned.viewport_top);
    assert_eq!(frame(&app).cells, pinned.cells);
    native.output(b"\x1b[?1000h\x1b[?1006h");
    wait(&mut app, |app| frame(app).modes.sgr_mouse);
    app.scroll_terminal(i64::MAX);
    wait(&mut app, |app| text(frame(app)).contains("row-1100"));
    mouse(
        &mut app,
        MouseEventKind::ScrollUp,
        2,
        2,
        KeyModifiers::SHIFT,
    );
    wait(&mut app, |app| {
        frame(app).viewport_top < frame(app).live_top
    });
    let old_top = frame(&app).viewport_top;
    let output = (1101..2201)
        .map(|n| format!("row-{n:04}\r\n"))
        .collect::<String>();
    native.output(output.as_bytes());
    wait(&mut app, |app| {
        frame(app).history_oldest > old_top && frame(app).viewport_clamped
    });
    assert_eq!(frame(&app).viewport_top, frame(&app).history_oldest);
    assert!(
        app.message
            .as_deref()
            .is_some_and(|message| message.contains("History expired"))
    );
    app.scroll_terminal(i64::MAX);
    wait(&mut app, |app| text(frame(app)).contains("row-2200"));
    native.output(b"\x1b[?1049h\x1b[HALT-LIVE");
    wait(&mut app, |app| {
        frame(app).alternate_screen && text(frame(app)).contains("ALT-LIVE")
    });
    app.scroll_terminal(i64::MIN);
    wait(&mut app, |app| {
        frame(app).alternate_screen && frame(app).viewport_top == 0
    });
    assert_eq!(frame(&app).history_oldest, 0);
    native.output(b"\x1b[?1049l");
    wait(&mut app, |app| !frame(app).alternate_screen);
    let holders = native.lab.running_holders();
    native.daemon.take().unwrap().interrupt().unwrap();
    wait(&mut app, |app| !app.is_connected());
    app.paste("OFFLINE");
    let mut daemon = lab::Daemon::start(&native.lab, &native.home, &[]).unwrap();
    daemon.ready().unwrap();
    native.daemon = Some(daemon);
    wait(&mut app, |app| {
        app.is_connected()
            && app
                .term
                .as_ref()
                .is_some_and(|term| term.full.as_ref().is_some_and(|full| full.ready))
    });
    assert_eq!(native.lab.running_holders(), holders);
    assert!(!app.full_mode() && !app.term.as_ref().unwrap().typing);
    app.paste("NO-AUTO-CONTROL");
    acquire(&mut app);
    app.paste("FRESH");
    native.expect(&mut app, b"FRESH");
    drop(app);
    assert_eq!(threads.load(Ordering::SeqCst), 0);
    // Every new App sees live output but starts without the previous owner.
    // Check thread count for each complete close, without weakening fd baselines.
    let descriptors = fd_count();
    for _ in 0..20 {
        let (mut app, threads) = native.app(40, 12);
        assert!(!app.term.as_ref().unwrap().typing);
        acquire(&mut app);
        drop(app);
        assert_eq!(threads.load(Ordering::SeqCst), 0);
        assert_eq!(fd_count(), descriptors, "App close leaked file descriptors");
    }
    native.restart();
    assert_eq!(native.lab.running_holders(), holders);
    native.lab.stop_all_holders();
    assert!(native.lab.running_holders().is_empty());
}

fn fd_count() -> usize {
    #[cfg(target_os = "linux")]
    let directory = "/proc/self/fd";
    #[cfg(not(target_os = "linux"))]
    let directory = "/dev/fd";
    fs::read_dir(directory).unwrap().count()
}
