//! Complete App events through real daemon/holder sockets to a raw PTY consumer.
//! Only this lab's children and holders are stopped; no live backend is launched.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/client_process.rs"]
mod clp;
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::model::Backend;
use agend_core::protocol::terminal::{TerminalFrame, TerminalMouseTracking, TerminalSize};
use agend_tui::{App, i18n::Language, source::client::ClientSource};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
const ID: &str = "g11-raw";
static LAB: std::sync::Mutex<()> = std::sync::Mutex::new(());

// Re-exec this test binary as the agent. The outer test invocation is a no-op;
// only the holder-provided instance identity enters the raw consumer loop.
#[test]
fn raw_pty_agent() {
    if std::env::var("AGEND_INSTANCE").as_deref() != Ok(ID) {
        return;
    }
    assert!(
        std::process::Command::new("stty")
            .args(["raw", "-echo"])
            .status()
            .unwrap()
            .success()
    );
    std::thread::spawn(|| {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(b"\x1b[2J\x1b[HRAW-READY").unwrap();
        stdout.flush().unwrap();
        fs::write("ready", b"ready").unwrap();
        for sequence in 0u64.. {
            let path = format!("output-{sequence}");
            while !Path::new(&path).exists() {
                std::thread::sleep(Duration::from_millis(2));
            }
            let bytes = fs::read(&path).unwrap();
            stdout.write_all(&bytes).unwrap();
            stdout.flush().unwrap();
            let size = std::process::Command::new("stty")
                .arg("size")
                .stdin(std::process::Stdio::inherit())
                .output()
                .unwrap();
            assert!(size.status.success());
            fs::write(format!("size-{sequence}"), size.stdout).unwrap();
            fs::write(format!("emitted-{sequence}"), b"emitted").unwrap();
        }
    });
    let mut input = std::io::stdin().lock();
    let mut received = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("received")
        .unwrap();
    let mut bytes = [0u8; 8192];
    loop {
        let count = input.read(&mut bytes).unwrap();
        if count == 0 {
            return;
        }
        received.write_all(&bytes[..count]).unwrap();
        received.flush().unwrap();
    }
}
struct Native {
    _permit: std::sync::MutexGuard<'static, ()>,
    lab: lab::Lab,
    home: PathBuf,
    daemon: Option<lab::Daemon>,
    sequence: u64,
}
impl Native {
    fn new() -> Self {
        let permit = LAB.lock().unwrap_or_else(|error| error.into_inner());
        let lab = lab::Lab::with_prefix(Path::new(env!("CARGO_BIN_EXE_agend")), "g11app");
        let home = lab.home(1);
        let executable = std::env::current_exe()
            .unwrap()
            .display()
            .to_string()
            .replace('\'', "'\\''");
        clp::add(
            &home,
            ID,
            Backend::Claude,
            &format!("exec '{executable}' --exact raw_pty_agent --nocapture"),
        )
        .unwrap();
        let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
        daemon.ready().unwrap();
        let native = Self {
            _permit: permit,
            lab,
            home,
            daemon: Some(daemon),
            sequence: 0,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !native.workspace().join("ready").exists() {
            assert!(Instant::now() < deadline, "raw agent did not start");
            std::thread::sleep(Duration::from_millis(5));
        }
        native
    }
    fn workspace(&self) -> PathBuf {
        self.home.join("workspace").join(ID)
    }
    fn received(&self) -> Vec<u8> {
        fs::read(self.workspace().join("received")).unwrap_or_default()
    }
    fn output(&mut self, bytes: &[u8]) {
        let workspace = self.workspace();
        let temporary = workspace.join("output-new");
        fs::write(&temporary, bytes).unwrap();
        fs::rename(
            temporary,
            workspace.join(format!("output-{}", self.sequence)),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !workspace
            .join(format!("emitted-{}", self.sequence))
            .exists()
        {
            assert!(Instant::now() < deadline, "native output did not flush");
            std::thread::sleep(Duration::from_millis(2));
        }
        self.sequence += 1;
    }
    fn actual_size(&mut self) -> TerminalSize {
        let sequence = self.sequence;
        self.output(b"");
        let size = fs::read_to_string(self.workspace().join(format!("size-{sequence}"))).unwrap();
        let values = size
            .split_whitespace()
            .map(|part| part.parse::<u16>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            values.len(),
            2,
            "stty did not return rows and columns: {size}"
        );
        TerminalSize {
            rows: values[0],
            columns: values[1],
        }
    }
    fn app(
        &self,
        columns: u16,
        rows: u16,
    ) -> (App, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let source = ClientSource::new(&clp::socket_of(&self.home), None);
        let threads = source.threads();
        let mut app = App::new(Box::new(source), Language::En);
        app.resize(columns, rows);
        key(&mut app, KeyCode::Char('/'));
        for character in ID.chars() {
            key(&mut app, KeyCode::Char(character));
        }
        key(&mut app, KeyCode::Enter);
        wait(&mut app, |app| {
            app.term
                .as_ref()
                .is_some_and(|term| term.full.as_ref().is_some_and(|full| full.ready))
        });
        (app, threads)
    }
    fn expect(&self, app: &mut App, expected: &[u8]) {
        wait(app, |_| {
            let received = self.received();
            assert!(
                expected.starts_with(&received),
                "unexpected native input: {received:?}; expected {expected:?}"
            );
            received == expected
        });
    }
    fn restart(&mut self) {
        self.daemon.take().unwrap().interrupt().unwrap();
        let mut daemon = lab::Daemon::start(&self.lab, &self.home, &[]).unwrap();
        daemon.ready().unwrap();
        self.daemon = Some(daemon);
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.interrupt();
        }
    }
}
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn wait(app: &mut App, ready: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.tick();
        if ready(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "native App timed out: message={:?}, frame={:?}, connected={}",
            app.message,
            frame_opt(app).map(|f| (f.size, f.revision, f.viewport_top, f.live_top)),
            app.is_connected()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn frame_opt(app: &App) -> Option<&TerminalFrame> {
    app.term
        .as_ref()?
        .full
        .as_ref()?
        .data
        .as_ref()
        .map(|data| &data.frame)
}
fn frame(app: &App) -> &TerminalFrame {
    frame_opt(app).unwrap()
}
fn text(frame: &TerminalFrame) -> String {
    frame
        .cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn acquire(app: &mut App) {
    key(app, KeyCode::Char('i'));
    wait(app, |app| app.term.as_ref().is_some_and(|term| term.typing));
}
fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) {
    app.event(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }));
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
