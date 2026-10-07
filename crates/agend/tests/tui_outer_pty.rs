//! Actual interactive event capture/render/restore inside a native outer PTY.
//! All daemons, App children, holders and PTYs belong to this test's own lab.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/client_process.rs"]
mod clp;
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
#[path = "common/native_app.rs"]
mod native;
use agend_core::protocol::terminal::{
    TerminalColor, TerminalCursorShape, TerminalFrame, TerminalMouseTracking, TerminalSize,
    TerminalViewport, style,
};
use agend_holder::screen::{ReplySink, Screen};
use native::{ID, Native, text};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, SlavePty};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[test]
fn raw_pty_agent() {
    native::raw_agent();
}
#[test]
fn panic_app_probe() {
    if std::env::var("AGEND_NATIVE_PANIC_PROBE").as_deref() != Ok("1") {
        return;
    }
    let home = std::path::PathBuf::from(std::env::var("AGEND_HOME").unwrap());
    let source = agend_tui::source::client::ClientSource::new(&clp::socket_of(&home), None);
    agend_tui::run_with(Box::new(source), agend_tui::i18n::Language::En, |_| {
        panic!("intentional native App unwind")
    })
    .unwrap();
}
struct Capture {
    screen: Screen,
    rows: u16,
    bytes: Vec<u8>,
    error: Option<String>,
    visible_marker: Option<(String, Option<Instant>)>,
}
struct Outer {
    master: Box<dyn MasterPty + Send>,
    slave: Option<Box<dyn SlavePty + Send>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    reader: Option<std::thread::JoinHandle<()>>,
    capture: Arc<Mutex<Capture>>,
    original_termios: Vec<u64>,
}
impl Outer {
    fn new(native: &Native, columns: u16, rows: u16, panic_probe: bool) -> Self {
        let pair = portable_pty::native_pty_system()
            .openpty(size(columns, rows))
            .unwrap();
        let original_termios = termios(&*pair.master);
        let capture = Arc::new(Mutex::new(Capture {
            screen: Screen::new(rows, columns, ReplySink::default()),
            rows,
            bytes: Vec::new(),
            error: None,
            visible_marker: None,
        }));
        let mut input = pair.master.try_clone_reader().unwrap();
        let output = Arc::clone(&capture);
        let reader = std::thread::spawn(move || {
            let mut bytes = [0u8; 8192];
            loop {
                match input.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => {
                        let mut state = output.lock().unwrap();
                        if state.bytes.len() + count > 8 * 1024 * 1024 {
                            state.error = Some("outer output exceeded test capture bound".into());
                            break;
                        }
                        state.bytes.extend_from_slice(&bytes[..count]);
                        state.screen.process(&bytes[..count]);
                        if let Some((marker, None)) = &state.visible_marker {
                            let frame = state
                                .screen
                                .frame(TerminalViewport {
                                    top: None,
                                    rows: state.rows,
                                })
                                .unwrap();
                            if row(&frame, 0).contains(marker) {
                                state.visible_marker.as_mut().unwrap().1 = Some(Instant::now());
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    // Unix PTY reads may report EIO after the last slave closes.
                    Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                    Err(error) => {
                        output.lock().unwrap().error = Some(error.to_string());
                        break;
                    }
                }
            }
        });
        let writer = pair.master.take_writer().unwrap();
        let mut command = if panic_probe {
            let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
            command.args(["--exact", "panic_app_probe", "--nocapture"]);
            command.env("AGEND_NATIVE_PANIC_PROBE", "1");
            command
        } else {
            let mut command = CommandBuilder::new(&native.lab.agend);
            command.args(["app", "--lang", "en"]);
            command
        };
        command.env("AGEND_HOME", &native.home);
        command.env_remove("AGEND_INSTANCE");
        command.env("TERM", "xterm-256color");
        command.env("NO_COLOR", "1");
        command.cwd(&native.home);
        let child = pair.slave.spawn_command(command).unwrap();
        Self {
            master: pair.master,
            slave: Some(pair.slave),
            writer,
            child,
            reader: Some(reader),
            capture,
            original_termios,
        }
    }
    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }
    fn frame(&self) -> TerminalFrame {
        let state = self.capture.lock().unwrap();
        assert!(
            state.error.is_none(),
            "outer reader failed: {:?}",
            state.error
        );
        state
            .screen
            .frame(TerminalViewport {
                top: None,
                rows: state.rows,
            })
            .unwrap()
    }
    fn wait(&mut self, ready: impl Fn(&TerminalFrame) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let frame = self.frame();
            if ready(&frame) {
                return;
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "App exited early: {}",
                text(&frame)
            );
            assert!(
                Instant::now() < deadline,
                "outer App timed out:\n{}",
                text(&frame)
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn open(&mut self) {
        self.wait(|frame| text(frame).contains("AgEnD"));
        assert!(self.frame().alternate_screen);
        let modes = self.frame().modes;
        assert!(modes.bracketed_paste && modes.focus_reporting && modes.sgr_mouse);
        assert_ne!(modes.mouse_tracking, TerminalMouseTracking::None);
        assert_ne!(
            termios(&*self.master),
            self.original_termios,
            "App did not enter raw mode"
        );
        self.send(format!("/{ID}\r").as_bytes());
        self.wait(|frame| {
            row(frame, 0).contains("read-only")
                && row(frame, frame.size.rows - 1).contains("read-only")
        });
    }
    fn acquire(&mut self) {
        self.send(b"i");
        self.wait(|frame| row(frame, frame.size.rows - 1).contains(" · input"));
    }
    fn resize(&mut self, columns: u16, rows: u16) {
        {
            let mut state = self.capture.lock().unwrap();
            state.rows = rows;
            state.screen.resize(rows, columns);
        }
        self.master.resize(size(columns, rows)).unwrap();
    }
    fn received(&mut self, native: &Native, expected: &[u8]) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let actual = native.received();
            assert!(
                expected.starts_with(&actual),
                "unexpected raw input: {actual:?}; expected {expected:?}"
            );
            if actual == expected {
                return;
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "App exited while sending input"
            );
            assert!(
                Instant::now() < deadline,
                "native input timed out: {actual:?}; expected {expected:?}; outer:\n{}",
                text(&self.frame())
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn quit(&mut self) {
        self.send(b"\x1d");
        self.wait(|frame| row(frame, 0).contains("read-only"));
        self.send(b"\x1b");
        self.wait(|frame| !row(frame, 0).contains("read-only"));
        self.send(b"q");
        assert!(self.finish().success());
        self.restored();
    }
    fn finish(&mut self) -> portable_pty::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "App did not exit");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            termios(&*self.master),
            self.original_termios,
            "App did not restore original termios"
        );
        self.slave.take();
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
        status
    }
    fn restored(&self) {
        let frame = self.frame();
        assert!(!frame.alternate_screen);
        assert!(
            !frame.modes.bracketed_paste && !frame.modes.focus_reporting && !frame.modes.sgr_mouse
        );
        assert_eq!(frame.modes.mouse_tracking, TerminalMouseTracking::None);
        assert!(frame.cursor.visible);
        let bytes = self.capture.lock().unwrap();
        let output = String::from_utf8_lossy(&bytes.bytes);
        for reset in [
            "\x1b[?1000l",
            "\x1b[?1002l",
            "\x1b[?1003l",
            "\x1b[?1006l",
            "\x1b[?2004l",
            "\x1b[?1004l",
            "\x1b[0 q",
            "\x1b[?25h",
            "\x1b[0m",
            "\x1b[39m",
            "\x1b[49m",
            "\x1b[59m",
        ] {
            assert!(output.contains(reset), "missing native reset {reset:?}");
        }
    }
}
impl Drop for Outer {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.slave.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    }
}
// tcflag_t is u64 on macOS and u32 on Linux; preserve both without casts.
fn widen<T: Into<u64>>(bits: T) -> u64 {
    bits.into()
}
fn termios(master: &dyn MasterPty) -> Vec<u64> {
    let termios = master.get_termios().expect("native PTY termios");
    let mut settings = vec![
        widen(termios.input_flags.bits()),
        widen(termios.output_flags.bits()),
        widen(termios.control_flags.bits()),
        widen(termios.local_flags.bits()),
    ];
    settings.extend(termios.control_chars.iter().map(|byte| u64::from(*byte)));
    settings
}
fn row(frame: &TerminalFrame, row: u16) -> String {
    frame.cells[usize::from(row)]
        .iter()
        .map(|cell| cell.text.as_str())
        .collect()
}
fn wait_size(native: &mut Native, expected: TerminalSize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let actual = native.actual_size();
        if actual == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "actual agent PTY size {actual:?}, expected {expected:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn actual_app_captures_events_resizes_transfers_control_and_restores_outer_pty() {
    let mut native = Native::new();
    let mut a = Outer::new(&native, 80, 24, false);
    a.open();
    a.acquire();
    a.resize(40, 12);
    wait_size(
        &mut native,
        TerminalSize {
            rows: 11,
            columns: 40,
        },
    );
    native.output("\x1b[H\x1b[1;38;2;12;34;56;48;5;42;58:2::90:80:70;4:3m界e\u{301}\x1b[0m\x1b[2;3H\x1b[6 q\x1b[?1h\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?1004h".as_bytes());
    a.wait(|frame| {
        frame.cells[0][0].text == "界"
            && frame.cells[0][2].text == "e\u{301}"
            && frame.cursor.shape == TerminalCursorShape::Beam
    });
    let frame = a.frame();
    assert_eq!(
        frame.cells[0][0].foreground,
        TerminalColor::Rgb {
            r: 12,
            g: 34,
            b: 56
        }
    );
    assert_eq!(
        frame.cells[0][0].background,
        TerminalColor::Indexed { index: 42 }
    );
    assert_eq!(
        frame.cells[0][0].underline_color,
        Some(TerminalColor::Rgb {
            r: 90,
            g: 80,
            b: 70
        })
    );
    assert_eq!(
        frame.cells[0][0].style & (style::BOLD | style::UNDERCURL),
        style::BOLD | style::UNDERCURL
    );
    assert_eq!((frame.cursor.row, frame.cursor.column), (1, 2));
    let mut expected = Vec::new();
    for (input, output) in [
        (b"q".as_slice(), b"q".as_slice()),
        (b"\x1b", b"\x1b"),
        (b"\x03", b"\x03"),
        (b"\x1b[A", b"\x1bOA"),
        ("界".as_bytes(), "界".as_bytes()),
    ] {
        a.send(input);
        expected.extend_from_slice(output);
        a.received(&native, &expected);
    }
    let paste = "\x1b[200~繁中\nsecond\x1d\x1b[201~".as_bytes();
    a.send(paste);
    expected.extend_from_slice(paste);
    a.send(b"\x1b[<0;1;1M\x1b[<26;40;11m");
    expected.extend_from_slice(b"\x1b[<0;1;1M\x1b[<26;40;11m");
    a.send(b"\x1b[<0;1;12M\x1b[<0;41;1M\x1b[<4;2;2M");
    a.send(b"\x1b[I\x1b[O");
    expected.extend_from_slice(b"\x1b[I\x1b[O");
    a.received(&native, &expected);
    a.resize(31, 9);
    wait_size(
        &mut native,
        TerminalSize {
            rows: 8,
            columns: 31,
        },
    );
    native.output(b"\x1b[HRESIZE-A\x1b[K");
    a.wait(|frame| row(frame, 0).contains("RESIZE-A") && row(frame, 8).contains(" · input"));
    let mut b = Outer::new(&native, 80, 24, false);
    b.open();
    b.acquire();
    b.resize(60, 16);
    wait_size(
        &mut native,
        TerminalSize {
            rows: 15,
            columns: 60,
        },
    );
    native.output(b"\x1b[HRESIZE-B\x1b[K");
    b.wait(|frame| row(frame, 0).contains("RESIZE-B") && row(frame, 15).contains(" · input"));
    a.wait(|frame| row(frame, 0).contains("read-only"));
    a.send(b"\x1b[200~OLD-WINDOW\x1b[201~");
    a.resize(20, 5);
    b.send(b"\x1b[200~NEW-WINDOW\x1b[201~");
    expected.extend_from_slice(b"\x1b[200~NEW-WINDOW\x1b[201~");
    b.received(&native, &expected);
    assert_eq!(
        native.actual_size(),
        TerminalSize {
            rows: 15,
            columns: 60
        }
    );
    a.acquire();
    wait_size(
        &mut native,
        TerminalSize {
            rows: 4,
            columns: 20,
        },
    );
    b.wait(|frame| row(frame, 0).contains("read-only"));
    a.quit();
    b.quit();
    assert_eq!(
        native.received(),
        expected,
        "local exits reached the raw agent"
    );
}
#[test]
fn actual_app_unwind_restores_raw_and_outer_capture_modes() {
    let native = Native::new();
    let mut app = Outer::new(&native, 80, 24, true);
    app.wait(|frame| text(frame).contains("AgEnD"));
    assert!(app.frame().alternate_screen && app.frame().modes.bracketed_paste);
    app.send(b"x");
    let status = app.finish();
    assert_eq!(status.exit_code(), 101);
    app.restored();
    assert!(
        String::from_utf8_lossy(&app.capture.lock().unwrap().bytes)
            .contains("intentional native App unwind")
    );
    assert!(native.received().is_empty());
}

