//! Native transport consumer tests. Frames come from the real holder parser;
//! this wire peer checks transport behavior, not daemon ownership policy.
#![cfg(unix)]
use agend_client::{Client, ClientError, FullTerminalUpdate, Redo};
use agend_core::protocol::client::*;
use agend_core::protocol::terminal::*;
use agend_core::protocol::{ProtocolVersion, negotiate};
use agend_holder::screen::{ReplySink, Screen};
use agend_testkit::tempdir::TempDir;
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

fn read(reader: &mut BufReader<UnixStream>) -> Option<ClientRequest> {
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap() == 0 {
        return None;
    }
    Some(serde_json::from_str(&line).unwrap())
}
fn send(writer: &mut UnixStream, response: ClientResponse) {
    let mut line = serde_json::to_vec(&response).unwrap();
    line.push(b'\n');
    writer.write_all(&line).unwrap();
}
fn frame() -> TerminalFrame {
    let mut screen = Screen::new(3, 40, ReplySink::default());
    screen.process("\x1b[31;1mred\x1b[0m 界e\u{301}\r\nnext\x1b[?1h\x1b[?2004h".as_bytes());
    screen
        .frame(TerminalViewport { top: None, rows: 3 })
        .unwrap()
}
fn framed(frame: TerminalFrame) -> ClientResponse {
    ClientResponse::TerminalFrame {
        data: ClientTerminalFrameData {
            request_id: "selection-1".into(),
            instance_id: "i-1".into(),
            view_id: "view-1".into(),
            frame,
        },
    }
}
fn operation(operation: ClientTerminalOperation) -> ClientRequest {
    ClientRequest::TerminalControl {
        data: ClientTerminalControlData {
            request_id: "op-1".into(),
            instance_id: "i-1".into(),
            view_id: "view-1".into(),
            generation: "test-generation".into(),
            operation,
        },
    }
}
fn subscription() -> TerminalSubscribeData {
    TerminalSubscribeData {
        request_id: "selection-1".into(),
        instance_id: "i-1".into(),
        viewport: TerminalViewport { top: None, rows: 3 },
    }
}
fn peer(
    versions: &[ProtocolVersion],
    body: impl FnOnce(BufReader<UnixStream>, UnixStream) + Send + 'static,
) -> (TempDir, Client, thread::JoinHandle<()>) {
    let dir = TempDir::new("full-client").unwrap();
    let socket = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let versions = versions.to_vec();
    let worker = thread::spawn(move || {
        let (mut writer, _) = listener.accept().unwrap();
        writer
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        writer
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(writer.try_clone().unwrap());
        let ClientRequest::Hello { data } = read(&mut reader).unwrap() else {
            panic!("hello required")
        };
        let selected = negotiate("client", &versions, &data.supported).unwrap();
        send(
            &mut writer,
            ClientResponse::Hello {
                data: SelectedVersionData::new(selected),
            },
        );
        body(reader, writer);
    });
    let client = Client::connect_once(&socket, None).unwrap();
    (dir, client, worker)
}

#[test]
fn old_daemon_still_connects_but_full_api_fails_before_io() {
    let (_dir, mut client, worker) = peer(&[V1_3], |mut reader, _| {
        assert!(read(&mut reader).is_none())
    });
    assert_eq!(client.selected(), V1_3);
    assert_eq!(agend_client::version::NEEDED, V1_3);
    let mut sender = client.sender().unwrap();
    assert!(matches!(
        sender.subscribe_terminal_frames(subscription()),
        Err(ClientError::Version(_))
    ));
    assert!(matches!(
        client.next_full_terminal(),
        Err(ClientError::Version(_))
    ));
    assert!(matches!(
        client.request(
            &operation(ClientTerminalOperation::Acquire {
                size: TerminalSize {
                    rows: 3,
                    columns: 40
                }
            }),
            Redo::Safe
        ),
        Err(ClientError::Version(_))
    ));
    sender.close();
    worker.join().unwrap();
}

