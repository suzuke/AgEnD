//! Drive the TUI by hand against scripted data (no real daemon, no agents).
//!
//! ```text
//! cargo run -p agend-tui --example tui_fake                  # scripted, in memory
//! cargo run -p agend-tui --example tui_fake -- --daemon      # testkit fake daemon, through agend-client
//! cargo run -p agend-tui --example tui_fake -- --lang zh-TW
//! ```
//!
//! The loop is `agend_tui::run_with`, the same as `agend app`. Demo-only
//! keys (handled here, not by the TUI): F2 stops the daemon, or starts it
//! again; F3 (scripted only) makes dev-2 follow up on ask A-1.

#[path = "support/demo_daemon.rs"]
mod demo_daemon;

use std::io;
use std::path::PathBuf;

use agend_testkit::fake_daemon::FakeDaemon;
use agend_testkit::tempdir::TempDir;
use agend_tui::i18n::Language;
use agend_tui::source::Source;
use agend_tui::source::client::ClientSource;
use agend_tui::source::scripted::{ScriptHandle, ScriptedSource};
use ratatui::crossterm::event::KeyCode;

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
        socket: PathBuf,
        _dir: TempDir,
    },
}

impl Backing {
    /// F2: stop the daemon, or start it again (on the same socket).
    fn toggle(&mut self) {
        match self {
            Backing::Scripted { handle, online } => {
                *online = !*online;
                handle.set_online(*online);
            }
            Backing::Daemon { daemon, socket, .. } => match daemon.take() {
                Some(running) => drop(running),
                None => {
                    if let Ok(fresh) = FakeDaemon::start_at(socket) {
                        demo_daemon::seed(&fresh);
                        *daemon = Some(fresh);
                    }
                }
            },
        }
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

    let (source, mut backing): (Box<dyn Source>, Backing) = if use_daemon {
        let dir = TempDir::new("tui-fake")?;
        let socket = dir.path().join("daemon.sock");
        let daemon = FakeDaemon::start_at(&socket)?;
        demo_daemon::seed(&daemon);
        let source = ClientSource::new(&socket, None);
        (
            Box::new(source),
            Backing::Daemon {
                daemon: Some(daemon),
                socket,
                _dir: dir,
            },
        )
    } else {
        let (source, handle) = ScriptedSource::demo();
        (
            Box::new(source),
            Backing::Scripted {
                handle,
                online: true,
            },
        )
    };

    let result = agend_tui::run_with(source, lang, |key| match key.code {
        KeyCode::F(2) => {
            backing.toggle();
            true
        }
        KeyCode::F(3) => {
            if let Backing::Scripted { handle, .. } = &backing {
                handle.follow_up(
                    "A-1",
                    "dev-2",
                    "Seed 42 hides the flake. Also run 20 times nightly?",
                    &["yes", "no"],
                );
            }
            true
        }
        _ => false,
    });
    eprintln!("{USAGE}");
    result
}
