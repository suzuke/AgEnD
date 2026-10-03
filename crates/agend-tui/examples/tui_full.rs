//! Interactive full-terminal demo: real holder parser behind FakeDaemon.
//! No real agent, model call, or persistent user home. F keys are demo-only.
#[allow(dead_code)]
#[path = "../../agend-testkit/tests/common/terminal_parser.rs"]
mod parser;
use agend_core::protocol::client::{AgentState, InstanceView};
use agend_core::protocol::terminal::*;
use agend_core::traits::TerminalProducer;
use agend_testkit::{fake_daemon::FakeDaemon, tempdir::TempDir};
use agend_tui::{i18n::Language, source::client::ClientSource};
use base64::{Engine, engine::general_purpose::STANDARD};
use ratatui::crossterm::event::KeyCode;
use std::{io, path::Path};

#[derive(Clone)]
struct Demo(parser::Parser);
impl TerminalProducer for Demo {
    fn frame(
        &mut self,
        request: TerminalFrameRequest,
    ) -> Result<TerminalFrameData, TerminalOperationError> {
        self.0.frame(request)
    }
    fn legacy_input(&mut self, bytes: String) -> Result<(), TerminalOperationError> {
        self.0.legacy_input(bytes)
    }
    fn control(
        &mut self,
        request: TerminalControlRequest,
    ) -> Result<TerminalControlData, TerminalOperationError> {
        let bytes = match &request.operation {
            TerminalControlOperation::Input { bytes_base64, .. } => {
                STANDARD.decode(bytes_base64).ok()
            }
            _ => None,
        };
        let response = self.0.control(request)?;
        if let Some(bytes) = bytes {
            // Render actual received bytes through Screen, never ideal cells.
            self.0
                .feed(format!("\r\nINPUT BYTES: {bytes:?}\r\n").as_bytes());
        } else if response.attach_id.is_some() {
            let size = self
                .0
                .frame(TerminalFrameRequest {
                    request_id: "demo-size".into(),
                    viewport: TerminalViewport { top: None, rows: 1 },
                })?
                .frame
                .size;
            self.0.feed(
                format!(
                    "\r\nPARSER SIZE: {} rows x {} columns\r\n",
                    size.rows, size.columns
                )
                .as_bytes(),
            );
        }
        Ok(response)
    }
}
fn daemon(socket: &Path, producer: &Demo) -> io::Result<FakeDaemon> {
    let daemon = FakeDaemon::start_at(socket)?;
    daemon.set_instance(InstanceView {
        instance_id: parser::ID.into(),
        team_id: "general".into(),
        backend: "claude".into(),
        state: AgentState::Idle,
        working_directory: None,
    });
    daemon
        .set_terminal_producer(parser::ID, producer.clone())
        .map_err(io::Error::other)?;
    Ok(daemon)
}
fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 2 && args[0] == "--socket" {
        let socket = std::path::PathBuf::from(&args[1]);
        if !socket.is_absolute() || !socket.exists() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "use an existing absolute demo socket",
            ));
        }
        return agend_tui::run(Box::new(ClientSource::new(&socket, None)), Language::ZhTw);
    }
    if !args.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: tui_full [--socket <existing demo socket>]",
        ));
    }
    let dir = TempDir::new("g11-full-demo")?;
    let socket = dir.path().join("daemon.sock");
    let producer = Demo(parser::Parser::default());
    producer
        .0
        .feed(b"\x1b[2J\x1b[H\x1b[38;2;80;200;160mFULL TERMINAL DEMO\x1b[0m\r\n");
    producer.0.feed("繁中 · 界 · e\u{301}\r\n".as_bytes());
    producer.0.feed(b"F1 socket; F2 reconnect; F3 mouse; F4 numbered history; F5 alternate\r\n\x1b[?2004h\x1b[?25h\x1b[6 q");
    let mut running = Some(daemon(&socket, &producer)?);
    let source = ClientSource::new(&socket, None);
    let mut mouse = false;
    let mut alternate = false;
    let result = agend_tui::run_with(Box::new(source), Language::ZhTw, |key| match key.code {
        KeyCode::F(1) => {
            producer
                .0
                .feed(format!("\r\nDEMO SOCKET: {}\r\n", socket.display()).as_bytes());
            true
        }
        KeyCode::F(2) => {
            if running.take().is_none() {
                match daemon(&socket, &producer) {
                    Ok(daemon) => running = Some(daemon),
                    Err(error) => eprintln!("demo reconnect failed: {error}"),
                }
            }
            true
        }
        KeyCode::F(3) => {
            mouse = !mouse;
            producer.0.feed(if mouse {
                b"\x1b[?1000h\x1b[?1006h\r\nMOUSE ON\r\n"
            } else {
                b"\x1b[?1000l\x1b[?1006l\r\nMOUSE OFF\r\n"
            });
            true
        }
        KeyCode::F(4) => {
            for n in 0..1200 {
                producer.0.feed(format!("history-{n}\r\n").as_bytes());
            }
            producer.0.feed(b"HISTORY END\r\n");
            true
        }
        KeyCode::F(5) => {
            alternate = !alternate;
            producer.0.feed(if alternate {
                b"\x1b[?1049h\x1b[HALTERNATE SCREEN\r\n"
            } else {
                b"\x1b[?1049l"
            });
            true
        }
        _ => false,
    });
    eprintln!("Full demo: select g11-contract with /; t read-only, i input, Ctrl-] back, q quit.");
    result
}