#[test]
fn actual_app_wheels_select_real_history_and_twenty_process_closes_leave_no_fds() {
    wheels_history_and_cleanup(Native::new());
}

#[test]
fn actual_app_with_background_startup_sampling_keeps_final_modes_and_history() {
    wheels_history_and_cleanup(Native::with_startup_sampling(true));
}

fn wheels_history_and_cleanup(mut native: Native) {
    let mut app = Outer::new(&native, 80, 24, false);
    app.open();
    app.acquire();
    let output = (0..1100)
        .map(|n| format!("row-{n:04}\r\n"))
        .collect::<String>();
    native.output(output.as_bytes());
    app.wait(|frame| text(frame).contains("row-1099"));
    // With no agent tracking, an actual outer SGR wheel selects history.
    app.send(b"\x1b[<64;3;3M");
    app.wait(|frame| !text(frame).contains("row-1099") && text(frame).contains("row-1097"));
    let pinned = app.frame().cells[..23].to_vec();
    native.output(b"row-1100\r\n\x1b[?1000h\x1b[?1006h\x1b[?1004h");
    // A focus receipt proves App consumed the frame containing new output
    // and the newly enabled tracking/focus modes; pinned cells must stay put.
    let deadline = Instant::now() + Duration::from_secs(10);
    while native.received().is_empty() {
        app.send(b"\x1b[I");
        assert!(
            Instant::now() < deadline,
            "new modes did not reach the interactive App"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // Drain all earlier focus jobs through a real input sentinel before
    // comparing the absence of forwarded wheel bytes.
    app.send(b"!");
    let deadline = Instant::now() + Duration::from_secs(10);
    let focus = loop {
        let bytes = native.received();
        if bytes.ends_with(b"!") {
            break bytes;
        }
        assert!(
            Instant::now() < deadline,
            "focus input barrier did not reach the raw consumer"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let reports = &focus[..focus.len() - 1];
    assert_eq!(reports.len() % 3, 0);
    assert!(reports.chunks(3).all(|bytes| bytes == b"\x1b[I"));
    assert_eq!(app.frame().cells[..23], pinned);
    let top = app.frame().cells[0].clone();
    // Tracking is now active, but Shift wheel still selects local history.
    app.send(b"\x1b[<68;3;3M");
    app.wait(|frame| frame.cells[0] != top);
    app.send(b"\x1b[<69;3;3M\x1b[<69;3;3M\x1b[<69;3;3M");
    app.wait(|frame| text(frame).contains("row-1100"));
    assert_eq!(
        native.received(),
        focus,
        "history wheels were forwarded to the agent"
    );
    app.quit();
    drop(app);
    let holders = native.lab.running_holders();
    let descriptors = fd_count();
    for _ in 0..20 {
        let mut app = Outer::new(&native, 80, 24, false);
        app.open();
        app.acquire();
        app.quit();
        drop(app);
        assert_eq!(
            fd_count(),
            descriptors,
            "outer App fixture leaked descriptors"
        );
    }
    assert_eq!(native.lab.running_holders(), holders);
    assert_eq!(native.received(), focus);
}
fn fd_count() -> usize {
    #[cfg(target_os = "linux")]
    let directory = "/proc/self/fd";
    #[cfg(not(target_os = "linux"))]
    let directory = "/dev/fd";
    std::fs::read_dir(directory).unwrap().count()
}

#[test]
fn actual_app_renders_each_final_dirty_burst_within_the_local_budget() {
    final_dirty_budget(Native::new(), 80, 24);
}

#[test]
fn actual_app_with_background_startup_sampling_keeps_final_dirty_output_within_budget() {
    // Exercise both a shorter operator viewport and the recorded startup
    // dimensions, where P5 still reads the shared complete 24-row grid.
    final_dirty_budget(Native::with_startup_sampling(true), 80, 24);
    final_dirty_budget(Native::with_startup_sampling(true), 100, 25);
}

fn final_dirty_budget(mut native: Native, columns: u16, rows: u16) {
    let mut outer = Outer::new(&native, columns, rows, false);
    outer.open();
    outer.acquire();
    // Measure more conservatively than the contract: start before notifying
    // the real producer, include file handoff/flush/render/outer parsing, and
    // require 300 ms without adding any holder round-trip allowance.
    for sequence in 0..12 {
        let marker = format!("FINAL-DIRTY-{sequence:02}");
        let mut burst = Vec::new();
        for intermediate in 0..100 {
            burst.extend_from_slice(format!("\x1b[Hintermediate-{intermediate:03}").as_bytes());
        }
        burst.extend_from_slice(format!("\x1b[2J\x1b[H{marker}").as_bytes());
        outer.capture.lock().unwrap().visible_marker = Some((marker.clone(), None));
        let started = Instant::now();
        native.output(&burst);
        let producer_ack = started.elapsed();
        outer.wait(|frame| row(frame, 0).contains(&marker));
        // Timestamp the actual outer parser observation, independently of the
        // producer's subsequent stty/size-file acknowledgment and wait polling.
        let visible_at = outer
            .capture
            .lock()
            .unwrap()
            .visible_marker
            .take()
            .unwrap()
            .1
            .expect("outer parser did not record the visible marker");
        let elapsed = visible_at.duration_since(started);
        eprintln!(
            "{marker}: trigger to outer visible = {elapsed:?}; producer acknowledgment = {producer_ack:?}; observer return = {:?}",
            started.elapsed()
        );
        assert!(
            elapsed <= Duration::from_millis(300),
            "last dirty output exceeded the 300 ms local budget before any holder round-trip allowance: {elapsed:?}"
        );
    }
    outer.quit();
    assert!(native.received().is_empty());
}
