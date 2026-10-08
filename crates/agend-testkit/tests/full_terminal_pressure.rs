//! Native socket reader responsiveness while the producer operation is blocked.
#![cfg(unix)]
#[path = "common/terminal_parser.rs"]
mod parser;
use agend_core::protocol::client::*;
use agend_core::protocol::terminal::*;
use agend_core::traits::TerminalProducer;
use agend_testkit::contract::terminal::Window;
use agend_testkit::fake_daemon::FakeDaemon;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::{Duration, Instant};
struct Held {
    parser: parser::Parser,
    entered: SyncSender<()>,
    release: Receiver<()>,
}
impl TerminalProducer for Held {
    fn frame(
        &mut self,
        request: TerminalFrameRequest,
    ) -> Result<TerminalFrameData, TerminalOperationError> {
        self.parser.frame(request)
    }
    fn legacy_input(&mut self, bytes: String) -> Result<(), TerminalOperationError> {
        self.parser.legacy_input(bytes)
    }
    fn control(
        &mut self,
        request: TerminalControlRequest,
    ) -> Result<TerminalControlData, TerminalOperationError> {
        if matches!(&request.operation, TerminalControlOperation::Input { bytes_base64, .. } if *bytes_base64 == STANDARD.encode(b"HELD\n"))
        {
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        self.parser.control(request)
    }
}
#[test]
fn same_socket_fleet_replies_and_new_grant_waits_for_old_input() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.set_instance(InstanceView {
        program: None,
        instance_id: parser::ID.into(),
        team_id: "general".into(),
        backend: "claude".into(),
        state: AgentState::Idle,
        working_directory: None,
    });
    let real = parser::Parser::default();
    let (entered, started) = sync_channel(1);
    let (release, held) = sync_channel(1);
    daemon
        .set_terminal_producer(
            parser::ID,
            Held {
                parser: real.clone(),
                entered,
                release: held,
            },
        )
        .unwrap();
    let fx = parser::Fake {
        daemon,
        parser: real.clone(),
    };
    let mut a = Window::open(&fx, None);
    let mut b = Window::open(&fx, None);
    let old = a.acquire(
        "old",
        TerminalSize {
            rows: 8,
            columns: 32,
        },
    );
    a.client
        .send(&a.request(
            "held",
            ClientTerminalOperation::Input {
                attach_id: old,
                bytes_base64: STANDARD.encode(b"HELD\n"),
            },
        ))
        .unwrap();
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    let before = Instant::now();
    a.client
        .send(&ClientRequest::GetFleet {
            data: RequestIdData {
                request_id: "fleet".into(),
            },
        })
        .unwrap();
    assert!(matches!(a.response("fleet"), ClientResponse::Fleet { .. }));
    assert!(
        before.elapsed() < Duration::from_secs(1),
        "native operation blocked the connection reader"
    );
    b.client
        .send(&b.request(
            "new",
            ClientTerminalOperation::Acquire {
                size: TerminalSize {
                    rows: 6,
                    columns: 25,
                },
            },
        ))
        .unwrap();
    // A following fleet reply proves the new Acquire was read and enqueued.
    b.client
        .send(&ClientRequest::GetFleet {
            data: RequestIdData {
                request_id: "enqueued".into(),
            },
        })
        .unwrap();
    assert!(matches!(
        b.response("enqueued"),
        ClientResponse::Fleet { .. }
    ));
    assert!(!real.received().contains("HELD"));
    drop(a);
    release.send(()).unwrap();
    let ClientResponse::TerminalControlAck { data } = b.response("new") else {
        panic!("new grant expected after the old write")
    };
    assert_eq!(
        data.frame.unwrap().size,
        TerminalSize {
            rows: 6,
            columns: 25
        }
    );
    assert!(real.received().contains("HELD"));
    let TerminalControlState::Controlled { attach_id } = data.control else {
        panic!("control expected")
    };
    assert!(matches!(
        b.input("fresh", &attach_id, b"FRESH\n"),
        ClientResponse::TerminalControlAck { .. }
    ));
}

struct HeldFrame {
    parser: parser::Parser,
    entered: SyncSender<&'static str>,
    release: Receiver<()>,
}
impl TerminalProducer for HeldFrame {
    fn frame(
        &mut self,
        request: TerminalFrameRequest,
    ) -> Result<TerminalFrameData, TerminalOperationError> {
        let marker = match (request.request_id.as_str(), request.viewport.rows) {
            ("old-selected", _) => Some("old"),
            ("new-selected", 1) => Some("new"),
            _ => None,
        };
        if let Some(marker) = marker {
            self.entered.send(marker).unwrap();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        self.parser.frame(request)
    }
    fn legacy_input(&mut self, bytes: String) -> Result<(), TerminalOperationError> {
        self.parser.legacy_input(bytes)
    }
    fn control(
        &mut self,
        request: TerminalControlRequest,
    ) -> Result<TerminalControlData, TerminalOperationError> {
        self.parser.control(request)
    }
}
#[test]
fn a_capture_finishing_after_replacement_cannot_publish_the_old_view() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.set_instance(InstanceView {
        program: None,
        instance_id: parser::ID.into(),
        team_id: "general".into(),
        backend: "claude".into(),
        state: AgentState::Idle,
        working_directory: None,
    });
    let real = parser::Parser::default();
    let (entered, started) = sync_channel(2);
    let (release, held) = sync_channel(2);
    daemon
        .set_terminal_producer(
            parser::ID,
            HeldFrame {
                parser: real.clone(),
                entered,
                release: held,
            },
        )
        .unwrap();
    let fx = parser::Fake {
        daemon,
        parser: real,
    };
    let mut a = Window::open(&fx, None);
    let old_view = a.frame.view_id.clone();
    a.client
        .send(&ClientRequest::SetTerminalViewport {
            data: TerminalViewportData {
                request_id: "old-selected".into(),
                instance_id: parser::ID.into(),
                view_id: old_view.clone(),
                generation: a.frame.frame.generation.clone(),
                viewport: TerminalViewport { top: None, rows: 3 },
            },
        })
        .unwrap();
    assert_eq!(started.recv_timeout(Duration::from_secs(2)).unwrap(), "old");
    a.client
        .send(&ClientRequest::SubscribeTerminalFrames {
            data: TerminalSubscribeData {
                request_id: "new-selected".into(),
                instance_id: parser::ID.into(),
                viewport: TerminalViewport { top: None, rows: 4 },
            },
        })
        .unwrap();
    a.client
        .send(&ClientRequest::GetFleet {
            data: RequestIdData {
                request_id: "replacement-read".into(),
            },
        })
        .unwrap();
    assert!(matches!(
        a.response("replacement-read"),
        ClientResponse::Fleet { .. }
    ));
    release.send(()).unwrap();
    assert_eq!(started.recv_timeout(Duration::from_secs(2)).unwrap(), "new");
    let reply = a.client.recv_within(Duration::from_millis(100));
    assert!(
        matches!(&reply, Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)),
        "old capture leaked after replacement: {reply:?}"
    );
    release.send(()).unwrap();
    let ClientResponse::TerminalFrame { data } = a
        .client
        .recv_within(Duration::from_secs(2))
        .unwrap()
        .unwrap()
    else {
        panic!("new subscription frame expected")
    };
    assert_eq!(data.request_id, "new-selected");
    assert_ne!(data.view_id, old_view);
    assert_eq!(data.frame.cells.len(), 4);
}
