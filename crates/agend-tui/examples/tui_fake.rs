//! Drive the TUI by hand against scripted data (no real daemon, no agents).
//!
//! ```text
//! cargo run -p agend-tui --example tui_fake                  # scripted, in memory
//! cargo run -p agend-tui --example tui_fake -- --daemon      # testkit fake daemon over its socket
//! cargo run -p agend-tui --example tui_fake -- --lang zh-TW
//! ```
//!
//! Demo-only keys (handled here, not by the TUI): F2 stops the daemon, or
//! starts it again; F3 (scripted only) makes dev-2 follow up on ask A-1.

#[path = "support/daemon_source.rs"]
mod daemon_source;

use std::io;
use std::time::{Duration, Instant};

use agend_testkit::fake_daemon::FakeDaemon;
use agend_tui::i18n::Language;
use agend_tui::source::scripted::{ScriptHandle, ScriptedSource, demo_catalog};
use agend_tui::{App, ui};
use ratatui::crossterm::event::{self, Event, KeyCode};

const USAGE: &str = "usage: tui_fake [--daemon] [--lang en|zh-TW]
Scripted data only; nothing real runs. Keys: q quit, L language, F2 stop/start
the daemon (see the disconnected state), F3 dev-2 follows up on A-1 (scripted).";

enum Backing {
    Scripted {
        handle: ScriptHandle,
        online: bool,
    },
    Daemon {
        daemon: Option<FakeDaemon>,
        address: daemon_source::Address,
    },
}

impl Backing {
    /// F2: stop the daemon, or start it again.
    fn toggle(&mut self) -> io::Result<()> {
        match self {
            Backing::Scripted { handle, online } => {
                *online = !*online;
                handle.set_online(*online);
            }
            Backing::Daemon { daemon, address } => match daemon.take() {
                Some(running) => drop(running),
                None => {
                    let fresh = FakeDaemon::start()?;
                    daemon_source::seed_demo(&fresh);
                    *address.lock().unwrap_or_else(|e| e.into_inner()) =
                        fresh.socket_path().to_path_buf();
                    *daemon = Some(fresh);
                }
            },
        }
        Ok(())
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut lang = Language::En;
    let mut use_daemon = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--daemon" => use_daemon = true,
            "--lang" => {
                lang = args
                    .next()
                    .and_then(|v| Language::parse(v))
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, USAGE))?
            }
            _ => {
                eprintln!("{USAGE}");
                return Ok(());
            }
        }
    }

    let (app, mut backing) = if use_daemon {
        let daemon = FakeDaemon::start()?;
        daemon_source::seed_demo(&daemon);
        let (source, address) =
            daemon_source::DaemonSource::new(daemon.socket_path().to_path_buf(), demo_catalog());
        let app = App::new(Box::new(source), lang);
        (
            app,
            Backing::Daemon {
                daemon: Some(daemon),
                address,
            },
        )
    } else {
        let (source, handle) = ScriptedSource::demo();
        let app = App::new(Box::new(source), lang);
        (
            app,
            Backing::Scripted {
                handle,
                online: true,
            },
        )
    };

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, app, &mut backing);
    ratatui::restore();
    eprintln!("{USAGE}");
    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    mut app: App,
    backing: &mut Backing,
) -> io::Result<()> {
    let mut last_tick: Option<Instant> = None;
    while !app.quit {
        if last_tick.is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
            app.tick();
            last_tick = Some(Instant::now());
        }
        terminal.draw(|frame| ui::render(frame, &mut app))?;
        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::F(2) => backing.toggle()?,
                KeyCode::F(3) => {
                    if let Backing::Scripted { handle, .. } = backing {
                        handle.follow_up(
                            "A-1",
                            "dev-2",
                            "Seed 42 hides the flake. Also run 20 times nightly?",
                            &["yes", "no"],
                        );
                    }
                }
                _ => app.key(key),
            }
        }
    }
    Ok(())
}
