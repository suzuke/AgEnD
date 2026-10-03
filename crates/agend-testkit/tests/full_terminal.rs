//! C shared contracts on the fake server with the actual holder parser.
#![cfg(unix)]
#[path = "common/terminal_parser.rs"]
mod parser;
use agend_testkit::contract::terminal;

#[test]
fn full_terminal_contracts_pass_on_the_fake_with_real_parser_frames() {
    let report = terminal::run("fake with holder parser", || {
        Box::new(parser::Fake::default())
    });
    println!("{report}");
    report.assert_passed();
}

#[test]
fn producer_generation_and_instance_stop_make_the_owner_read_only() {
    use agend_core::protocol::client::*;
    use agend_core::protocol::terminal::*;
    use std::time::Duration;
    use terminal::{FullTerminalFixture, Window};
    let fx = parser::Fake::default();
    let mut a = Window::open(&fx, None);
    let attach = a.acquire(
        "a",
        TerminalSize {
            rows: 8,
            columns: 30,
        },
    );
    fx.parser.restart();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let response = a
            .client
            .recv_within(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap()
            .unwrap();
        if matches!(response, ClientResponse::TerminalControlChanged { data } if data.control == TerminalControlState::ReadOnly)
        {
            break;
        }
    }
    assert!(
        matches!(a.response("a"), ClientResponse::Error { data } if data.code == "stale_terminal")
    );
    assert!(
        matches!(a.input("old-generation", &attach, b"DENIED-OLD\n"), ClientResponse::Error { data } if data.code == "stale_terminal")
    );
    let mut fresh = Window::open(&fx, None);
    assert_ne!(fresh.frame.frame.generation, a.frame.frame.generation);
    let token = fresh.acquire(
        "fresh",
        TerminalSize {
            rows: 9,
            columns: 31,
        },
    );
    fx.daemon.set_instance(InstanceView {
        instance_id: fx.instance(),
        team_id: "general".into(),
        backend: "claude".into(),
        state: AgentState::Failed,
        working_directory: None,
    });
    assert!(
        matches!(fresh.input("closed", &token, b"DENIED-CLOSED\n"), ClientResponse::Error { data } if data.code == "no_terminal")
    );
    assert!(!fx.parser.received().contains("DENIED-"));
}

#[test]
fn restart_closes_full_scopes_but_keeps_producer_generation_and_last_size() {
    use agend_core::protocol::client::*;
    use agend_core::protocol::terminal::*;
    use agend_testkit::fake_daemon::ProbeClient;
    use terminal::{FullTerminalFixture, Window};
    let fx = parser::Fake::default();
    let mut old = Window::open(&fx, None);
    let token = old.acquire(
        "old",
        TerminalSize {
            rows: 9,
            columns: 27,
        },
    );
    let generation = old.frame.frame.generation.clone();
    let old_request = old.request(
        "old-after-restart",
        ClientTerminalOperation::Input {
            attach_id: token,
            bytes_base64: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                b"DENIED-REPLAY\n",
            ),
        },
    );
    let (mut control, _) = ProbeClient::hello(&fx.socket(), None).unwrap();
    let reply = control
        .request(&ClientRequest::Operator {
            data: OperatorData {
                request_id: "restart".into(),
                command: OperatorCommand::DaemonRestart { binary: None },
            },
        })
        .unwrap();
    assert!(
        matches!(reply, ClientResponse::CommandResult { data } if matches!(data.result, CommandResult::Restarting { .. }))
    );
    let mut fresh = Window::open(&fx, None);
    assert_eq!(fresh.frame.frame.generation, generation);
    assert_eq!(
        fresh.frame.frame.size,
        TerminalSize {
            rows: 9,
            columns: 27
        }
    );
    fresh.client.send(&old_request).unwrap();
    assert!(
        matches!(fresh.response("old-after-restart"), ClientResponse::Error { data } if data.code == "stale_terminal")
    );
    let next = fresh.acquire(
        "fresh",
        TerminalSize {
            rows: 7,
            columns: 23,
        },
    );
    assert!(matches!(
        fresh.input("valid", &next, b"AFTER-RESTART\n"),
        ClientResponse::TerminalControlAck { .. }
    ));
    assert!(!fx.received().contains("DENIED-REPLAY"));
}
