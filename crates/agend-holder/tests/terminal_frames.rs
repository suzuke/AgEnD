//! Real PTY producers exercise the holder's one parser and structured frame.
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

use agend_core::protocol::holder::SpawnData;
use agend_core::protocol::terminal::*;
use agend_holder::screen::{ReplySink, Screen};

fn output(script: &str, rows: u16, columns: u16) -> Vec<u8> {
    let mut agent = agend_holder::pty::spawn(
        &SpawnData {
            instance_id: "frame-producer".into(),
            program: "/bin/bash".into(),
            args: vec!["-c".into(), script.into()],
            env: BTreeMap::new(),
            working_directory: "/tmp".into(),
        },
        rows,
        columns,
    )
    .unwrap();
    let pid = agent.pid;
    let (done, result) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = agent.reader.read_to_end(&mut bytes);
        let _ = done.send((read, bytes));
    });
    let completed = result.recv_timeout(Duration::from_secs(10));
    if completed.is_err() {
        assert!(pid > 1);
        // SAFETY: only the child just spawned by this test, never a shared pid.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    let exit = agend_holder::exit::wait(pid);
    agend_holder::exit::reap(pid);
    reader.join().unwrap();
    let (read, bytes) = completed.expect("PTY producer finishes within 10 seconds");
    read.unwrap();
    assert_eq!(exit.code, Some(0));
    bytes
}

fn frame(screen: &Screen, top: Option<u64>, rows: u16) -> TerminalFrame {
    screen.frame(TerminalViewport { top, rows }).unwrap()
}

fn row(frame: &TerminalFrame, index: usize) -> String {
    frame.cells[index]
        .iter()
        .map(|cell| cell.text.as_str())
        .collect::<String>()
        .trim_end()
        .into()
}

#[test]
fn real_pty_styles_wide_combining_cursor_and_modes_survive_every_byte_split() {
    let bytes = output(
        r"printf '\033[31;44;1;4;7mA\033[0m\033[38;5;123mB\033[48;2;10;20;30mC\033[0m漢é\033[?1h\033=\033[?2004h\033[?1003h\033[?1006h\033[?1004h\033[6 q'",
        4,
        16,
    );
    let mut whole = Screen::new(4, 16, ReplySink::default());
    let mut split = Screen::new(4, 16, ReplySink::default());
    whole.process(&bytes);
    for byte in &bytes {
        split.process(&[*byte]);
    }
    let a = frame(&whole, None, 4);
    let b = frame(&split, None, 4);
    assert_eq!(a.cells, b.cells);
    assert_eq!(a.cursor, b.cursor);
    assert_eq!(a.modes, b.modes);
    assert_eq!(
        a.cells[0][0].foreground,
        TerminalColor::Indexed { index: 1 }
    );
    assert_eq!(
        a.cells[0][0].background,
        TerminalColor::Indexed { index: 4 }
    );
    assert_eq!(
        a.cells[0][0].style,
        style::BOLD | style::UNDERLINE | style::INVERSE
    );
    assert_eq!(
        a.cells[0][1].foreground,
        TerminalColor::Indexed { index: 123 }
    );
    assert_eq!(
        a.cells[0][2].background,
        TerminalColor::Rgb {
            r: 10,
            g: 20,
            b: 30
        }
    );
    assert_eq!(a.cells[0][3].text, "漢");
    assert_eq!(a.cells[0][3].width, 2);
    assert_eq!(a.cells[0][4].text, "");
    assert_eq!(a.cells[0][4].width, 0);
    assert_eq!(a.cells[0][5].text, "é");
    assert_eq!(a.cursor.column, 6);
    assert_eq!(a.cursor.shape, TerminalCursorShape::Beam);
    assert!(!a.cursor.blinking);
    assert!(a.modes.application_cursor && a.modes.application_keypad && a.modes.bracketed_paste);
    assert!(a.modes.sgr_mouse && a.modes.focus_reporting);
    assert_eq!(a.modes.mouse_tracking, TerminalMouseTracking::Motion);
}

