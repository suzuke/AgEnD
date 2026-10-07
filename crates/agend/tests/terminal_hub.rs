//! Gate 11C's full client path: real daemon, holder, parser and native PTY.
//! No constructed frame fixtures and no real LLM calls.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/client_process.rs"]
mod clp;
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

use agend_core::model::Backend;
use agend_core::protocol::client::*;
use agend_core::protocol::terminal::*;
use agend_testkit::fake_daemon::ProbeClient;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const ID: &str = "g11-hub";
const BIN: &str = env!("CARGO_BIN_EXE_agend");
const SHELL: &str = "stty -echo; printf 'READY\\r\\n'; while IFS= read -r line; do printf '%s\\n' \"$line\" >> delivered; printf 'GOT:%s\\r\\n' \"$line\"; done";
struct Lab {
    native: lab::Lab,
    home: PathBuf,
    daemon: Option<lab::Daemon>,
}
impl Lab {
    fn start(script: &str) -> Self {
        let native = lab::Lab::with_prefix(Path::new(BIN), "g11h");
        let home = native.home(1);
        clp::add(&home, ID, Backend::Claude, script).unwrap();
        let mut daemon = lab::Daemon::start(&native, &home, &[]).unwrap();
        daemon.ready().unwrap();
        Self {
            native,
            home,
            daemon: Some(daemon),
        }
    }
    fn socket(&self) -> PathBuf {
        clp::socket_of(&self.home)
    }
    fn window(&self, id: &str, rows: u16) -> Window {
        Window::open(&self.socket(), None, id, rows)
    }
    fn delivered(&self) -> String {
        fs::read_to_string(self.home.join("workspace").join(ID).join("delivered"))
            .unwrap_or_default()
    }
    fn restart(&mut self) {
        self.daemon.take().unwrap().interrupt().unwrap();
        let mut daemon = lab::Daemon::start(&self.native, &self.home, &[]).unwrap();
        daemon.ready().unwrap();
        self.daemon = Some(daemon);
    }
}
impl Drop for Lab {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.interrupt();
        }
    }
}
struct Window {
    client: ProbeClient,
    frame: ClientTerminalFrameData,
    notices: Vec<TerminalControlChangedData>,
}
impl Window {
    fn open(socket: &Path, caller: Option<&str>, id: &str, rows: u16) -> Self {
        let (mut client, version) = ProbeClient::hello(socket, caller).unwrap();
        assert_eq!(version, V1_6);
        client
            .send(&ClientRequest::SubscribeTerminalFrames {
                data: TerminalSubscribeData {
                    request_id: id.into(),
                    instance_id: ID.into(),
                    viewport: TerminalViewport { top: None, rows },
                },
            })
            .unwrap();
        let response = client
            .recv_within(Duration::from_secs(15))
            .unwrap()
            .unwrap();
        match response {
            ClientResponse::TerminalFrame { data } => Self {
                client,
                frame: data,
                notices: Vec::new(),
            },
            other => panic!("subscription failed: {other:?}"),
        }
    }