#[test]
fn generic_retry_api_never_sends_connection_scoped_operations() {
    let (_dir, mut client, worker) = peer(&[V1_4], |mut reader, _| {
        assert!(read(&mut reader).is_none())
    });
    let error = client
        .request(
            &operation(ClientTerminalOperation::Acquire {
                size: TerminalSize {
                    rows: 3,
                    columns: 40,
                },
            }),
            Redo::Safe,
        )
        .unwrap_err();
    assert!(error.to_string().contains("never reconnect or replay"));
    client.sender().unwrap().close();
    worker.join().unwrap();
}

#[test]
fn actual_parser_frames_control_notifications_and_refusals_keep_identity() {
    let expected = frame();
    let producer = expected.clone();
    let (_dir, mut client, worker) = peer(&[V1_4], move |mut reader, mut writer| {
        assert_eq!(
            read(&mut reader),
            Some(ClientRequest::SubscribeTerminalFrames {
                data: subscription()
            })
        );
        send(&mut writer, framed(producer.clone()));
        send(
            &mut writer,
            ClientResponse::TerminalControlAck {
                data: ClientTerminalControlAck {
                    request_id: "resize-2".into(),
                    instance_id: "i-1".into(),
                    view_id: "view-1".into(),
                    generation: producer.generation.clone(),
                    control: TerminalControlState::Controlled {
                        attach_id: "fresh-2".into(),
                    },
                    frame: Some(producer.clone()),
                },
            },
        );
        send(
            &mut writer,
            ClientResponse::TerminalControlChanged {
                data: TerminalControlChangedData {
                    instance_id: "i-1".into(),
                    view_id: "view-1".into(),
                    generation: producer.generation,
                    control: TerminalControlState::ReadOnly,
                    reason: "another window controls this terminal".into(),
                },
            },
        );
        send(
            &mut writer,
            ClientResponse::Error {
                data: ErrorData {
                    request_id: Some("stale-input-3".into()),
                    code: "stale_attach".into(),
                    message: "control changed".into(),
                },
            },
        );
        assert!(read(&mut reader).is_none());
    });
    let mut sender = client.sender().unwrap();
    sender.subscribe_terminal_frames(subscription()).unwrap();
    let FullTerminalUpdate::Frame(data) = client.next_full_terminal().unwrap() else {
        panic!("frame expected")
    };
    assert_eq!(data.frame, expected);
    assert_eq!(data.request_id, "selection-1");
    assert!(data.frame.modes.application_cursor && data.frame.modes.bracketed_paste);
    assert!(
        data.frame
            .cells
            .iter()
            .flatten()
            .any(|cell| cell.text == "界" && cell.width == 2)
    );
    assert!(
        data.frame
            .cells
            .iter()
            .flatten()
            .any(|cell| cell.text == "e\u{301}")
    );
    let FullTerminalUpdate::ControlAck(data) = client.next_full_terminal().unwrap() else {
        panic!("ack expected")
    };
    assert_eq!(data.request_id, "resize-2");
    assert_eq!(data.frame.unwrap(), expected);
    let FullTerminalUpdate::ControlChanged(data) = client.next_full_terminal().unwrap() else {
        panic!("control loss expected")
    };
    assert_eq!(data.control, TerminalControlState::ReadOnly);
    let FullTerminalUpdate::Rejected(data) = client.next_full_terminal().unwrap() else {
        panic!("refusal expected")
    };
    assert_eq!(data.request_id.as_deref(), Some("stale-input-3"));
    sender.close();
    worker.join().unwrap();
}

#[test]
fn full_frame_line_limit_includes_newline_and_failure_closes_all_handles() {
    for extra in [0, 1] {
        let expected = frame();
        let mut bytes = serde_json::to_vec(&framed(expected.clone())).unwrap();
        bytes.resize(MAX_FRAME_LINE + extra - 1, b' ');
        bytes.push(b'\n');
        let (_dir, mut client, worker) = peer(&[V1_4], move |mut reader, mut writer| {
            // Rejected peers can close during the final chunk.
            let _ = writer.write_all(&bytes);
            assert!(read(&mut reader).is_none());
        });
        let mut sender = client.sender().unwrap();
        let result = client.next_full_terminal();
        if extra == 0 {
            let FullTerminalUpdate::Frame(data) = result.unwrap() else {
                panic!("complete frame expected")
            };
            assert_eq!(data.frame, expected);
            sender.close();
        } else {
            assert!(result.unwrap_err().to_string().contains("exceeds 8 MiB"));
            assert!(sender.subscribe_terminal_frames(subscription()).is_err());
        }
        worker.join().unwrap();
    }
}

