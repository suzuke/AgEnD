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

#[test]
fn full_tui_source_controls_the_real_daemon_and_native_pty() {
    use agend_core::protocol::terminal::{TerminalSize, TerminalViewport};
    use agend_tui::source::{FullTerminalEvent as Event, Source, client::ClientSource};
    use std::time::{Duration, Instant};
    fn wait(source: &mut ClientSource, accept: impl Fn(&Event) -> bool) -> Event {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            for event in source.poll_full_terminal() {
                assert!(
                    !matches!(event, Event::Closed(_) | Event::Refused(_)),
                    "{event:?}"
                );
                if accept(&event) {
                    return event;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("native full Source timed out");
    }
    let native = Native::default();
    let mut source = ClientSource::new(&native.socket(), None);
    source.connect().unwrap();
    let threads = source.threads();
    assert!(
        source
            .open_full_terminal(
                parser::ID,
                "native-view".into(),
                TerminalViewport {
                    top: None,
                    rows: 20
                }
            )
            .unwrap()
    );
    let Event::Frame(frame) = wait(
        &mut source,
        |event| matches!(event, Event::Frame(data) if data.frame.cells.iter().flat_map(|row| row.iter()).map(|cell| cell.text.as_str()).collect::<String>().contains("READY")),
    ) else {
        unreachable!()
    };
    let data = |id: &str, operation| ClientTerminalControlData {
        request_id: id.into(),
        instance_id: parser::ID.into(),
        view_id: frame.view_id.clone(),
        generation: frame.frame.generation.clone(),
        operation,
    };
    source
        .terminal_control(data(
            "native-grant",
            ClientTerminalOperation::Acquire {
                size: TerminalSize {
                    rows: 12,
                    columns: 40,
                },
            },
        ))
        .unwrap();
    let Event::ControlAck(grant) = wait(
        &mut source,
        |event| matches!(event, Event::ControlAck(data) if data.request_id == "native-grant"),
    ) else {
        unreachable!()
    };
    assert_eq!(
        grant.frame.as_ref().unwrap().size,
        TerminalSize {
            rows: 12,
            columns: 40
        }
    );
    let TerminalControlState::Controlled { attach_id } = grant.control else {
        panic!("native owner grant missing")
    };
    let at = Instant::now();
    source
        .terminal_control(data(
            "native-input",
            agend_client::terminal::input_operation(&attach_id, b"SOURCE-PTY\n"),
        ))
        .unwrap();
    assert!(
        at.elapsed() < Duration::from_millis(100),
        "Source waited for PTY ack"
    );
    wait(
        &mut source,
        |event| matches!(event, Event::ControlAck(data) if data.request_id == "native-input"),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !native.received().contains("SOURCE-PTY") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        native.received().contains("SOURCE-PTY"),
        "actual native PTY consumer did not receive input"
    );
    source.close_terminal();
    assert_eq!(threads.load(std::sync::atomic::Ordering::SeqCst), 1);
    drop(source);
    assert_eq!(threads.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn small_socket_buffers_reject_large_input_and_keep_native_consumer_live() {
    use agend_core::protocol::terminal::{TerminalSize, TerminalViewport};
    use agend_testkit::contract::terminal::Window;
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};
    let native = Native::default();
    let mut window = Window::open(&native, None);
    let attach = window.acquire(
        "duplex-owner",
        TerminalSize {
            rows: 8,
            columns: 32,
        },
    );
    let socket = window.client.writer_clone().unwrap();
    let buffer: libc::c_int = 4096;
    for option in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    option,
                    (&buffer as *const libc::c_int).cast(),
                    std::mem::size_of_val(&buffer) as libc::socklen_t,
                )
            },
            0
        );
    }
    // Keep the native server's actual parser frame pending while the large
    // request is sent. This recreates the duplex buffering pressure in CI.
    window
        .client
        .send(&ClientRequest::SetTerminalViewport {
            data: TerminalViewportData {
                request_id: "duplex-pending-frame".into(),
                instance_id: window.frame.instance_id.clone(),
                view_id: window.frame.view_id.clone(),
                generation: window.frame.frame.generation.clone(),
                viewport: TerminalViewport { top: None, rows: 8 },
            },
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        matches!(window.input("duplex-oversized",&attach,&vec![b'x';agend_core::protocol::holder::MAX_REQUEST_LINE]),ClientResponse::Error {data} if data.code==error_code::INVALID_REQUEST)
    );
    assert!(matches!(
        window.input("duplex-valid-after", &attach, b"DUPLEX-NATIVE-AFTER\n"),
        ClientResponse::TerminalControlAck { .. }
    ));
    assert_eq!(window.frame.request_id, "duplex-pending-frame");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !native.received().contains("DUPLEX-NATIVE-AFTER") {
        assert!(
            Instant::now() < deadline,
            "valid subsequent input did not reach native consumer"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        native.received().trim(),
        "DUPLEX-NATIVE-AFTER",
        "oversized input reached native PTY consumer"
    );
    drop(window);
    let root = native._lab.root.clone();
    drop(native);
    assert!(!root.exists(), "own native duplex lab leaked");
}

#[test]
fn small_socket_buffers_accept_valid_input_below_the_previous_duplex_threshold() {
    use agend_core::protocol::terminal::{TerminalSize, TerminalViewport};
    use agend_testkit::contract::terminal::Window;
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};
    let native = Native::default();
    let mut window = Window::open(&native, None);
    let attach = window.acquire(
        "duplex-owner",
        TerminalSize {
            rows: 8,
            columns: 32,
        },
    );
    let socket = window.client.writer_clone().unwrap();
    let buffer: libc::c_int = 4096;
    for option in [libc::SO_SNDBUF, libc::SO_RCVBUF] {
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    option,
                    (&buffer as *const libc::c_int).cast(),
                    std::mem::size_of_val(&buffer) as libc::socklen_t,
                )
            },
            0
        );
    }
    // Keep a real parser frame pending while sending valid input below the
    // previous 64 KiB threshold. Both directions can exhaust their buffers.
    window
        .client
        .send(&ClientRequest::SetTerminalViewport {
            data: TerminalViewportData {
                request_id: "duplex-pending-frame".into(),
                instance_id: window.frame.instance_id.clone(),
                view_id: window.frame.view_id.clone(),
                generation: window.frame.frame.generation.clone(),
                viewport: TerminalViewport { top: None, rows: 8 },
            },
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let mut line = vec![b'x'; 256];
    line.push(b'\n');
    let payload = line.repeat(96);
    let encoded = serde_json::to_vec(&window.request(
        "duplex-valid-24k",
        ClientTerminalOperation::Input {
            attach_id: attach.clone(),
            bytes_base64: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &payload,
            ),
        },
    ))
    .unwrap();
    assert!(encoded.len() < 64 * 1024);
    eprintln!(
        "4KiB duplex buffers; serialized valid request {} bytes",
        encoded.len()
    );
    assert!(matches!(
        window.input("duplex-valid-24k", &attach, &payload),
        ClientResponse::TerminalControlAck { .. }
    ));
    assert!(matches!(
        window.input("duplex-valid-after", &attach, b"DUPLEX-NATIVE-AFTER\n"),
        ClientResponse::TerminalControlAck { .. }
    ));
    assert_eq!(window.frame.request_id, "duplex-pending-frame");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !native.received().contains("DUPLEX-NATIVE-AFTER") {
        assert!(
            Instant::now() < deadline,
            "valid subsequent input did not reach native consumer"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        native.received().bytes().filter(|b| *b == b'x').count(),
        24 * 1024
    );
    drop(window);
    let root = native._lab.root.clone();
    drop(native);
    assert!(!root.exists(), "own native duplex lab leaked");
}
