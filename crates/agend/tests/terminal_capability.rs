//! An old 1.3 client cannot use the new API, against real or fake servers.
//! New requests must fail explicitly without changing legacy subscriptions.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/client_process.rs"]
mod clp;
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::protocol::client::*;
use agend_core::protocol::terminal::*;
use agend_testkit::contract::client::ClientProtocolFixture;
use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};
use std::path::Path;

fn unavailable(socket: &Path) {
    for caller in [None, Some("unknown-agent")] {
        let mut client = ProbeClient::connect(socket).unwrap();
        let ClientResponse::Hello { data } = client
            .request(&ClientRequest::Hello {
                data: ClientHello {
                    supported: vec![V1_3],
                    caller: caller.map(str::to_owned),
                },
            })
            .unwrap()
        else {
            panic!("hello expected")
        };
        let selected = data.selected;
        assert_eq!(selected, V1_3);
        for (request, control) in [
            (
                ClientRequest::SubscribeTerminalFrames {
                    data: TerminalSubscribeData {
                        request_id: "subscribe-1".into(),
                        instance_id: "missing".into(),
                        viewport: TerminalViewport { top: None, rows: 0 },
                    },
                },
                false,
            ),
            (
                ClientRequest::SetTerminalViewport {
                    data: TerminalViewportData {
                        fit_size: None,
                        request_id: "viewport-2".into(),
                        instance_id: "missing".into(),
                        view_id: "stale-view".into(),
                        generation: "stale-holder".into(),
                        viewport: TerminalViewport {
                            top: Some(100),
                            rows: 0,
                        },
                    },
                },
                false,
            ),
            (
                ClientRequest::TerminalControl {
                    data: ClientTerminalControlData {
                        request_id: "control-3".into(),
                        instance_id: "missing".into(),
                        view_id: "stale-view".into(),
                        generation: "stale-holder".into(),
                        operation: ClientTerminalOperation::Acquire {
                            size: TerminalSize {
                                rows: 0,
                                columns: 0,
                            },
                        },
                    },
                },
                true,
            ),
        ] {
            let expected_id = match &request {
                ClientRequest::SubscribeTerminalFrames { data } => &data.request_id,
                ClientRequest::SetTerminalViewport { data } => &data.request_id,
                ClientRequest::TerminalControl { data } => &data.request_id,
                _ => unreachable!(),
            };
            let ClientResponse::Error { data } = client.request(&request).unwrap() else {
                panic!("explicit refusal expected")
            };
            assert_eq!(data.request_id.as_deref(), Some(expected_id.as_str()));
            assert_eq!(
                data.code,
                if control && caller.is_some() {
                    error_code::FORBIDDEN
                } else {
                    error_code::NOT_SUPPORTED
                }
            );
            if data.code == error_code::NOT_SUPPORTED {
                assert!(data.message.contains("client protocol 1.4"));
            }
            assert!(
                matches!(
                    client
                        .request(&ClientRequest::GetFleet {
                            data: RequestIdData {
                                request_id: "still-connected".into(),
                            }
                        })
                        .unwrap(),
                    ClientResponse::Fleet { .. }
                ),
                "refusal must preserve ordinary 1.3 use"
            );
        }
    }
}

#[test]
fn fake_and_real_servers_refuse_full_requests_on_a_negotiated_1_3_connection() {
    let fake = FakeDaemon::start().unwrap();
    unavailable(fake.socket_path());
    let real = clp::RealDaemon::fixture(Path::new(env!("CARGO_BIN_EXE_agend")));
    unavailable(&real.socket());
}