#[test]
fn partial_eof_is_rejected_and_read_failure_releases_the_connection() {
    let bytes = serde_json::to_vec(&framed(frame())).unwrap();
    let (_dir, mut client, worker) = peer(&[V1_4], move |mut reader, mut writer| {
        writer.write_all(&bytes[..bytes.len() / 2]).unwrap();
        writer.shutdown(Shutdown::Write).unwrap();
        assert!(read(&mut reader).is_none());
    });
    let error = client.next_full_terminal().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("incomplete terminal protocol line")
    );
    worker.join().unwrap();
}

#[test]
fn invalid_frame_dimensions_and_generation_mismatch_are_not_rendered() {
    for mismatch in [false, true] {
        let mut producer = frame();
        let response = if mismatch {
            ClientResponse::TerminalControlAck {
                data: ClientTerminalControlAck {
                    request_id: "op-1".into(),
                    instance_id: "i-1".into(),
                    view_id: "view-1".into(),
                    generation: "another-holder".into(),
                    control: TerminalControlState::Controlled {
                        attach_id: "fresh-2".into(),
                    },
                    frame: Some(producer),
                },
            }
        } else {
            producer.cells[0].pop();
            framed(producer)
        };
        let (_dir, mut client, worker) = peer(&[V1_4], move |mut reader, mut writer| {
            send(&mut writer, response);
            assert!(read(&mut reader).is_none());
        });
        assert!(matches!(
            client.next_full_terminal(),
            Err(ClientError::Disconnected(_))
        ));
        worker.join().unwrap();
    }
}

#[test]
fn paste_and_dimensions_are_checked_whole_before_any_write() {
    let payload = b"one\x1d\x1b[200~two\nthree";
    let expected = operation(agend_client::terminal::input_operation("attach-1", payload));
    let received = expected.clone();
    let (_dir, client, worker) = peer(&[V1_4], move |mut reader, _| {
        assert_eq!(read(&mut reader), Some(received));
        assert!(
            read(&mut reader).is_none(),
            "invalid operations must not write a prefix"
        );
    });
    let mut sender = client.sender().unwrap();
    for request in [
        operation(agend_client::terminal::input_operation(
            "attach-1",
            &vec![b'x'; 800_000],
        )),
        operation(ClientTerminalOperation::Input {
            attach_id: "attach-1".into(),
            bytes_base64: "???".into(),
        }),
        operation(ClientTerminalOperation::Resize {
            attach_id: "attach-1".into(),
            size: TerminalSize {
                rows: 0,
                columns: 40,
            },
        }),
    ] {
        assert!(matches!(
            sender.send_terminal(&request),
            Err(ClientError::Daemon { .. })
        ));
    }
    sender.send_terminal(&expected).unwrap();
    sender.close();
    worker.join().unwrap();
}

#[test]
fn failed_control_write_cannot_reconnect_to_a_replacement_peer() {
    let (gate, arrived) = mpsc::channel();
    let (dir, mut client, worker) = peer(&[V1_4], move |reader, writer| {
        writer.shutdown(Shutdown::Both).unwrap();
        drop(reader);
        drop(writer);
        gate.send(()).unwrap();
    });
    arrived.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.join().unwrap();
    let socket = dir.path().join("daemon.sock");
    std::fs::remove_file(&socket).unwrap();
    let replacement = UnixListener::bind(socket).unwrap();
    replacement.set_nonblocking(true).unwrap();
    // Consume authoritative EOF before trying the old sender.
    assert!(client.next_full_terminal().is_err());
    let mut sender = client.sender().unwrap();
    let started = Instant::now();
    assert!(
        sender
            .send_terminal(&operation(ClientTerminalOperation::Acquire {
                size: TerminalSize {
                    rows: 3,
                    columns: 40
                }
            }))
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        replacement.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "failed control must not connect to the replacement daemon"
    );
}

