//! Client 1.4 contracts, shared verbatim by fake and native daemon fixtures.
//! Frames come from each fixture's real parser; assertions inspect both wire
//! replies and the input producer's independent record of delivered bytes.
use super::{Case, CaseResult, Report, run_suite};
use crate::fake_daemon::ProbeClient;
use agend_core::protocol::client::*;
use agend_core::protocol::terminal::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const WITHIN: Duration = Duration::from_secs(10);
pub trait FullTerminalFixture {
    fn socket(&self) -> PathBuf;
    fn instance(&self) -> String;
    /// Independent record from the input consumer, not from a grant/ack.
    fn received(&self) -> String;
    /// Makes the actual parser receive numbered lines, ending in FINAL-DIRTY.
    fn output(&mut self);
}
pub type Fixture = Box<dyn FullTerminalFixture>;
pub fn cases() -> Vec<Case<Fixture>> {
    vec![
        Case {
            rule: "CLP-23",
            name: "full_frame_preserves_correlation_and_size",
            check: frames,
        },
        Case {
            rule: "CLP-24",
            name: "last_acquire_wins_and_old_operations_do_not_write",
            check: takeover,
        },
        Case {
            rule: "CLP-25",
            name: "refusals_do_not_resize_or_write",
            check: refusals,
        },
        Case {
            rule: "CLP-26",
            name: "eof_releases_without_restoring_an_old_grant",
            check: eof,
        },
        Case {
            rule: "CLP-27",
            name: "viewports_are_independent_and_last_dirty_arrives",
            check: history,
        },
        Case {
            rule: "CLP-28",
            name: "replacement_and_old_capability_invalidate_views",
            check: replacement,
        },
    ]
}
pub fn run(implementation: &str, make: impl FnMut() -> Fixture) -> Report {
    run_suite("ClientProtocol", implementation, &cases(), make)
}
pub fn run_rules(implementation: &str, rules: &[&str], make: impl FnMut() -> Fixture) -> Report {
    let cases = cases()
        .into_iter()
        .filter(|c| rules.contains(&c.rule))
        .collect::<Vec<_>>();
    run_suite("ClientProtocol", implementation, &cases, make)
}

