//! Native holder/PTY control operations use the same reader and writer queue.
use super::*;
use agend_core::protocol::terminal::{
    TerminalControlData, TerminalControlOperation as Op, TerminalControlRequest, TerminalSize,
};
use base64::{Engine, engine::general_purpose::STANDARD};

fn generation(client: &mut HolderClient) -> String {
    request_frame(client, "generation", None, 1);
    let HolderResponse::TerminalFrame { data } = frame_reply(client) else {
        panic!("missing frame generation")
    };
    data.frame.generation
}

fn submit(client: &mut HolderClient, id: &str, generation: &str, operation: Op) {
    client
        .send(&HolderRequest::TerminalControl {
            data: TerminalControlRequest {
                request_id: id.into(),
                generation: generation.into(),
                operation,
            },
        })
        .unwrap();
}

fn response(client: &mut HolderClient) -> HolderResponse {
    let deadline = Instant::now() + LONG;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "control response missed deadline");
        match client.recv(remaining).unwrap() {
            Some(
                reply @ (HolderResponse::TerminalControl { .. }
                | HolderResponse::TerminalOperationError { .. }),
            ) => return reply,
            Some(HolderResponse::PtyBytes { .. } | HolderResponse::Exited { .. }) => (),
            other => panic!("unexpected control reply: {other:?}"),
        }
    }
}

fn accepted(client: &mut HolderClient, id: &str) -> TerminalControlData {
    let HolderResponse::TerminalControl { data } = response(client) else {
        panic!("control operation refused")
    };
    assert_eq!(data.request_id, id);
    data
}

fn refused(client: &mut HolderClient, id: &str, code: &str) {
    let HolderResponse::TerminalOperationError { data } = response(client) else {
        panic!("invalid control operation accepted")
    };
    assert_eq!(data.request_id, id);
    assert_eq!(data.code, code);
}