    fn control_request(&self, id: &str, operation: ClientTerminalOperation) -> ClientRequest {
        ClientRequest::TerminalControl {
            data: ClientTerminalControlData {
                request_id: id.into(),
                instance_id: ID.into(),
                view_id: self.frame.view_id.clone(),
                generation: self.frame.frame.generation.clone(),
                operation,
            },
        }
    }
    fn response(&mut self, id: &str, within: Duration) -> ClientResponse {
        self.response_matching(id, within, false)
    }
    fn response_matching(
        &mut self,
        id: &str,
        within: Duration,
        frame_reply: bool,
    ) -> ClientResponse {
        let deadline = Instant::now() + within;
        loop {
            assert!(Instant::now() < deadline, "no response for {id}");
            let response = self
                .client
                .recv_within(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
                .expect("connection closed");
            match response {
                ClientResponse::TerminalFrame { data } => {
                    let selected = frame_reply && data.request_id == id;
                    self.frame = data.clone();
                    if selected {
                        return ClientResponse::TerminalFrame { data };
                    }
                }
                ClientResponse::TerminalControlChanged { data } => self.notices.push(data),
                ClientResponse::TerminalControlAck { ref data } if data.request_id == id => {
                    return response;
                }
                ClientResponse::Error { ref data } if data.request_id.as_deref() == Some(id) => {
                    return response;
                }
                ClientResponse::Fleet { ref data } if data.request_id == id => return response,
                ClientResponse::Error { ref data } if data.request_id.is_none() => return response,
                other => panic!("unexpected response: {other:?}"),
            }
        }
    }
    fn control(&mut self, id: &str, operation: ClientTerminalOperation) -> ClientResponse {
        self.client
            .send(&self.control_request(id, operation))
            .unwrap();
        self.response(id, Duration::from_secs(15))
    }
    fn acquire(&mut self, id: &str, rows: u16, columns: u16) -> String {
        let ClientResponse::TerminalControlAck { data } = self.control(
            id,
            ClientTerminalOperation::Acquire {
                size: TerminalSize { rows, columns },
            },
        ) else {
            panic!("grant expected")
        };
        let TerminalControlState::Controlled { attach_id } = data.control else {
            panic!("controlled grant expected")
        };
        let frame = data.frame.expect("grant needs a complete frame");
        assert_eq!(frame.size, TerminalSize { rows, columns });
        assert_eq!(frame.cells.len(), usize::from(rows));
        assert_eq!(frame.cells[0].len(), usize::from(columns));
        self.frame.request_id = id.into();
        self.frame.frame = frame;
        attach_id
    }
    fn viewport(&mut self, id: &str, top: Option<u64>, rows: u16) -> ClientTerminalFrameData {
        self.client
            .send(&ClientRequest::SetTerminalViewport {
                data: TerminalViewportData {
                    request_id: id.into(),
                    instance_id: ID.into(),
                    view_id: self.frame.view_id.clone(),
                    generation: self.frame.frame.generation.clone(),
                    viewport: TerminalViewport { top, rows },
                },
            })
            .unwrap();
        let ClientResponse::TerminalFrame { data } =
            self.response_matching(id, Duration::from_secs(15), true)
        else {
            panic!("frame expected")
        };
        data
    }
    fn input(&mut self, id: &str, attach: &str, bytes: &[u8]) -> ClientResponse {
        self.control(
            id,
            ClientTerminalOperation::Input {
                attach_id: attach.into(),
                bytes_base64: STANDARD.encode(bytes),
            },
        )
    }
    fn until_text(&mut self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !text(&self.frame.frame).contains(marker) {
            assert!(
                Instant::now() < deadline,
                "missing {marker}: {}",
                text(&self.frame.frame)
            );
            match self
                .client
                .recv_within(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
                .unwrap()
            {
                ClientResponse::TerminalFrame { data } => self.frame = data,
                ClientResponse::TerminalControlChanged { data } => self.notices.push(data),
                other => panic!("expected a dirty frame, got {other:?}"),
            }
        }
    }
}
fn text(frame: &TerminalFrame) -> String {
    frame
        .cells
        .iter()
        .map(|row| row.iter().map(|c| c.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn refused(response: ClientResponse, code: &str) {
    let ClientResponse::Error { data } = response else {
        panic!("refusal expected: {response:?}")
    };
    assert_eq!(data.code, code, "{}", data.message);
}
fn accepted(response: ClientResponse) {
    assert!(
        matches!(response, ClientResponse::TerminalControlAck { .. }),
        "{response:?}"
    );
}
fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "condition timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn latest_window_controls_real_size_and_eof_preserves_size_without_restoring_old_owner() {
    let lab = Lab::start(SHELL);
    let mut a = lab.window("a", 24);
    a.until_text("READY");
    let mut b = lab.window("b", 12);
    let a_token = a.acquire("a-i", 24, 80);
    let b_token = b.acquire("b-i", 12, 40);
    refused(a.input("old-a", &a_token, b"DENIED-A\n"), "control_lost");
    assert!(a.notices.iter().any(
        |n| n.control == TerminalControlState::ReadOnly && n.reason.contains("another window")
    ));
    let mut legacy = ProbeClient::hello(&lab.socket(), None).unwrap().0;
    legacy
        .send(&ClientRequest::TerminalInput {
            data: TerminalInputData {
                instance_id: ID.into(),
                bytes_base64: STANDARD.encode(b"DENIED-LEGACY\n"),
            },
        })
        .unwrap();
    refused(legacy.recv().unwrap().unwrap(), "control_required");
    accepted(b.input("b-input", &b_token, b"B-ONLY\n"));
    wait_for(|| lab.delivered().contains("B-ONLY"));
    assert!(!lab.delivered().contains("DENIED"));
    // A foreign connection cannot borrow either an actual view or a token.
    let mut stranger = lab.window("stranger", 10);
    let forged = b.control_request(
        "forged-view",
        ClientTerminalOperation::Acquire {
            size: TerminalSize {
                rows: 10,
                columns: 20,
            },
        },
    );
    stranger.client.send(&forged).unwrap();
    refused(
        stranger.response("forged-view", Duration::from_secs(5)),
        "stale_terminal",
    );
    drop(b);
    // A viewport reply runs after the actor observes EOF and releases B.
    let after_eof = a.viewport("after-eof", None, 24);
    assert_eq!(
        after_eof.frame.size,
        TerminalSize {
            rows: 12,
            columns: 40
        }
    );
    assert_eq!(after_eof.frame.cells.len(), 12);
    refused(
        a.input("still-old-a", &a_token, b"DENIED-RESTORE\n"),
        "control_lost",
    );
    legacy
        .send(&ClientRequest::TerminalInput {
            data: TerminalInputData {
                instance_id: ID.into(),
                bytes_base64: STANDARD.encode(b"LEGACY-AFTER-EOF\n"),
            },
        })
        .unwrap();
    wait_for(|| lab.delivered().contains("LEGACY-AFTER-EOF"));
    let new_token = a.acquire("a-retake", 20, 60);
    assert_ne!(new_token, a_token);
    refused(
        a.input("old-token", &a_token, b"DENIED-TOKEN\n"),
        "control_lost",
    );
    accepted(a.control(
        "a-release",
        ClientTerminalOperation::Release {
            attach_id: new_token,
        },
    ));
    assert_eq!(
        stranger.viewport("release-keeps-size", None, 10).frame.size,
        TerminalSize {
            rows: 20,
            columns: 60
        }
    );
    assert!(!lab.delivered().contains("DENIED"));
}

#[test]
fn caller_generation_and_size_refusals_do_not_change_the_owner_or_pty() {
    let lab = Lab::start(SHELL);
    let mut owner = lab.window("owner", 12);
    let attach = owner.acquire("i", 12, 40);
    for (id, rows, columns) in [("zero", 0, 40), ("huge", 1000, 1000)] {
        refused(
            owner.control(
                id,
                ClientTerminalOperation::Acquire {
                    size: TerminalSize { rows, columns },
                },
            ),
            "invalid_size",
        );
    }
    let mut bad_generation = owner.control_request(
        "wrong-generation",
        ClientTerminalOperation::Resize {
            attach_id: attach.clone(),
            size: TerminalSize {
                rows: 5,
                columns: 10,
            },
        },
    );
    if let ClientRequest::TerminalControl { data } = &mut bad_generation {
        data.generation = "old-process".into();
    }
    owner.client.send(&bad_generation).unwrap();
    refused(
        owner.response("wrong-generation", Duration::from_secs(5)),
        "stale_terminal",
    );
    let (mut agent, _) = ProbeClient::hello(&lab.socket(), Some("unknown-agent")).unwrap();
    agent.send(&bad_generation).unwrap();
    refused(agent.recv().unwrap().unwrap(), "forbidden");
    refused(
        owner.control(
            "bad-base64",
            ClientTerminalOperation::Input {
                attach_id: attach.clone(),
                bytes_base64: "!!!!".into(),
            },
        ),
        "bad_request",
    );
    accepted(owner.input("valid", &attach, b"UNCHANGED-OWNER\n"));
    wait_for(|| lab.delivered().contains("UNCHANGED-OWNER"));
    assert_eq!(
        owner.viewport("actual-size", None, 12).frame.size,
        TerminalSize {
            rows: 12,
            columns: 40
        }
    );
}

#[test]
fn blocked_native_input_keeps_socket_responsive_and_new_grant_waits_even_after_writer_eof() {
    let lab = Lab::start(
        "stty raw -echo; printf READY; IFS= read -r -n 1 byte; printf entered > write-started; exec sleep 600",
    );
    let mut a = lab.window("a", 10);
    a.until_text("READY");
    let mut b = lab.window("b", 10);
    let token = a.acquire("a-i", 10, 40);
    // Production ClientSource reads on a separate thread. Drive this native
    // probe the same way: otherwise both peers can block writing a pending
    // frame and a large request, which tests slow-client eviction instead.
    let mut writer = a.client.writer_clone().unwrap();
    let input = a.control_request(
        "blocked-input",
        ClientTerminalOperation::Input {
            attach_id: token,
            bytes_base64: STANDARD.encode(vec![b'x'; 256 * 1024]),
        },
    );
    let at = Instant::now();
    let sending = std::thread::spawn(move || {
        use std::io::Write;
        for request in [
            input,
            ClientRequest::GetFleet {
                data: RequestIdData {
                    request_id: "responsive".into(),
                },
            },
        ] {
            let mut line = serde_json::to_vec(&request).unwrap();
            line.push(b'\n');
            writer.write_all(&line).unwrap();
        }
    });
    assert!(matches!(
        a.response("responsive", Duration::from_secs(1)),
        ClientResponse::Fleet { .. }
    ));
    sending.join().unwrap();
    // GetFleet proves reader responsiveness, not that the queued operation
    // has reached the native writer. Require the actual PTY consumer to read
    // the first byte before closing the old scope; otherwise EOF can legally
    // discard work that never started and the next grant need not wait.
    wait_for(|| {
        lab.home
            .join("workspace")
            .join(ID)
            .join("write-started")
            .exists()
    });
    b.client
        .send(&b.control_request(
            "b-i",
            ClientTerminalOperation::Acquire {
                size: TerminalSize {
                    rows: 8,
                    columns: 30,
                },
            },
        ))
        .unwrap();
    drop(a);
    let ClientResponse::TerminalControlAck { data } = b.response("b-i", Duration::from_secs(10))
    else {
        panic!("new grant expected")
    };
    assert!(
        at.elapsed() >= Duration::from_secs(3),
        "grant raced ahead of the native blocked write"
    );
    assert_eq!(
        data.frame.unwrap().size,
        TerminalSize {
            rows: 8,
            columns: 30
        }
    );
    assert!(matches!(
        data.control,
        TerminalControlState::Controlled { .. }
    ));
}

#[test]
fn restart_invalidates_client_views_but_keeps_holder_generation_and_size() {
    let mut lab = Lab::start(SHELL);
    let mut a = lab.window("a", 12);
    let token = a.acquire("i", 12, 40);
    let old_request = a.control_request(
        "old-connection",
        ClientTerminalOperation::Input {
            attach_id: token,
            bytes_base64: STANDARD.encode(b"DENIED-REPLAY\n"),
        },
    );
    let generation = a.frame.frame.generation.clone();
    lab.restart();
    let mut b = lab.window("b", 12);
    assert_eq!(b.frame.frame.generation, generation);
    assert_eq!(
        b.frame.frame.size,
        TerminalSize {
            rows: 12,
            columns: 40
        }
    );
    b.client.send(&old_request).unwrap();
    refused(
        b.response("old-connection", Duration::from_secs(5)),
        "stale_terminal",
    );
    let attach = b.acquire("new-i", 12, 40);
    accepted(b.input("new-input", &attach, b"AFTER-RESTART\n"));
    wait_for(|| lab.delivered().contains("AFTER-RESTART"));
    assert!(!lab.delivered().contains("DENIED"));
}

#[test]
fn independent_history_viewports_and_the_last_dirty_frame_survive_output_bursts() {
    let lab = Lab::start(
        "printf 'READY\\r\\n'; while [ ! -e \"$AGEND_HOME/burst\" ]; do sleep 0.01; done; for n in $(seq 1 120); do printf 'row-%03d\\r\\n' \"$n\"; done; printf 'FINAL-DIRTY'; exec sleep 600",
    );
    let mut a = lab.window("a", 50);
    a.until_text("READY");
    let mut b = lab.window("b", 50);
    fs::write(lab.home.join("burst"), b"go").unwrap();
    let at = Instant::now();
    a.until_text("FINAL-DIRTY");
    assert!(
        at.elapsed() < Duration::from_secs(3),
        "last dirty frame was lost or delayed"
    );
    b.until_text("FINAL-DIRTY");
    let history = a.viewport("history-a", Some(a.frame.frame.history_oldest), 10);
    assert!(text(&history.frame).contains("row-001"));
    assert!(!text(&history.frame).contains("FINAL-DIRTY"));
    let live = b.viewport("bottom-b", None, 10);
    // A live viewport starts at the top of the live grid; show its entire grid
    // to find the final output without rewriting A's independently pinned top.
    let live = if text(&live.frame).contains("FINAL-DIRTY") {
        live
    } else {
        b.viewport("whole-live-b", None, live.frame.size.rows)
    };
    assert!(text(&live.frame).contains("FINAL-DIRTY"));
    assert_eq!(history.frame.viewport_top, history.frame.history_oldest);
    assert_eq!(
        a.viewport("pinned-again", Some(history.frame.viewport_top), 10)
            .frame
            .viewport_top,
        history.frame.viewport_top
    );
}

#[test]
fn the_dedicated_client_reader_and_sender_use_the_real_daemon_path() {
    use agend_client::{Client, FullTerminalUpdate};
    let lab = Lab::start(SHELL);
    let mut reader = Client::connect_once(&lab.socket(), None).unwrap();
    assert_eq!(reader.selected(), V1_6);
    let mut sender = reader.sender().unwrap();
    sender
        .subscribe_terminal_frames(TerminalSubscribeData {
            request_id: "client-subscribe".into(),
            instance_id: ID.into(),
            viewport: TerminalViewport {
                top: None,
                rows: 12,
            },
        })
        .unwrap();
    let FullTerminalUpdate::Frame(frame) = reader.next_full_terminal().unwrap() else {
        panic!("initial frame expected")
    };
    let make = |id: &str, operation| ClientTerminalControlData {
        request_id: id.into(),
        instance_id: ID.into(),
        view_id: frame.view_id.clone(),
        generation: frame.frame.generation.clone(),
        operation,
    };
    sender
        .terminal_control(make(
            "client-i",
            ClientTerminalOperation::Acquire {
                size: TerminalSize {
                    rows: 12,
                    columns: 40,
                },
            },
        ))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let attach = loop {
        assert!(Instant::now() < deadline, "grant never arrived");
        match reader.next_full_terminal().unwrap() {
            FullTerminalUpdate::ControlAck(ack) if ack.request_id == "client-i" => {
                assert_eq!(
                    ack.frame.unwrap().size,
                    TerminalSize {
                        rows: 12,
                        columns: 40
                    }
                );
                let TerminalControlState::Controlled { attach_id } = ack.control else {
                    panic!("controlled ack expected")
                };
                break attach_id;
            }
            FullTerminalUpdate::Frame(_) => {}
            other => panic!("unexpected grant response: {other:?}"),
        }
    };
    sender
        .terminal_control(make(
            "client-input",
            ClientTerminalOperation::Input {
                attach_id: attach,
                bytes_base64: STANDARD.encode(b"DEDICATED-CLIENT\n"),
            },
        ))
        .unwrap();
    loop {
        assert!(Instant::now() < deadline, "input ack never arrived");
        match reader.next_full_terminal().unwrap() {
            FullTerminalUpdate::ControlAck(ack) if ack.request_id == "client-input" => break,
            FullTerminalUpdate::Frame(_) => {}
            other => panic!("unexpected input response: {other:?}"),
        }
    }
    wait_for(|| lab.delivered().contains("DEDICATED-CLIENT"));
    sender.close();
    assert!(reader.next_full_terminal().is_err());
}

#[test]
fn twenty_view_open_grant_and_close_cycles_do_not_restore_previous_tokens() {
    let lab = Lab::start(SHELL);
    let mut observer = lab.window("observer", 10);
    let mut previous = None;
    for n in 0..20 {
        let mut window = lab.window(&format!("open-{n}"), 10);
        let token = window.acquire(&format!("grant-{n}"), 10, 40);
        if let Some(ref old) = previous {
            assert_ne!(old, &token);
        }
        let old_request = window.control_request(
            &format!("closed-{n}"),
            ClientTerminalOperation::Input {
                attach_id: token.clone(),
                bytes_base64: STANDARD.encode(b"DENIED-CLOSED\n"),
            },
        );
        drop(window);
        observer.viewport(&format!("after-close-{n}"), None, 10);
        observer.client.send(&old_request).unwrap();
        refused(
            observer.response(&format!("closed-{n}"), Duration::from_secs(5)),
            "stale_terminal",
        );
        previous = Some(token);
    }
    assert!(!lab.delivered().contains("DENIED"));
    assert_eq!(
        observer.viewport("still-live", None, 10).frame.size,
        TerminalSize {
            rows: 10,
            columns: 40
        }
    );
}

#[test]
fn stopping_the_entry_service_invalidates_a_completed_owner_on_the_native_holder() {
    use agend_core::traits::HolderLaunch;
    use agend_daemon::fleet::Fleet;
    use agend_daemon::runtime::HolderRuntime;
    use agend_daemon::terminal_hub::{ReplyScope, TerminalHub};
    use std::sync::{Arc, atomic::AtomicBool};
    use tokio::sync::mpsc;
    let native = lab::Lab::with_prefix(Path::new(BIN), "g11stop");
    let home = native.home(1);
    let rt = HolderRuntime::new(&home, Path::new(BIN), Vec::new(), Arc::new(|_| {}));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            rt.start(&HolderLaunch {
                instance_id: ID.into(),
                backend: Backend::Claude,
                executable: "/bin/bash".into(),
                args: vec!["-c".into(), SHELL.into()],
                working_directory: home.display().to_string(),
            })
            .await
            .unwrap();
            let connection = rt.terminal_connection(ID).unwrap();
            let fleet = Arc::new(Fleet::new(1));
            fleet.set_instance(
                InstanceView {
                    instance_id: ID.into(),
                    team_id: "test".into(),
                    backend: "claude".into(),
                    state: AgentState::Unknown,
                    working_directory: None,
                },
                "native test".into(),
            );
            let hub = TerminalHub::new(rt.clone(), fleet);
            let (replies, mut receiver) = mpsc::channel(8);
            let scope = ReplyScope {
                client: 1,
                alive: Arc::new(AtomicBool::new(true)),
                replies,
            };
            let view = hub
                .subscribe(
                    scope.clone(),
                    TerminalSubscribeData {
                        request_id: "sub".into(),
                        instance_id: ID.into(),
                        viewport: TerminalViewport { top: None, rows: 5 },
                    },
                )
                .unwrap();
            let ClientResponse::TerminalFrame { data } =
                tokio::time::timeout(Duration::from_secs(15), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap()
            else {
                panic!("frame expected")
            };
            hub.control(
                scope,
                &view,
                ClientTerminalControlData {
                    request_id: "i".into(),
                    instance_id: ID.into(),
                    view_id: data.view_id,
                    generation: data.frame.generation.clone(),
                    operation: ClientTerminalOperation::Acquire {
                        size: TerminalSize {
                            rows: 5,
                            columns: 20,
                        },
                    },
                },
            )
            .unwrap();
            let ClientResponse::TerminalControlAck { data: ack } =
                tokio::time::timeout(Duration::from_secs(15), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap()
            else {
                panic!("grant expected")
            };
            let TerminalControlState::Controlled { attach_id } = ack.control else {
                panic!("owner expected")
            };
            hub.stop();
            tokio::time::timeout(Duration::from_secs(1), async {
                while connection.is_current() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("stopped actor left its completed grant active");
            let fresh = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    if let Ok(connection) = rt.terminal_connection(ID) {
                        break connection;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let frame = fresh
                .frame(TerminalViewport { top: None, rows: 5 })
                .await
                .unwrap();
            assert_eq!(frame.generation, data.frame.generation);
            assert_eq!(
                frame.size,
                TerminalSize {
                    rows: 5,
                    columns: 20
                }
            );
            let error = fresh
                .control(
                    frame.generation,
                    TerminalControlOperation::Input {
                        attach_id,
                        bytes_base64: STANDARD.encode(b"DENIED-STOP\n"),
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, "control_lost");
        });
    rt.detach(ID);
}
