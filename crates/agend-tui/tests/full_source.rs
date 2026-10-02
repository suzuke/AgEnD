//! Full Source I/O uses real holder parser output and actual daemon sockets.
#![cfg(unix)]
#[path = "../../agend-testkit/tests/common/terminal_parser.rs"]
mod parser;
use agend_core::protocol::client::*;
use agend_core::protocol::terminal::*;
use agend_core::traits::TerminalProducer;
use agend_testkit::fake_daemon::FakeDaemon;
use agend_tui::source::{FullTerminalEvent as Event, Source, client::ClientSource};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

static SERIAL: Mutex<()> = Mutex::new(());

fn source(fake: &parser::Fake) -> ClientSource {
    let mut source = ClientSource::new(fake.daemon.socket_path(), None);
    source.connect().unwrap();
    source
}
fn wait(source: &mut ClientSource, mut accept: impl FnMut(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        for event in source.poll_full_terminal() {
            assert!(!matches!(event, Event::Closed(_)), "{event:?}");
            if accept(&event) {
                return event;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("full Source event timed out");
}
fn open(source: &mut ClientSource) -> ClientTerminalFrameData {
    assert!(
        source
            .open_full_terminal(
                parser::ID,
                "view".into(),
                TerminalViewport {
                    top: None,
                    rows: 20
                }
            )
            .unwrap()
    );
    match wait(source, |e| matches!(e, Event::Frame(_))) {
        Event::Frame(data) => *data,
        _ => unreachable!(),
    }
}
fn control(
    frame: &ClientTerminalFrameData,
    id: &str,
    operation: ClientTerminalOperation,
) -> ClientTerminalControlData {
    ClientTerminalControlData {
        request_id: id.into(),
        instance_id: parser::ID.into(),
        view_id: frame.view_id.clone(),
        generation: frame.frame.generation.clone(),
        operation,
    }
}
fn acquire(source: &mut ClientSource, frame: &ClientTerminalFrameData, id: &str) -> String {
    source
        .terminal_control(control(
            frame,
            id,
            ClientTerminalOperation::Acquire {
                size: TerminalSize {
                    rows: 8,
                    columns: 31,
                },
            },
        ))
        .unwrap();
    match wait(
        source,
        |e| matches!(e, Event::ControlAck(data) if data.request_id == id),
    ) {
        Event::ControlAck(data) => {
            assert_eq!(
                data.frame.as_ref().unwrap().size,
                TerminalSize {
                    rows: 8,
                    columns: 31
                }
            );
            match data.control {
                TerminalControlState::Controlled { attach_id } => attach_id,
                _ => panic!("no grant"),
            }
        }
        _ => unreachable!(),
    }
}

#[test]
fn full_source_preserves_control_identity_and_owner_refusals() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    let fake = parser::Fake::default();
    let mut a = source(&fake);
    let frame_a = open(&mut a);
    let token_a = acquire(&mut a, &frame_a, "acquire-a");
    let mut b = source(&fake);
    let frame_b = open(&mut b);
    let token_b = acquire(&mut b, &frame_b, "acquire-b");
    let event = wait(&mut a, |e| matches!(e, Event::ControlChanged(_)));
    assert!(
        matches!(event, Event::ControlChanged(data) if data.view_id == frame_a.view_id && data.control == TerminalControlState::ReadOnly)
    );
    a.terminal_control(control(
        &frame_a,
        "old-owner",
        agend_client::terminal::input_operation(&token_a, b"DENIED\n"),
    ))
    .unwrap();
    assert!(
        matches!(wait(&mut a, |e| matches!(e, Event::Refused(_))), Event::Refused(data) if data.request_id.as_deref() == Some("old-owner") && data.code == "control_lost")
    );
    b.terminal_control(control(
        &frame_b,
        "input",
        agend_client::terminal::input_operation(&token_b, b"VALID\n"),
    ))
    .unwrap();
    assert!(matches!(
        wait(
            &mut b,
            |e| matches!(e, Event::ControlAck(data) if data.request_id == "input")
        ),
        Event::ControlAck(_)
    ));
    assert!(fake.parser.received().contains("VALID"));
    assert!(!fake.parser.received().contains("DENIED"));
    assert!(a.poll().unwrap().is_empty());
    b.terminal_viewport(TerminalViewportData {
        request_id: "viewport".into(),
        instance_id: parser::ID.into(),
        view_id: frame_b.view_id,
        generation: frame_b.frame.generation,
        viewport: TerminalViewport { top: None, rows: 4 },
    })
    .unwrap();
    assert!(
        matches!(wait(&mut b, |e| matches!(e, Event::Frame(data) if data.request_id == "viewport")), Event::Frame(data) if data.frame.cells.len() == 4)
    );
}

#[test]
fn an_old_daemon_keeps_full_capability_disabled() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    let daemon = FakeDaemon::start().unwrap();
    let mut source = ClientSource::new(daemon.socket_path(), None);
    source.connect().unwrap();
    assert!(
        !source
            .open_full_terminal(
                "old",
                "view".into(),
                TerminalViewport {
                    top: None,
                    rows: 20
                }
            )
            .unwrap()
    );
    assert_eq!(
        source.threads().load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(source.poll_full_terminal().is_empty());
}

struct Held {
    parser: parser::Parser,
    entered: mpsc::SyncSender<()>,
    release: mpsc::Receiver<()>,
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
        if matches!(request.operation, TerminalControlOperation::Input { .. }) {
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        self.parser.control(request)
    }
}
#[test]
fn queued_input_does_not_block_source_polling_or_replay_after_close() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    let fake = parser::Fake::default();
    let (entered, seen) = mpsc::sync_channel(1);
    let (release, released) = mpsc::sync_channel(1);
    fake.daemon
        .set_terminal_producer(
            parser::ID,
            Held {
                parser: fake.parser.clone(),
                entered,
                release: released,
            },
        )
        .unwrap();
    let mut source = source(&fake);
    let frame = open(&mut source);
    let token = acquire(&mut source, &frame, "acquire");
    let started = Instant::now();
    source
        .terminal_control(control(
            &frame,
            "held",
            agend_client::terminal::input_operation(&token, b"HELD\n"),
        ))
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(100));
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    for _ in 0..40 {
        source
            .terminal_control(control(
                &frame,
                "queued",
                agend_client::terminal::input_operation(&token, b"DENIED-REPLAY\n"),
            ))
            .ok();
    }
    let started = Instant::now();
    assert!(source.poll().unwrap().is_empty());
    source.poll_full_terminal();
    source.close_terminal();
    assert!(started.elapsed() < Duration::from_secs(1));
    let deadline = Instant::now() + Duration::from_secs(3);
    while fake.daemon.open_connections() != 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        fake.daemon.open_connections(),
        2,
        "old full socket did not close"
    );
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !fake.parser.received().contains("HELD") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(fake.parser.received().contains("HELD"));
    // Synchronize with cleanup through a new actual view/grant, rather than
    // interpreting a quiet interval as proof that old queued work was retired.
    let next_frame = open(&mut source);
    acquire(&mut source, &next_frame, "next-grant");
    assert!(!fake.parser.received().contains("DENIED-REPLAY"));
}