#[test]
fn actual_resize_and_input_acknowledgements_follow_fifo_and_last_acquire_wins() {
    let holder = TestHolder::start(Duration::from_secs(100));
    let (mut client, _) = holder.spawn_bash(r#"stty -echo; printf 'READY\r\n'; while IFS= read -r line; do printf 'GOT:%s SIZE:%s\r\n' "$line" "$(stty size)"; done"#);
    wait_screen(&mut client, |s| s.contains("READY"));
    let generation = generation(&mut client);
    submit(
        &mut client,
        "first",
        &generation,
        Op::Acquire {
            attach_id: "window-a".into(),
            size: TerminalSize {
                rows: 7,
                columns: 40,
            },
        },
    );
    let first = accepted(&mut client, "first");
    assert_eq!(first.attach_id.as_deref(), Some("window-a"));
    assert_eq!(
        first.frame.unwrap().size,
        TerminalSize {
            rows: 7,
            columns: 40
        }
    );
    submit(
        &mut client,
        "input-a",
        &generation,
        Op::Input {
            attach_id: "window-a".into(),
            bytes_base64: STANDARD.encode(b"one\n"),
        },
    );
    submit(
        &mut client,
        "second",
        &generation,
        Op::Acquire {
            attach_id: "window-b".into(),
            size: TerminalSize {
                rows: 9,
                columns: 50,
            },
        },
    );
    submit(
        &mut client,
        "stale",
        &generation,
        Op::Input {
            attach_id: "window-a".into(),
            bytes_base64: STANDARD.encode(b"NEVER_WRITE\n"),
        },
    );
    accepted(&mut client, "input-a");
    let second = accepted(&mut client, "second");
    assert_eq!(
        second.frame.unwrap().size,
        TerminalSize {
            rows: 9,
            columns: 50
        }
    );
    refused(&mut client, "stale", "control_lost");
    submit(
        &mut client,
        "input-b",
        &generation,
        Op::Input {
            attach_id: "window-b".into(),
            bytes_base64: STANDARD.encode(b"two\n"),
        },
    );
    accepted(&mut client, "input-b");
    let screen = wait_screen(&mut client, |s| s.contains("GOT:two SIZE:9 50"));
    assert!(!screen.contains("NEVER_WRITE"));
    submit(
        &mut client,
        "resize-b",
        &generation,
        Op::Resize {
            attach_id: "window-b".into(),
            size: TerminalSize {
                rows: 6,
                columns: 45,
            },
        },
    );
    assert_eq!(
        accepted(&mut client, "resize-b").frame.unwrap().size,
        TerminalSize {
            rows: 6,
            columns: 45
        }
    );
    submit(
        &mut client,
        "input-size",
        &generation,
        Op::Input {
            attach_id: "window-b".into(),
            bytes_base64: STANDARD.encode(b"size\n"),
        },
    );
    accepted(&mut client, "input-size");
    wait_screen(&mut client, |s| s.contains("GOT:size SIZE:6 45"));
    submit(
        &mut client,
        "release",
        &generation,
        Op::Release {
            attach_id: "window-b".into(),
        },
    );
    assert_eq!(accepted(&mut client, "release").attach_id, None);
    // Release retains the last size, and legacy input works once no owner exists.
    client
        .send(&HolderRequest::OperatorTerminalInput {
            data: agend_core::protocol::holder::OperatorTerminalInputData {
                bytes_base64: STANDARD.encode(b"legacy\n"),
            },
        })
        .unwrap();
    wait_screen(&mut client, |s| s.contains("GOT:legacy SIZE:6 45"));
}

#[test]
fn old_owner_legacy_input_bad_generation_and_invalid_size_have_no_pty_effects() {
    let holder = TestHolder::start(Duration::from_secs(100));
    let (mut client, _) = holder.spawn_bash(r#"stty -echo; printf 'READY\r\n'; while IFS= read -r line; do printf 'GOT:%s SIZE:%s\r\n' "$line" "$(stty size)"; done"#);
    wait_screen(&mut client, |s| s.contains("READY"));
    let generation = generation(&mut client);
    submit(
        &mut client,
        "take",
        &generation,
        Op::Acquire {
            attach_id: "active".into(),
            size: TerminalSize {
                rows: 8,
                columns: 60,
            },
        },
    );
    accepted(&mut client, "take");
    for (id, requested_generation, op, code) in [
        (
            "old-owner",
            generation.as_str(),
            Op::Resize {
                attach_id: "old".into(),
                size: TerminalSize {
                    rows: 3,
                    columns: 10,
                },
            },
            "control_lost",
        ),
        (
            "bad-gen",
            "old-holder",
            Op::Input {
                attach_id: "active".into(),
                bytes_base64: STANDARD.encode(b"NEVER_WRITE\n"),
            },
            "stale_terminal",
        ),
        (
            "zero",
            generation.as_str(),
            Op::Resize {
                attach_id: "active".into(),
                size: TerminalSize {
                    rows: 0,
                    columns: 10,
                },
            },
            "invalid_size",
        ),
        (
            "huge",
            generation.as_str(),
            Op::Resize {
                attach_id: "active".into(),
                size: TerminalSize {
                    rows: 1000,
                    columns: 1000,
                },
            },
            "frame_too_large",
        ),
    ] {
        submit(&mut client, id, requested_generation, op);
        refused(&mut client, id, code);
    }
    client
        .send(&HolderRequest::OperatorTerminalInput {
            data: agend_core::protocol::holder::OperatorTerminalInputData {
                bytes_base64: STANDARD.encode(b"NEVER_WRITE\n"),
            },
        })
        .unwrap();
    loop {
        match client.recv(LONG).unwrap() {
            Some(HolderResponse::Error { data }) => {
                assert_eq!(data.code, "control_lost");
                break;
            }
            Some(HolderResponse::PtyBytes { .. }) => (),
            other => panic!("legacy input wasn't refused: {other:?}"),
        }
    }
    submit(
        &mut client,
        "valid",
        &generation,
        Op::Input {
            attach_id: "active".into(),
            bytes_base64: STANDARD.encode(b"valid\n"),
        },
    );
    accepted(&mut client, "valid");
    let screen = wait_screen(&mut client, |s| s.contains("GOT:valid SIZE:8 60"));
    assert!(!screen.contains("NEVER_WRITE"));
}

#[test]
fn holder_reconnect_invalidates_attach_and_preserves_size_until_explicit_retake() {
    let holder = TestHolder::start(Duration::from_secs(100));
    let (mut first, _) = holder
        .spawn_bash(r#"printf 'READY'; while read -r line; do printf '%s\r\n' "$line"; done"#);
    wait_screen(&mut first, |s| s.contains("READY"));
    let generation = generation(&mut first);
    submit(
        &mut first,
        "take",
        &generation,
        Op::Acquire {
            attach_id: "first".into(),
            size: TerminalSize {
                rows: 8,
                columns: 60,
            },
        },
    );
    accepted(&mut first, "take");
    drop(first);
    let (mut second, _) = holder.connect();
    submit(
        &mut second,
        "stale",
        &generation,
        Op::Input {
            attach_id: "first".into(),
            bytes_base64: STANDARD.encode(b"NO\n"),
        },
    );
    refused(&mut second, "stale", "control_lost");
    request_frame(&mut second, "size", None, 1);
    let HolderResponse::TerminalFrame { data } = frame_reply(&mut second) else {
        panic!("missing retained frame")
    };
    assert_eq!(
        data.frame.size,
        TerminalSize {
            rows: 8,
            columns: 60
        }
    );
    assert_eq!(data.frame.generation, generation);
    submit(
        &mut second,
        "retake",
        &generation,
        Op::Acquire {
            attach_id: "second".into(),
            size: TerminalSize {
                rows: 5,
                columns: 30,
            },
        },
    );
    assert_eq!(
        accepted(&mut second, "retake").frame.unwrap().size,
        TerminalSize {
            rows: 5,
            columns: 30
        }
    );
}

#[test]
fn unnegotiated_control_is_refused_before_generation_or_live_agent_checks() {
    let holder = TestHolder::start(Duration::from_secs(100));
    let mut client = HolderClient::connect_with(
        &holder.socket,
        &HolderRequest::Hello {
            data: Hello::new(&[agend_core::protocol::holder::V1]),
        },
    )
    .unwrap();
    assert!(matches!(
        client.recv(LONG).unwrap(),
        Some(HolderResponse::Hello { .. })
    ));
    assert!(matches!(
        client.recv(LONG).unwrap(),
        Some(HolderResponse::ScreenSnapshot { .. })
    ));
    submit(
        &mut client,
        "old-client",
        "wrong-generation",
        Op::Acquire {
            attach_id: "old".into(),
            size: TerminalSize {
                rows: 24,
                columns: 80,
            },
        },
    );
    refused(&mut client, "old-client", "not_supported");
}

#[test]
fn native_pty_backpressure_finishes_old_input_before_granting_new_control() {
    let holder = TestHolder::start(Duration::from_secs(100));
    // Raw mode prevents canonical input from silently discarding a full line.
    // This live agent intentionally never reads its stdin.
    let (mut client, _) = holder.spawn_bash("stty raw -echo; printf READY; sleep 60");
    wait_screen(&mut client, |s| s.contains("READY"));
    let generation = generation(&mut client);
    submit(
        &mut client,
        "old",
        &generation,
        Op::Acquire {
            attach_id: "old".into(),
            size: TerminalSize {
                rows: 8,
                columns: 60,
            },
        },
    );
    accepted(&mut client, "old");
    let started = Instant::now();
    submit(
        &mut client,
        "blocked-input",
        &generation,
        Op::Input {
            attach_id: "old".into(),
            bytes_base64: STANDARD.encode(vec![b'x'; 500_000]),
        },
    );
    submit(
        &mut client,
        "new",
        &generation,
        Op::Acquire {
            attach_id: "new".into(),
            size: TerminalSize {
                rows: 9,
                columns: 70,
            },
        },
    );
    // The input must really hit the native PTY's finite deadline. The grant
    // cannot acknowledge while an old write is still blocked in the kernel.
    let HolderResponse::TerminalOperationError { data } = response(&mut client) else {
        panic!("grant or input success arrived before native backpressure ended")
    };
    assert_eq!(data.request_id, "blocked-input");
    assert_eq!(data.code, "pty_write_failed");
    assert!(
        data.message.contains("exceeded 5 seconds"),
        "{}",
        data.message
    );
    assert!(started.elapsed() >= Duration::from_secs(5));
    let grant = accepted(&mut client, "new");
    assert_eq!(grant.attach_id.as_deref(), Some("new"));
    assert_eq!(
        grant.frame.unwrap().size,
        TerminalSize {
            rows: 9,
            columns: 70
        }
    );
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "handoff remained stuck"
    );
    submit(
        &mut client,
        "old-again",
        &generation,
        Op::Input {
            attach_id: "old".into(),
            bytes_base64: STANDARD.encode(b"NEVER_WRITE"),
        },
    );
    refused(&mut client, "old-again", "control_lost");
}
