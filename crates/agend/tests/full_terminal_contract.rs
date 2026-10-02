//! The same client 1.4 cases on fake parser-backed and native daemon fixtures.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/client_process.rs"]
mod clp;
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
#[path = "../../agend-testkit/tests/common/terminal_parser.rs"]
mod parser;
use agend_core::model::Backend;
use agend_core::protocol::client::*;
use agend_testkit::contract::terminal::{self, FullTerminalFixture};
use agend_testkit::fake_daemon::ProbeClient;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::fs;
use std::path::{Path, PathBuf};

struct Native {
    _lab: lab::Lab,
    daemon: Option<lab::Daemon>,
    home: PathBuf,
}
impl Default for Native {
    fn default() -> Self {
        let native = lab::Lab::with_prefix(Path::new(env!("CARGO_BIN_EXE_agend")), "g11ct");
        let home = native.home(1);
        clp::add(&home, parser::ID, Backend::Claude, parser::SCRIPT).unwrap();
        let mut daemon = lab::Daemon::start(&native, &home, &[]).unwrap();
        daemon.ready().unwrap();
        Self {
            _lab: native,
            daemon: Some(daemon),
            home,
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.interrupt();
        }
    }
}
impl FullTerminalFixture for Native {
    fn socket(&self) -> PathBuf {
        clp::socket_of(&self.home)
    }
    fn instance(&self) -> String {
        parser::ID.into()
    }
    fn received(&self) -> String {
        fs::read_to_string(
            self.home
                .join("workspace")
                .join(parser::ID)
                .join("delivered"),
        )
        .unwrap_or_default()
    }
    fn output(&mut self) {
        let previous = self
            .received()
            .lines()
            .filter(|line| *line == parser::BURST)
            .count();
        let (mut client, _) = ProbeClient::hello(&self.socket(), None).unwrap();
        client
            .send(&ClientRequest::TerminalInput {
                data: TerminalInputData {
                    instance_id: parser::ID.into(),
                    bytes_base64: STANDARD.encode(format!("{}\n", parser::BURST)),
                },
            })
            .unwrap();
        assert!(matches!(
            client
                .request(&ClientRequest::GetFleet {
                    data: RequestIdData {
                        request_id: "after-output".into()
                    }
                })
                .unwrap(),
            ClientResponse::Fleet { .. }
        ));
        // Keep the legacy socket alive until its queued write reaches the
        // consumer, rather than mistaking get_fleet for an input ack.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self
            .received()
            .lines()
            .filter(|line| *line == parser::BURST)
            .count()
            <= previous
        {
            assert!(
                std::time::Instant::now() < deadline,
                "burst did not reach the PTY consumer"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
#[test]
fn full_terminal_contracts_match_fake_and_native_daemon() {
    let fake = terminal::run("fake with holder parser", || {
        Box::new(parser::Fake::default())
    });
    println!("{fake}");
    fake.assert_passed();
    let native = terminal::run("native daemon / holder / PTY", || {
        Box::new(Native::default())
    });
    println!("{native}");
    native.assert_passed();
}