#[test]
fn real_numbered_history_is_capped_and_absolute_viewport_stays_pinned() {
    let bytes = output(
        "for ((i=0;i<1200;i++)); do printf 'line %04d\r\n' \"$i\"; done",
        4,
        20,
    );
    let mut screen = Screen::new(4, 20, ReplySink::default());
    for byte in &bytes {
        screen.process(&[*byte]);
    }
    let live = frame(&screen, None, 4);
    assert_eq!(live.live_top, 1197);
    assert_eq!(live.history_oldest, 197);
    assert_eq!(row(&live, 0), "line 1197");
    let pinned = frame(&screen, Some(300), 4);
    assert_eq!(row(&pinned, 0), "line 0300");
    assert!(!pinned.cursor.visible);
    let classifier = screen.text();
    assert_eq!(classifier.split('\n').next(), Some("line 1197"));
    screen.process(&output(r"printf 'line 1200\r\n'", 4, 20));
    assert_eq!(row(&frame(&screen, Some(300), 4), 0), "line 0300");
    let trimmed = frame(&screen, Some(0), 4);
    assert_eq!(trimmed.viewport_top, 198);
    assert!(trimmed.viewport_clamped);
    assert_eq!(row(&trimmed, 0), "line 0198");
}

#[test]
fn alternate_screen_restores_normal_history_and_never_invents_alt_history() {
    let mut screen = Screen::new(3, 8, ReplySink::default());
    screen.process(&output(r"printf 'one\r\ntwo\r\nthree\r\nfour'", 3, 8));
    let normal = frame(&screen, Some(0), 3);
    screen.process(&output(r"printf '\033[?1049hALT\033[?25l'", 3, 8));
    let alt = frame(&screen, Some(0), 3);
    assert!(alt.alternate_screen);
    assert_eq!(
        (alt.history_oldest, alt.live_top, alt.viewport_top),
        (0, 0, 0)
    );
    assert!(!alt.cursor.visible);
    screen.process(&output(r"printf '\033[?1049l\033[?25h'", 3, 8));
    let back = frame(&screen, Some(0), 3);
    assert_eq!(back.cells, normal.cells);
    assert_eq!(back.live_top, normal.live_top);
    assert_eq!(back.generation, normal.generation);
    assert!(back.revision > normal.revision);
}

#[test]
fn wide_edge_and_dynamic_palette_come_from_the_real_parser() {
    let mut screen = Screen::new(3, 4, ReplySink::default());
    screen.process(&output(
        r"printf 'abc漢\033]4;1;rgb:12/34/56\007\033[31mZ'",
        3,
        4,
    ));
    let a = frame(&screen, None, 3);
    assert!(a.cells[0][3].leading_spacer);
    assert!(a.cells[0][3].wrap);
    assert_eq!(a.cells[1][0].text, "漢");
    assert_eq!(a.cells[1][0].width, 2);
    assert_eq!(
        a.cells[1][2].foreground,
        TerminalColor::Rgb {
            r: 0x12,
            g: 0x34,
            b: 0x56
        }
    );
    assert!(
        screen
            .frame(TerminalViewport { top: None, rows: 0 })
            .is_err()
    );
    assert!(
        screen
            .frame(TerminalViewport { top: None, rows: 4 })
            .is_err()
    );
}