#[test]
fn viewport_and_control_requests_reach_the_peer_once_with_their_ids() {
    let frame = frame();
    let frame_generation = frame.generation.clone();
    let (_dir, mut client, worker) = peer(&[V1_4], move |mut reader, mut writer| {
        let ClientRequest::SetTerminalViewport { data } = read(&mut reader).unwrap() else {
            panic!("viewport expected")
        };
        assert_eq!(data.request_id, "viewport-2");
        assert_eq!(data.view_id, "view-1");
        assert_eq!(data.viewport.top, Some(100));
        let ClientRequest::TerminalControl { data } = read(&mut reader).unwrap() else {
            panic!("control expected")
        };
        assert_eq!(data.request_id, "acquire-3");
        assert_eq!(data.generation, frame.generation);
        assert_eq!(
            data.operation,
            ClientTerminalOperation::Acquire { size: frame.size }
        );
        send(
            &mut writer,
            ClientResponse::TerminalControlAck {
                data: ClientTerminalControlAck {
                    request_id: data.request_id,
                    instance_id: data.instance_id,
                    view_id: data.view_id,
                    generation: frame.generation.clone(),
                    control: TerminalControlState::Controlled {
                        attach_id: "fresh-3".into(),
                    },
                    frame: Some(frame),
                },
            },
        );
        assert!(read(&mut reader).is_none());
    });
    let mut sender = client.sender().unwrap();
    sender
        .set_terminal_viewport(TerminalViewportData {
            request_id: "viewport-2".into(),
            instance_id: "i-1".into(),
            view_id: "view-1".into(),
            generation: frame_generation.clone(),
            viewport: TerminalViewport {
                top: Some(100),
                rows: 3,
            },
        })
        .unwrap();
    sender
        .terminal_control(ClientTerminalControlData {
            request_id: "acquire-3".into(),
            instance_id: "i-1".into(),
            view_id: "view-1".into(),
            generation: frame_generation,
            operation: ClientTerminalOperation::Acquire {
                size: TerminalSize {
                    rows: 3,
                    columns: 40,
                },
            },
        })
        .unwrap();
    let FullTerminalUpdate::ControlAck(data) = client.next_full_terminal().unwrap() else {
        panic!("ack expected")
    };
    assert_eq!(data.request_id, "acquire-3");
    sender.close();
    worker.join().unwrap();
}

#[test]
fn native_socket_backpressure_expires_the_whole_write_and_does_not_replay() {
    let (read_started, signal) = mpsc::channel();
    let (write_finished, finished) = mpsc::channel();
    let (_dir, mut client, worker) = peer(&[V1_4], move |mut reader, _writer| {
        let mut prefix = [0; 1024];
        std::io::Read::read_exact(&mut reader, &mut prefix).unwrap();
        read_started.send(()).unwrap();
        // The peer remains alive without draining until the actual write
        // has returned. Then check its buffered prefix and authoritative EOF.
        finished.recv_timeout(Duration::from_secs(12)).unwrap();
        let mut remainder = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut remainder).unwrap();
        assert!(
            remainder.len() + prefix.len() < 900_000,
            "the blocked request must not be replayed"
        );
    });
    let mut sender = client.sender().unwrap();
    let write = thread::spawn(move || {
        let started = Instant::now();
        let error = sender
            .send_terminal(&operation(agend_client::terminal::input_operation(
                "attach-1",
                &vec![b'x'; 700_000],
            )))
            .unwrap_err();
        (started.elapsed(), error)
    });
    signal.recv_timeout(Duration::from_secs(2)).unwrap();
    let (elapsed, error) = write.join().unwrap();
    write_finished.send(()).unwrap();
    assert!(
        elapsed >= Duration::from_secs(4) && elapsed < Duration::from_secs(10),
        "deadline was {elapsed:?}"
    );
    assert!(error.to_string().contains("not replayed"));
    assert!(client.next_full_terminal().is_err());
    worker.join().unwrap();
}

