//! Gate 11C's holder link uses native holder processes and real shell agents.
//! The Lab stops only its own holders with Shutdown, including on panic.
#![cfg(unix)]

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

use agend_core::model::Backend;
use agend_core::protocol::terminal::{
    TerminalControlOperation as Op, TerminalFrame, TerminalSize, TerminalViewport,
};
use agend_core::traits::HolderLaunch;
use agend_daemon::runtime::HolderRuntime;
use agend_daemon::runtime::terminal::TerminalConnection;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_agend");
const ID: &str = "g11-terminal";

fn runtime(home: &Path) -> HolderRuntime {
    HolderRuntime::new(home, Path::new(BIN), Vec::new(), Arc::new(|_| {}))
}
fn launch(home: &Path, script: &str) -> HolderLaunch {
    HolderLaunch {
        instance_id: ID.into(),
        backend: Backend::Claude,
        executable: "/bin/bash".into(),
        args: vec!["-c".into(), script.into()],
        working_directory: home.display().to_string(),
    }
}
fn run(body: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(body);
}
fn text(frame: &TerminalFrame) -> String {
    frame
        .cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
async fn screen(connection: &TerminalConnection, marker: &str, rows: u16) -> TerminalFrame {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let frame = connection
            .frame(TerminalViewport { top: None, rows })
            .await
            .unwrap();
        if text(&frame).contains(marker) {
            return frame;
        }
        assert!(
            Instant::now() < deadline,
            "missing {marker}: {}",
            text(&frame)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn current(rt: &HolderRuntime) -> TerminalConnection {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(connection) = rt.terminal_connection(ID) {
            return connection;
        }
        assert!(Instant::now() < deadline, "runtime never reconnected");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
fn size(rows: u16, columns: u16) -> TerminalSize {
    TerminalSize { rows, columns }
}

#[test]
fn native_frames_controls_and_input_replies_are_correlated_on_the_runtime_link() {
    run(async {
        let lab = lab::Lab::with_prefix(Path::new(BIN), "g11r");
        let home = lab.home(1);
        let rt = runtime(&home);
        rt.start(&launch(&home, r#"stty -echo; printf 'READY\r\n'; while IFS= read -r line; do printf '\033[31mGOT:%s SIZE:%s\033[0m\r\n' "$line" "$(stty size)"; done"#)).await.unwrap();
        let connection = rt.terminal_connection(ID).unwrap();
        let initial = screen(&connection, "READY", 50).await;
        let first = connection
            .control(
                initial.generation.clone(),
                Op::Acquire {
                    attach_id: "a".into(),
                    size: size(8, 60),
                },
            )
            .await
            .unwrap();
        assert_eq!(first.attach_id.as_deref(), Some("a"));
        assert_eq!(first.frame.unwrap().size, size(8, 60));
        let input = connection
            .control(
                initial.generation.clone(),
                Op::Input {
                    attach_id: "a".into(),
                    bytes_base64: STANDARD.encode(b"hello\n"),
                },
            )
            .await
            .unwrap();
        assert_ne!(input.request_id, first.request_id);
        assert!(input.frame.is_none());
        let frame = screen(&connection, "GOT:hello SIZE:8 60", 8).await;
        assert!(frame.cells.iter().flatten().any(|cell| cell.text == "G"
            && matches!(
                cell.foreground,
                agend_core::protocol::terminal::TerminalColor::Indexed { index: 1 }
            )));
        // Concurrent viewport requests return their own requested row count.
        let a = connection.clone();
        let one = tokio::spawn(async move {
            a.frame(TerminalViewport { top: None, rows: 1 })
                .await
                .unwrap()
        });
        let b = connection.clone();
        let three = tokio::spawn(async move {
            b.frame(TerminalViewport { top: None, rows: 3 })
                .await
                .unwrap()
        });
        assert_eq!(one.await.unwrap().cells.len(), 1);
        assert_eq!(three.await.unwrap().cells.len(), 3);
        connection
            .control(
                initial.generation.clone(),
                Op::Acquire {
                    attach_id: "b".into(),
                    size: size(6, 40),
                },
            )
            .await
            .unwrap();
        let error = connection
            .control(
                initial.generation.clone(),
                Op::Input {
                    attach_id: "a".into(),
                    bytes_base64: STANDARD.encode(b"NEVER_WRITE\n"),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "control_lost");
        connection
            .control(
                initial.generation.clone(),
                Op::Input {
                    attach_id: "b".into(),
                    bytes_base64: STANDARD.encode(b"new\n"),
                },
            )
            .await
            .unwrap();
        let frame = screen(&connection, "GOT:new SIZE:6 40", 6).await;
        assert!(!text(&frame).contains("NEVER_WRITE"));
        connection
            .control(
                initial.generation,
                Op::Release {
                    attach_id: "b".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            connection
                .frame(TerminalViewport { top: None, rows: 6 })
                .await
                .unwrap()
                .size,
            size(6, 40)
        );
        rt.stop(ID).await.unwrap();
    });
}

#[test]
fn cancelled_native_input_invalidates_the_old_connection_without_replaying_after_reconnect() {
    run(async {
        let lab = lab::Lab::with_prefix(Path::new(BIN), "g11r");
        let home = lab.home(1);
        let rt = runtime(&home);
        let started = rt
            .start(&launch(&home, "stty raw -echo; printf READY; sleep 60"))
            .await
            .unwrap();
        let connection = rt.terminal_connection(ID).unwrap();
        let initial = screen(&connection, "READY", 50).await;
        connection
            .control(
                initial.generation.clone(),
                Op::Acquire {
                    attach_id: "old".into(),
                    size: size(8, 60),
                },
            )
            .await
            .unwrap();
        let input = connection.control(
            initial.generation.clone(),
            Op::Input {
                attach_id: "old".into(),
                bytes_base64: STANDARD.encode(vec![b'x'; 500_000]),
            },
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(200), input)
                .await
                .is_err(),
            "native agent unexpectedly read input"
        );
        assert!(
            !connection.is_current(),
            "cancelled request kept its connection usable"
        );
        assert_eq!(
            connection
                .frame(TerminalViewport { top: None, rows: 1 })
                .await
                .unwrap_err()
                .code,
            "stale_terminal"
        );
        let fresh = current(&rt).await;
        let frame = fresh
            .frame(TerminalViewport { top: None, rows: 8 })
            .await
            .unwrap();
        assert_eq!(frame.generation, initial.generation);
        assert_eq!(frame.size, size(8, 60));
        assert_eq!(
            agend_daemon::runtime::files::running(&home, ID).unwrap(),
            started.handle.process_id
        );
        assert_eq!(
            fresh
                .control(
                    initial.generation.clone(),
                    Op::Input {
                        attach_id: "old".into(),
                        bytes_base64: STANDARD.encode(b"NO")
                    }
                )
                .await
                .unwrap_err()
                .code,
            "control_lost"
        );
        let grant = fresh
            .control(
                initial.generation,
                Op::Acquire {
                    attach_id: "fresh".into(),
                    size: size(6, 40),
                },
            )
            .await
            .unwrap();
        assert_eq!(grant.frame.unwrap().size, size(6, 40));
        // The stale clone still cannot follow the new connection.
        assert!(!connection.is_current());
        rt.stop(ID).await.unwrap();
    });
}

#[test]
fn oversized_input_is_refused_whole_and_a_new_holder_gets_a_new_generation() {
    run(async {
        let lab = lab::Lab::with_prefix(Path::new(BIN), "g11r");
        let home = lab.home(1);
        let rt = runtime(&home);
        rt.start(&launch(
            &home,
            r#"stty -echo; printf READY; while read -r line; do printf '%s\r\n' "$line"; done"#,
        ))
        .await
        .unwrap();
        let old = rt.terminal_connection(ID).unwrap();
        let initial = screen(&old, "READY", 50).await;
        old.control(
            initial.generation.clone(),
            Op::Acquire {
                attach_id: "a".into(),
                size: size(8, 60),
            },
        )
        .await
        .unwrap();
        let error = old
            .control(
                initial.generation.clone(),
                Op::Input {
                    attach_id: "a".into(),
                    bytes_base64: STANDARD.encode(vec![b'x'; 800_000]),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "invalid_request");
        assert!(
            old.is_current(),
            "whole rejection closed a usable connection"
        );
        old.control(
            initial.generation.clone(),
            Op::Input {
                attach_id: "a".into(),
                bytes_base64: STANDARD.encode(b"after rejection\n"),
            },
        )
        .await
        .unwrap();
        screen(&old, "after rejection", 8).await;
        rt.stop(ID).await.unwrap();
        rt.start(&launch(&home, "printf REPLACED; exec sleep 60"))
            .await
            .unwrap();
        assert!(!old.is_current());
        let fresh = rt.terminal_connection(ID).unwrap();
        let frame = screen(&fresh, "REPLACED", 50).await;
        assert_ne!(frame.generation, initial.generation);
        assert_eq!(
            fresh
                .control(
                    initial.generation,
                    Op::Acquire {
                        attach_id: "a".into(),
                        size: size(8, 60)
                    }
                )
                .await
                .unwrap_err()
                .code,
            "stale_terminal"
        );
        rt.stop(ID).await.unwrap();
    });
}

#[test]
fn cancelling_an_acquire_after_its_native_reply_arrived_cannot_leave_a_live_grant() {
    run(async {
        use std::future::Future;
        use std::task::{Context, Poll, Waker};
        let lab = lab::Lab::with_prefix(Path::new(BIN), "g11r");
        let home = lab.home(1);
        let rt = runtime(&home);
        rt.start(&launch(&home, "printf READY; exec sleep 60"))
            .await
            .unwrap();
        let connection = rt.terminal_connection(ID).unwrap();
        let initial = screen(&connection, "READY", 50).await;
        let mut grant = Box::pin(connection.control(
            initial.generation.clone(),
            Op::Acquire {
                attach_id: "unconsumed".into(),
                size: size(6, 40),
            },
        ));
        assert!(matches!(
            grant.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        // Do not poll the grant consumer. A frame at the new size comes after
        // the native Acquire ack on the same holder response stream, proving
        // the reply is queued while the consumer has not accepted it.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let frame = connection
                .frame(TerminalViewport { top: None, rows: 1 })
                .await
                .unwrap();
            if frame.size == size(6, 40) {
                break;
            }
            assert!(Instant::now() < deadline, "native acquire never resized");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(connection.is_current());
        drop(grant);
        assert!(
            !connection.is_current(),
            "an unconsumed grant survived cancellation"
        );
        let fresh = current(&rt).await;
        assert_eq!(
            fresh
                .control(
                    initial.generation.clone(),
                    Op::Input {
                        attach_id: "unconsumed".into(),
                        bytes_base64: STANDARD.encode(b"NO")
                    }
                )
                .await
                .unwrap_err()
                .code,
            "control_lost"
        );
        assert_eq!(
            fresh
                .frame(TerminalViewport { top: None, rows: 1 })
                .await
                .unwrap()
                .size,
            size(6, 40)
        );
        rt.stop(ID).await.unwrap();
    });
}

#[test]
fn cancelling_a_readonly_frame_keeps_the_native_controller_and_connection() {
    run(async {
        use std::future::Future;
        use std::task::{Context, Poll, Waker};
        let lab = lab::Lab::with_prefix(Path::new(BIN), "g11r");
        let home = lab.home(1);
        let rt = runtime(&home);
        rt.start(&launch(
            &home,
            r#"stty -echo; printf READY; while read -r line; do printf '%s\r\n' "$line"; done"#,
        ))
        .await
        .unwrap();
        let connection = rt.terminal_connection(ID).unwrap();
        let initial = screen(&connection, "READY", 50).await;
        connection
            .control(
                initial.generation.clone(),
                Op::Acquire {
                    attach_id: "active".into(),
                    size: size(8, 60),
                },
            )
            .await
            .unwrap();
        let mut query = Box::pin(connection.frame(TerminalViewport { top: None, rows: 1 }));
        assert!(matches!(
            query.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        drop(query);
        assert!(
            connection.is_current(),
            "a cancelled history lookup interrupted control"
        );
        connection
            .control(
                initial.generation,
                Op::Input {
                    attach_id: "active".into(),
                    bytes_base64: STANDARD.encode(b"still controlled\n"),
                },
            )
            .await
            .unwrap();
        screen(&connection, "still controlled", 8).await;
        rt.stop(ID).await.unwrap();
    });
}
