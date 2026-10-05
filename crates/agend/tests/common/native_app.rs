//! Shared true daemon/holder/raw PTY fixture for direct and outer App tests.
#![allow(dead_code)]
use crate::{clp, lab};
use agend_core::model::Backend;
use agend_core::protocol::terminal::{TerminalFrame, TerminalSize};
use agend_tui::{App, i18n::Language, source::client::ClientSource};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
pub const ID: &str = "g11-raw";
static LAB: std::sync::Mutex<()> = std::sync::Mutex::new(());

// Re-exec this test binary as the agent. The outer test invocation is a no-op;
// only the holder-provided instance identity enters the raw consumer loop.
pub fn raw_agent() {
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
pub struct Native {
    pub lab: lab::Lab,
    pub home: PathBuf,
    pub daemon: Option<lab::Daemon>,
    sequence: u64,
    // Release the fixture slot only after the lab's holder/fd cleanup.
    _permit: std::sync::MutexGuard<'static, ()>,
}
impl Native {
    pub fn new() -> Self {
        Self::with_startup_sampling(false)
    }
    pub fn with_startup_sampling(automatic: bool) -> Self {
        let permit = LAB.lock().unwrap_or_else(|error| error.into_inner());
        let lab = lab::Lab::with_prefix(Path::new(env!("CARGO_BIN_EXE_agend")), "g11app");
        let home = lab.home(1);
        let executable = std::env::current_exe()
            .unwrap()
            .display()
            .to_string()
            .replace('\'', "'\\''");
        let instance = clp::add(
            &home,
            ID,
            Backend::Claude,
            &format!("exec '{executable}' --exact raw_pty_agent --nocapture"),
        )
        .unwrap();
        // This raw PTY consumer has no Claude startup protocol. Keep its
        // terminal test independent of automatic backend initialization.
        if !automatic {
            let store = agend_daemon::store::SqliteStore::open(&home, 0).unwrap();
            agend_testkit::block_on(
                store.manual_claude_startup(ID, instance.session_id.as_deref().unwrap()),
            )
            .unwrap();
            drop(store);
        }
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
    pub fn workspace(&self) -> PathBuf {
        self.home.join("workspace").join(ID)
    }
    pub fn received(&self) -> Vec<u8> {
        fs::read(self.workspace().join("received")).unwrap_or_default()
    }
    pub fn output(&mut self, bytes: &[u8]) {
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
    pub fn actual_size(&mut self) -> TerminalSize {
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
    pub fn app(
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
    pub fn expect(&self, app: &mut App, expected: &[u8]) {
        wait(app, |_| {
            let received = self.received();
            assert!(
                expected.starts_with(&received),
                "unexpected native input: {received:?}; expected {expected:?}"
            );
            received == expected
        });
    }
    pub fn restart(&mut self) {
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
pub fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}
pub fn wait(app: &mut App, ready: impl Fn(&App) -> bool) {
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
pub fn frame_opt(app: &App) -> Option<&TerminalFrame> {
    app.term
        .as_ref()?
        .full
        .as_ref()?
        .data
        .as_ref()
        .map(|data| &data.frame)
}
pub fn frame(app: &App) -> &TerminalFrame {
    frame_opt(app).unwrap()
}
pub fn text(frame: &TerminalFrame) -> String {
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
pub fn acquire(app: &mut App) {
    key(app, KeyCode::Char('i'));
    wait(app, |app| app.term.as_ref().is_some_and(|term| term.typing));
}
pub fn mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) {
    app.event(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }));
}