#[test]
fn invalid_frame_permanently_invalidates_even_buffered_following_frames() {
    let mut invalid = frame();
    invalid.size.rows = 0;
    let mut bytes = serde_json::to_vec(&framed(invalid)).unwrap();
    bytes.push(b'\n');
    bytes.extend(serde_json::to_vec(&framed(frame())).unwrap());
    bytes.push(b'\n');
    let (_dir, mut client, worker) = peer(&[V1_4], move |mut reader, mut writer| {
        let _ = writer.write_all(&bytes);
        assert!(read(&mut reader).is_none());
    });
    assert!(client.next_full_terminal().is_err());
    let error = client.next_full_terminal().unwrap_err();
    assert!(error.to_string().contains("reconnect read-only"));
    worker.join().unwrap();
}

#[test]
fn outgoing_line_limit_includes_newline_and_rejects_the_whole_request() {
    use agend_core::protocol::holder::MAX_REQUEST_LINE;
    let base = operation(agend_client::terminal::input_operation("attach-1", b""));
    let overhead = serde_json::to_vec(&base).unwrap().len() + 1;
    let payload = vec![b'x'; ((MAX_REQUEST_LINE - overhead) / 4) * 3];
    let mut exact = operation(agend_client::terminal::input_operation(
        "attach-1", &payload,
    ));
    let padding = MAX_REQUEST_LINE - serde_json::to_vec(&exact).unwrap().len() - 1;
    let ClientRequest::TerminalControl { data } = &mut exact else {
        unreachable!()
    };
    data.request_id.extend(std::iter::repeat_n('x', padding));
    assert_eq!(
        serde_json::to_vec(&exact).unwrap().len() + 1,
        MAX_REQUEST_LINE
    );
    let mut oversized = exact.clone();
    let ClientRequest::TerminalControl { data } = &mut oversized else {
        unreachable!()
    };
    data.request_id.push('x');
    let expected = exact.clone();
    let (_dir, client, worker) = peer(&[V1_4], move |mut reader, _| {
        assert_eq!(read(&mut reader), Some(expected));
        assert!(
            read(&mut reader).is_none(),
            "the oversized operation must send no bytes"
        );
    });
    let mut sender = client.sender().unwrap();
    assert!(matches!(
        sender.send_terminal(&oversized),
        Err(ClientError::Daemon { .. })
    ));
    sender.send_terminal(&exact).unwrap();
    sender.close();
    worker.join().unwrap();
}

#[test]
fn cloned_senders_keep_each_operation_a_complete_json_line() {
    let (_dir, client, worker) = peer(&[V1_4], |mut reader, _| {
        let mut ids = Vec::new();
        for _ in 0..2 {
            let ClientRequest::TerminalControl { data } = read(&mut reader).unwrap() else {
                panic!("control expected")
            };
            let ClientTerminalOperation::Input { bytes_base64, .. } = data.operation else {
                panic!("input expected")
            };
            assert_eq!(bytes_base64.len(), 80_000);
            ids.push(data.request_id);
        }
        ids.sort();
        assert_eq!(ids, ["writer-1", "writer-2"]);
        assert!(read(&mut reader).is_none());
    });
    let close = client.sender().unwrap();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut writers = Vec::new();
    for id in ["writer-1", "writer-2"] {
        let mut sender = client.sender().unwrap();
        let gate = gate.clone();
        writers.push(thread::spawn(move || {
            let mut request = operation(agend_client::terminal::input_operation(
                "attach-1",
                &vec![b'x'; 60_000],
            ));
            let ClientRequest::TerminalControl { data } = &mut request else {
                unreachable!()
            };
            data.request_id = id.into();
            gate.wait();
            sender.send_terminal(&request).unwrap();
        }));
    }
    gate.wait();
    for writer in writers {
        writer.join().unwrap();
    }
    close.close();
    worker.join().unwrap();
}