#[test]
fn scroll_regions_explicit_scroll_clear_and_resize_do_not_reuse_history_ids() {
    let mut screen = Screen::new(4, 8, ReplySink::default());
    screen.process(&output(r"printf '\033[4;1H\n\033[1000S'", 4, 8));
    let a = frame(&screen, None, 4);
    assert_eq!(a.live_top, 5); // LF plus scroll bounded to four rows.
    screen.process(&output(r"printf '\033[2;4r\033[4;1H\n\033[10S'", 4, 8));
    assert_eq!(frame(&screen, None, 4).live_top, a.live_top);
    screen.process(&output(r"printf '\033[r\033[Habc\033[2J'", 4, 8));
    let b = frame(&screen, None, 4);
    assert!(b.live_top > a.live_top);
    screen.resize(3, 6);
    let resized = frame(&screen, Some(a.live_top), 3);
    assert_eq!(
        resized.size,
        TerminalSize {
            rows: 3,
            columns: 6
        }
    );
    assert!(resized.history_oldest > b.live_top);
    assert!(resized.viewport_clamped);
    assert!(resized.revision > b.revision);
    assert_eq!(resized.generation, a.generation);
}

#[test]
fn holder_process_generation_is_distinct_for_new_screens_and_noop_resize_keeps_revision() {
    let mut a = Screen::new(2, 8, ReplySink::default());
    let b = Screen::new(2, 8, ReplySink::default());
    let before = frame(&a, None, 2);
    assert_ne!(before.generation, frame(&b, None, 2).generation);
    a.resize(2, 8);
    a.process(b"");
    assert_eq!(frame(&a, None, 2), before);
    a.process(&output(r"printf 'x'", 2, 8));
    assert!(frame(&a, None, 2).revision > before.revision);
}

/// Frozen 1.0 reader: new variants must be unknown; existing snapshots keep
/// the adjacent data object and its plain-text field.
#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OldResponse {
    ScreenSnapshot {
        data: OldSnapshot,
    },
    #[serde(other)]
    Unknown,
}
#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
struct OldSnapshot {
    screen: String,
}

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OldRequest {
    Resize {
        data: OldSize,
    },
    Snapshot,
    #[serde(other)]
    Unknown,
}
#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
struct OldSize {
    rows: u16,
    columns: u16,
}

#[test]
fn real_frame_serializer_is_additive_for_frozen_holder_1_0_readers() {
    use agend_core::protocol::holder::*;
    let mut screen = Screen::new(2, 8, ReplySink::default());
    screen.process(&output(r"printf '\033[31m漢é\033[0m'", 2, 8));
    let response = HolderResponse::TerminalFrame {
        data: TerminalFrameData {
            request_id: "golden".into(),
            frame: frame(&screen, None, 2),
        },
    };
    let encoded = serde_json::to_vec(&response).unwrap();
    assert_eq!(
        serde_json::from_slice::<OldResponse>(&encoded).unwrap(),
        OldResponse::Unknown
    );
    assert_eq!(
        serde_json::from_slice::<HolderResponse>(&encoded).unwrap(),
        response
    );
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value["type"], "terminal_frame");
    assert_eq!(value["data"]["request_id"], "golden");
    assert_eq!(value["data"]["frame"]["cells"][0][0]["text"], "漢");
    assert_eq!(
        value["data"]["frame"]["cells"][0][0]["foreground"],
        serde_json::json!({"kind":"indexed","index":1})
    );
    let legacy = HolderResponse::ScreenSnapshot {
        data: ScreenSnapshotData {
            screen: screen.text(),
        },
    };
    let old: OldResponse = serde_json::from_slice(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(
        old,
        OldResponse::ScreenSnapshot {
            data: OldSnapshot {
                screen: screen.text()
            }
        }
    );
    let resize = HolderRequest::Resize {
        data: ResizeData {
            rows: 2,
            columns: 8,
        },
    };
    assert_eq!(
        serde_json::from_slice::<OldRequest>(&serde_json::to_vec(&resize).unwrap()).unwrap(),
        OldRequest::Resize {
            data: OldSize {
                rows: 2,
                columns: 8
            }
        }
    );
    let request = HolderRequest::GetTerminalFrame {
        data: TerminalFrameRequest {
            request_id: "golden".into(),
            viewport: TerminalViewport { top: None, rows: 2 },
        },
    };
    assert_eq!(
        serde_json::from_slice::<OldRequest>(&serde_json::to_vec(&request).unwrap()).unwrap(),
        OldRequest::Unknown
    );
}