pub struct Window {
    pub client: ProbeClient,
    pub frame: ClientTerminalFrameData,
    pub notices: Vec<TerminalControlChangedData>,
    pub stream_errors: Vec<ErrorData>,
}
impl Window {
    pub fn open(fx: &dyn FullTerminalFixture, caller: Option<&str>) -> Self {
        let (mut client, version) = ProbeClient::hello(&fx.socket(), caller).unwrap();
        assert_eq!(version, V1_4);
        client
            .send(&ClientRequest::SubscribeTerminalFrames {
                data: TerminalSubscribeData {
                    request_id: "subscribe".into(),
                    instance_id: fx.instance(),
                    viewport: TerminalViewport {
                        top: None,
                        rows: 50,
                    },
                },
            })
            .unwrap();
        let ClientResponse::TerminalFrame { data } = client.recv_within(WITHIN).unwrap().unwrap()
        else {
            panic!("complete frame expected")
        };
        assert_eq!(data.request_id, "subscribe");
        assert_eq!(data.instance_id, fx.instance());
        assert!(!data.view_id.is_empty());
        assert!(!data.frame.generation.is_empty());
        Self {
            client,
            frame: data,
            notices: Vec::new(),
            stream_errors: Vec::new(),
        }
    }
    pub fn response(&mut self, id: &str) -> ClientResponse {
        self.response_matching(id, false)
    }
    fn response_matching(&mut self, id: &str, frame_reply: bool) -> ClientResponse {
        let deadline = Instant::now() + WITHIN;
        loop {
            let response = self
                .client
                .recv_within(deadline.saturating_duration_since(Instant::now()))
                .unwrap()
                .expect("connection closed");
            match response {
                ClientResponse::TerminalFrame { data } => {
                    let matches = frame_reply && data.request_id == id;
                    self.frame = data.clone();
                    if matches {
                        return ClientResponse::TerminalFrame { data };
                    }
                }
                ClientResponse::TerminalControlChanged { data } => self.notices.push(data),
                ClientResponse::TerminalControlAck { ref data } if data.request_id == id => {
                    return response;
                }
                ClientResponse::Error { ref data }
                    if data.request_id.as_deref() == Some(id) || data.request_id.is_none() =>
                {
                    return response;
                }
                ClientResponse::Error { data }
                    if data.request_id.as_deref() == Some(self.frame.request_id.as_str()) =>
                {
                    self.stream_errors.push(data)
                }
                ClientResponse::Fleet { ref data } if data.request_id == id => return response,
                other => panic!("uncorrelated response for {id}: {other:?}"),
            }
        }
    }
    pub fn request(&self, id: &str, operation: ClientTerminalOperation) -> ClientRequest {
        ClientRequest::TerminalControl {
            data: ClientTerminalControlData {
                request_id: id.into(),
                instance_id: self.frame.instance_id.clone(),
                view_id: self.frame.view_id.clone(),
                generation: self.frame.frame.generation.clone(),
                operation,
            },
        }
    }
    pub fn control(&mut self, id: &str, operation: ClientTerminalOperation) -> ClientResponse {
        self.client.send(&self.request(id, operation)).unwrap();
        self.response(id)
    }
    pub fn acquire(&mut self, id: &str, size: TerminalSize) -> String {
        let ClientResponse::TerminalControlAck { data } =
            self.control(id, ClientTerminalOperation::Acquire { size })
        else {
            panic!("grant expected")
        };
        assert_eq!(data.view_id, self.frame.view_id);
        assert_eq!(data.generation, self.frame.frame.generation);
        let TerminalControlState::Controlled { attach_id } = data.control else {
            panic!("controlled grant expected")
        };
        assert!(!attach_id.is_empty());
        let frame = data.frame.expect("complete resized frame before grant");
        assert_eq!(frame.size, size);
        assert_eq!(frame.cells.len(), usize::from(size.rows));
        assert!(
            frame
                .cells
                .iter()
                .all(|r| r.len() == usize::from(size.columns))
        );
        self.frame.request_id = id.into();
        self.frame.frame = frame;
        attach_id
    }
    pub fn viewport(&mut self, id: &str, top: Option<u64>, rows: u16) -> TerminalFrame {
        self.client
            .send(&ClientRequest::SetTerminalViewport {
                data: TerminalViewportData {
                    request_id: id.into(),
                    instance_id: self.frame.instance_id.clone(),
                    view_id: self.frame.view_id.clone(),
                    generation: self.frame.frame.generation.clone(),
                    viewport: TerminalViewport { top, rows },
                },
            })
            .unwrap();
        let ClientResponse::TerminalFrame { data } = self.response_matching(id, true) else {
            panic!("selected frame expected")
        };
        data.frame
    }
    pub fn input(&mut self, id: &str, attach: &str, bytes: &[u8]) -> ClientResponse {
        self.control(
            id,
            ClientTerminalOperation::Input {
                attach_id: attach.into(),
                bytes_base64: STANDARD.encode(bytes),
            },
        )
    }
    fn fleet(&mut self, id: &str) {
        self.client
            .send(&ClientRequest::GetFleet {
                data: RequestIdData {
                    request_id: id.into(),
                },
            })
            .unwrap();
        assert!(matches!(self.response(id), ClientResponse::Fleet { .. }));
    }
}
fn size(rows: u16, columns: u16) -> TerminalSize {
    TerminalSize { rows, columns }
}
fn denied(response: ClientResponse, code: &str) {
    assert!(
        matches!(&response, ClientResponse::Error { data } if data.code == code),
        "expected {code}: {response:?}"
    );
}
fn delivered(fx: &dyn FullTerminalFixture, expected: &str) {
    let deadline = Instant::now() + WITHIN;
    while !fx.received().contains(expected) {
        assert!(
            Instant::now() < deadline,
            "input consumer did not receive {expected:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn text(frame: &TerminalFrame) -> String {
    frame
        .cells
        .iter()
        .map(|r| r.iter().map(|c| c.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn frames(fx: Fixture) -> CaseResult {
    let mut a = Window::open(&*fx, None);
    let first = a.frame.frame.clone();
    let attach = a.acquire("first", size(7, 31));
    assert!(a.frame.frame.revision >= first.revision);
    assert_eq!(a.frame.frame.generation, first.generation);
    let fresh = a.acquire("second", size(8, 29));
    assert_ne!(attach, fresh, "each acquire needs a fresh token");
    let selected = a.viewport("select", None, 3);
    assert_eq!(selected.cells.len(), 3);
    assert_eq!(selected.size, size(8, 29));
    Ok(())
}
fn takeover(fx: Fixture) -> CaseResult {
    let mut a = Window::open(&*fx, None);
    let mut b = Window::open(&*fx, None);
    let aa = a.acquire("a", size(9, 34));
    let bb = b.acquire("b", size(6, 28));
    denied(
        a.input("old-input", &aa, b"DENIED-OLD\n"),
        error_code::CONTROL_LOST,
    );
    denied(
        a.control(
            "old-resize",
            ClientTerminalOperation::Resize {
                attach_id: aa,
                size: size(2, 2),
            },
        ),
        error_code::CONTROL_LOST,
    );
    assert!(
        a.notices
            .iter()
            .any(|n| n.view_id == a.frame.view_id && n.control == TerminalControlState::ReadOnly),
        "old window needs read-only notice"
    );
    assert_eq!(a.viewport("dimensions", None, 5).size, size(6, 28));
    assert!(matches!(
        b.input("current", &bb, b"ALLOWED-CURRENT\n"),
        ClientResponse::TerminalControlAck { .. }
    ));
    delivered(&*fx, "ALLOWED-CURRENT");
    assert!(!fx.received().contains("DENIED-OLD"));
    let next = a.acquire("retake", size(4, 21));
    assert_ne!(next, bb);
    denied(
        b.input("now-old", &bb, b"DENIED-SECOND\n"),
        error_code::CONTROL_LOST,
    );
    Ok(())
}
fn refusals(fx: Fixture) -> CaseResult {
    let mut a = Window::open(&*fx, None);
    let mut agent = Window::open(&*fx, Some("agent"));
    let attach = a.acquire("owner", size(8, 32));
    denied(
        agent.control(
            "forbidden",
            ClientTerminalOperation::Acquire { size: size(0, 0) },
        ),
        error_code::FORBIDDEN,
    );
    let mut stale = a.request(
        "stale",
        ClientTerminalOperation::Input {
            attach_id: attach.clone(),
            bytes_base64: STANDARD.encode(b"DENIED-STALE\n"),
        },
    );
    if let ClientRequest::TerminalControl { data } = &mut stale {
        data.generation = "old-generation".into();
    }
    a.client.send(&stale).unwrap();
    denied(a.response("stale"), error_code::STALE_TERMINAL);
    denied(
        a.control(
            "size",
            ClientTerminalOperation::Resize {
                attach_id: attach.clone(),
                size: size(0, 12),
            },
        ),
        error_code::INVALID_SIZE,
    );
    denied(
        a.input(
            "oversized",
            &attach,
            &vec![b'x'; agend_core::protocol::holder::MAX_REQUEST_LINE],
        ),
        error_code::INVALID_REQUEST,
    );
    let mut foreign = agent.request(
        "foreign",
        ClientTerminalOperation::Acquire { size: size(8, 30) },
    );
    if let ClientRequest::TerminalControl { data } = &mut foreign {
        data.view_id = a.frame.view_id.clone();
    }
    // A second operator cannot use another connection's view, either.
    let mut other = Window::open(&*fx, None);
    other.client.send(&foreign).unwrap();
    denied(other.response("foreign"), error_code::STALE_TERMINAL);
    a.client
        .send(&ClientRequest::TerminalInput {
            data: TerminalInputData {
                instance_id: fx.instance(),
                bytes_base64: STANDARD.encode(b"DENIED-LEGACY\n"),
            },
        })
        .unwrap();
    denied(a.response("legacy"), error_code::CONTROL_REQUIRED);
    assert_eq!(a.viewport("size-after", None, 8).size, size(8, 32));
    assert!(matches!(
        a.input("valid", &attach, b"ALLOWED-AFTER\n"),
        ClientResponse::TerminalControlAck { .. }
    ));
    delivered(&*fx, "ALLOWED-AFTER");
    assert!(!fx.received().contains("DENIED-"));
    Ok(())
}
fn eof(fx: Fixture) -> CaseResult {
    let mut a = Window::open(&*fx, None);
    let aa = a.acquire("a", size(9, 33));
    let mut b = Window::open(&*fx, None);
    let _ = b.acquire("b", size(6, 23));
    drop(b);
    denied(
        a.input("old", &aa, b"DENIED-RESTORED\n"),
        error_code::CONTROL_LOST,
    );
    let deadline = Instant::now() + WITHIN;
    loop {
        a.client
            .send(&ClientRequest::TerminalInput {
                data: TerminalInputData {
                    instance_id: fx.instance(),
                    bytes_base64: STANDARD.encode(b"AFTER-EOF\n"),
                },
            })
            .unwrap();
        a.client
            .send(&ClientRequest::GetFleet {
                data: RequestIdData {
                    request_id: "eof-check".into(),
                },
            })
            .unwrap();
        if matches!(a.response("eof-check"), ClientResponse::Fleet { .. }) {
            break;
        }
        a.fleet("drain");
        assert!(Instant::now() < deadline, "EOF did not release control");
        std::thread::sleep(Duration::from_millis(10));
    }
    delivered(&*fx, "AFTER-EOF");
    assert_eq!(a.viewport("size-retained", None, 5).size, size(6, 23));
    assert!(!fx.received().contains("DENIED-RESTORED"));
    Ok(())
}
fn history(mut fx: Fixture) -> CaseResult {
    let mut a = Window::open(&*fx, None);
    let mut b = Window::open(&*fx, None);
    fx.output();
    let deadline = Instant::now() + WITHIN;
    while !text(&b.frame.frame).contains("FINAL-DIRTY") {
        let ClientResponse::TerminalFrame { data } = b
            .client
            .recv_within(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
            .unwrap()
        else {
            panic!("last dirty frame expected")
        };
        assert_eq!(data.request_id, "subscribe");
        assert!(data.frame.revision >= b.frame.frame.revision);
        b.frame = data;
    }
    assert!(b.frame.frame.live_top > 0);
    let top = b.frame.frame.history_oldest + 2;
    let pinned = a.viewport("pinned", Some(top), 4);
    assert_eq!(pinned.viewport_top, top);
    let previous_top = b.frame.frame.live_top;
    fx.output();
    let deadline = Instant::now() + WITHIN;
    while b.viewport("after-second-output", None, 50).live_top <= previous_top {
        assert!(
            Instant::now() < deadline,
            "second output never reached the parser"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let later = a.viewport("pinned-again", Some(top), 4);
    assert_eq!(text(&later), text(&pinned));
    assert_eq!(later.viewport_top, top);
    let bottom = b.viewport("bottom", None, 50);
    assert_eq!(bottom.viewport_top, bottom.live_top);
    assert_ne!(a.frame.frame.viewport_top, b.frame.frame.viewport_top);
    Ok(())
}
fn replacement(fx: Fixture) -> CaseResult {
    let mut a = Window::open(&*fx, None);
    let token = a.acquire("owner", size(7, 27));
    a.client
        .send(&ClientRequest::SubscribeTerminalFrames {
            data: TerminalSubscribeData {
                request_id: "missing".into(),
                instance_id: "missing-instance".into(),
                viewport: TerminalViewport { top: None, rows: 3 },
            },
        })
        .unwrap();
    denied(a.response("missing"), error_code::NO_TERMINAL);
    denied(
        a.input("replaced", &token, b"DENIED-REPLACED\n"),
        error_code::STALE_TERMINAL,
    );
    let mut old = ProbeClient::connect(&fx.socket()).unwrap();
    let ClientResponse::Hello { data } = old
        .request(&ClientRequest::Hello {
            data: ClientHello {
                supported: vec![V1_3],
                caller: None,
            },
        })
        .unwrap()
    else {
        panic!("old hello failed")
    };
    assert_eq!(data.selected, V1_3);
    let response = old
        .request(&ClientRequest::SubscribeTerminalFrames {
            data: TerminalSubscribeData {
                request_id: "old-version".into(),
                instance_id: fx.instance(),
                viewport: TerminalViewport { top: None, rows: 3 },
            },
        })
        .unwrap();
    denied(response, error_code::NOT_SUPPORTED);
    assert!(!fx.received().contains("DENIED-REPLACED"));
    Ok(())
}
