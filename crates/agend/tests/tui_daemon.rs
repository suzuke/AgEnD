//! The TUI against the real `agend daemon` (gate 11 B): the scenario in
//! `common/tui_process.rs` (fleet view, `retry`, live terminal, typing,
//! reconnect, `agend app` exits), and a codex instance's terminal refusing
//! input for the unapproved fake CLI version (P6). Real binary, temp homes under
//! `/tmp/g11.t-<pid>-<n>`, `fake_codex` for codex; see the modules for the
//! safety rules.
#![cfg(unix)]

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

#[path = "../../agend-daemon/tests/common/client_process.rs"]
mod clp;

#[path = "../../agend-daemon/tests/common/codex_process.rs"]
mod codex;

#[path = "common/tui_process.rs"]
mod tui;

use std::path::Path;
use std::time::Duration;

use agend_core::protocol::client::{ClientResponse, error_code};
use agend_testkit::contract::client::terminal_input;
use agend_testkit::fake_daemon::ProbeClient;

const BIN: &str = env!("CARGO_BIN_EXE_agend");

#[test]
fn the_tui_reads_and_acts_on_the_real_daemon() {
    let lab = tui::lab(Path::new(BIN));
    let lines = tui::scenario(&lab).unwrap();
    for line in &lines {
        println!("{line}");
    }
    lab.stop_all_holders();
    assert_eq!(lab.running_holders(), vec![], "holders left running");
}

#[test]
fn typing_into_an_unapproved_codex_cli_terminal_remains_not_supported() {
    let lab = tui::lab(Path::new(BIN));
    let home = lab.home(2);
    let id = format!("g11-{}x", clp::tag());
    codex::add(&home, &id, &codex::fake_codex().unwrap(), 300).unwrap();
    let _bound = codex::BoundSocket::of(&home, &id);
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    daemon.expect(&format!("{id}: go (resume ")).unwrap();
    let socket = clp::socket_of(&home);
    // Its terminal is live (it has a screen)...
    clp::terminal_screen(&socket, &id).unwrap();
    // ...but typing is refused, and nothing is written.
    let (mut c, _) = ProbeClient::hello(&socket, None).unwrap();
    c.send(&terminal_input(&id, b"x")).unwrap();
    let reply = c.recv_within(Duration::from_secs(10)).unwrap();
    let Some(ClientResponse::Error { data }) = reply else {
        panic!("expected not_supported, got {reply:?}");
    };
    assert_eq!(data.code, error_code::NOT_SUPPORTED);
    assert_eq!(
        data.message,
        "Codex terminal input requires the approved CLI 0.159.3 and a connected link with durable own-clientId receipts; nothing was written"
    );
    assert_eq!(data.request_id, None);
    daemon.interrupt().unwrap();
    lab.stop_all_holders();
    assert_eq!(lab.running_holders(), vec![], "holders left running");
}