#[test]
fn twenty_full_source_cycles_join_workers_and_release_descriptors() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    let fake = parser::Fake::default();
    let mut source = source(&fake);
    let threads = source.threads();
    let descriptors = || std::fs::read_dir("/dev/fd").unwrap().count();
    let baseline = descriptors();
    for i in 0..20 {
        let frame = open(&mut source);
        acquire(&mut source, &frame, &format!("grant-{i}"));
        assert_eq!(threads.load(std::sync::atomic::Ordering::SeqCst), 3);
        source.close_terminal();
        assert_eq!(threads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while (fake.daemon.open_connections() != 2 || descriptors() != baseline)
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fake.daemon.open_connections(), 2);
    assert_eq!(descriptors(), baseline);
    drop(source);
    assert_eq!(threads.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn a_slow_ui_is_closed_when_ordered_replies_reach_the_bound() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    let fake = parser::Fake::default();
    let mut source = source(&fake);
    let frame = open(&mut source);
    let token = acquire(&mut source, &frame, "initial-grant");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut sent = 0;
    // Do not drain full replies: the actual parser acknowledges each input.
    // A bounded transport must close explicitly rather than silently dropping
    // a control reply or growing indefinitely behind a stalled renderer.
    while Instant::now() < deadline {
        match source.terminal_control(control(
            &frame,
            &format!("input-{sent}"),
            agend_client::terminal::input_operation(&token, b"x"),
        )) {
            Ok(()) => {
                sent += 1;
                // Stay below the server reply-forwarder rate; this test
                // isolates the UI mailbox bound, not slow socket eviction.
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(agend_tui::source::SourceError::Rejected { code, .. }) if code == "busy" => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(agend_tui::source::SourceError::Disconnected(_)) => break,
            other => panic!("unexpected queue result: {other:?}"),
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut closed = None;
    while closed.is_none() && Instant::now() < deadline {
        for event in source.poll_full_terminal() {
            if let Event::Closed(reason) = event {
                closed = Some(reason);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        closed
            .as_ref()
            .is_some_and(|reason| reason.contains("reply queue overflow")),
        "missing explicit overflow close: {closed:?}, sent={sent}"
    );
    assert_eq!(
        source.threads().load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    let fresh_frame = open(&mut source);
    let fresh_token = acquire(&mut source, &fresh_frame, "fresh-grant");
    assert_ne!(fresh_token, token);
}
