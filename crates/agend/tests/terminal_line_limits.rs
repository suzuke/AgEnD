//! Gate 9's line limits against the TUI's terminal path (gate 11 B):
//! `MAX_LINE_BYTES` (8 MiB) bounds only the lines the daemon *reads* from a
//! client, and the holder's `MAX_REQUEST_LINE` (1 MiB) the lines it reads
//! from the daemon. On the terminal path the client sends `subscribe_terminal`
//! and one `terminal_input` per key (a few bytes each); the large lines
//! (the screen, PTY chunks, the fleet view) go daemon → client, where no
//! limit applies. These pin that the largest lines of that path still fit
//! far inside the limits, built by the real producers.

use agend_core::protocol::client::{
    ClientRequest, ClientResponse, InstanceData, MAX_LINE_BYTES, TerminalBytesData,
    TerminalInputData, TerminalSnapshotData,
};
use agend_holder::screen::Screen;
use agend_holder::server::{MAX_REQUEST_LINE, MAX_SCREEN_SIDE};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The holder reads the PTY in chunks of this many bytes (`server.rs`).
const PTY_CHUNK: usize = 8192;

fn line_len<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).unwrap().len() + 1
}

fn snapshot_line(rows: u16, columns: u16) -> usize {
    let mut screen = Screen::new(rows, columns, Default::default());
    // A 4-byte, one-column character in every cell: the most bytes a
    // screen of this size can carry.
    let row: String = "𝕏".repeat(columns as usize);
    for r in 0..rows {
        screen.process(row.as_bytes());
        if r + 1 < rows {
            screen.process(b"\r\n");
        }
    }
    let text = screen.text();
    assert!(
        text.len() >= rows as usize * columns as usize * 4,
        "{}",
        text.len()
    );
    line_len(&ClientResponse::TerminalSnapshot {
        data: TerminalSnapshotData {
            instance_id: "g11-limits-long-name-xx".into(),
            screen: text,
        },
    })
}

#[test]
fn a_full_screen_fits_in_a_line_even_at_the_holders_largest_size() {
    let default = snapshot_line(50, 200);
    assert!(default < 64 << 10, "50x200: {default} bytes");
    let largest = snapshot_line(MAX_SCREEN_SIDE, MAX_SCREEN_SIDE);
    assert!(largest < MAX_LINE_BYTES, "1000x1000: {largest} bytes");
    println!(
        "screen line: 50x200 {default} bytes, 1000x1000 {largest} bytes, limit {MAX_LINE_BYTES}"
    );
}

#[test]
fn pty_chunks_and_keys_are_small_lines() {
    use base64::Engine;
    let chunk = line_len(&ClientResponse::TerminalBytes {
        data: TerminalBytesData {
            instance_id: "g11-limits-long-name-xx".into(),
            bytes_base64: base64::engine::general_purpose::STANDARD.encode([0xff; PTY_CHUNK]),
        },
    });
    assert!(chunk < 16 << 10, "{chunk}");
    let keys = [
        KeyEvent::new(KeyCode::Char('𝕏'), KeyModifiers::ALT),
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::ALT),
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
    ];
    let longest = keys
        .iter()
        .map(|k| {
            let bytes = agend_tui::app::key_bytes(k);
            assert!(!bytes.is_empty());
            line_len(&ClientRequest::TerminalInput {
                data: TerminalInputData {
                    instance_id: "g11-limits-long-name-xx".into(),
                    bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                },
            })
        })
        .max()
        .unwrap();
    let subscribe = line_len(&ClientRequest::SubscribeTerminal {
        data: InstanceData {
            instance_id: "g11-limits-long-name-xx".into(),
        },
    });
    assert!(longest < 256 && subscribe < 256, "{longest} {subscribe}");
    // The holder's 1 MiB is the tighter of the two limits on this line.
    assert!(longest < MAX_REQUEST_LINE && MAX_REQUEST_LINE < MAX_LINE_BYTES);
    println!(
        "terminal_bytes line {chunk}, terminal_input line <= {longest}, subscribe {subscribe}"
    );
}
