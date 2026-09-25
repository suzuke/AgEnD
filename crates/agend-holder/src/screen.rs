//! Maintains the terminal screen with alacritty_terminal and serves snapshots.
//! A reconnecting daemon gets the current screen, never a byte replay (a
//! replay can start in the middle of an escape sequence).
//!
//! The snapshot is plain text of the visible screen: each row with trailing
//! spaces removed, rows joined with `\n` (P5). Scrollback (1,000 rows) stays in
//! memory and is not sent in gate 4.
//!
//! Terminal queries the agent prints (for example cursor position `ESC[6n`)
//! produce replies from alacritty; they are handed to the PTY writer queue
//! without blocking and dropped if the queue is full (P6, third writer).
//!
//! Must NOT: classify the screen (that is `agend_core::screen`).

use std::sync::mpsc::SyncSender;
use std::sync::{Arc, OnceLock};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;

pub const DEFAULT_ROWS: u16 = 50;
pub const DEFAULT_COLUMNS: u16 = 200;
pub const SCROLLBACK_ROWS: usize = 1_000;

/// Where terminal-query replies go: the PTY writer queue, once an agent exists.
pub type ReplySink = Arc<OnceLock<SyncSender<Vec<u8>>>>;

pub struct QueryReplies(ReplySink);

impl EventListener for QueryReplies {
    fn send_event(&self, event: Event) {
        if let (Event::PtyWrite(text), Some(queue)) = (event, self.0.get()) {
            // Best effort: a full queue means the agent is not reading anyway.
            let _ = queue.try_send(text.into_bytes());
        }
    }
}

struct Size {
    rows: u16,
    columns: u16,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.columns as usize
    }
}

pub struct Screen {
    term: Term<QueryReplies>,
    parser: Processor,
    rows: u16,
    columns: u16,
}

impl Screen {
    pub fn new(rows: u16, columns: u16, replies: ReplySink) -> Self {
        let config = Config {
            scrolling_history: SCROLLBACK_ROWS,
            ..Config::default()
        };
        Self {
            term: Term::new(config, &Size { rows, columns }, QueryReplies(replies)),
            parser: Processor::new(),
            rows,
            columns,
        }
    }

    pub fn size(&self) -> (u16, u16) {
        (self.rows, self.columns)
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, rows: u16, columns: u16) {
        self.rows = rows;
        self.columns = columns;
        self.term.resize(Size { rows, columns });
    }

    /// Visible screen as plain text (P5).
    pub fn text(&self) -> String {
        let grid = self.term.grid();
        let mut rows = Vec::with_capacity(grid.screen_lines());
        for line in 0..grid.screen_lines() {
            let row = &grid[Line(line as i32)];
            let mut text = String::with_capacity(grid.columns());
            for column in 0..grid.columns() {
                let cell = &row[Column(column)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                text.push(cell.c);
                if let Some(extra) = cell.zerowidth() {
                    text.extend(extra);
                }
            }
            rows.push(text.trim_end().to_string());
        }
        rows.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;

    fn screen() -> Screen {
        Screen::new(DEFAULT_ROWS, DEFAULT_COLUMNS, ReplySink::default())
    }

    #[test]
    fn snapshot_is_plain_text_rows_without_trailing_spaces() {
        let mut s = screen();
        s.process(b"\x1b[31mcounter=15\x1b[0m   \r\n$ ");
        let text = s.text();
        let rows: Vec<&str> = text.split('\n').collect();
        assert_eq!(rows.len(), DEFAULT_ROWS as usize);
        assert_eq!(rows[0], "counter=15");
        assert_eq!(rows[1], "$");
        assert!(rows[2..].iter().all(|r| r.is_empty()));
    }

    #[test]
    fn a_200_column_line_does_not_wrap_and_wide_chars_are_whole() {
        let mut s = screen();
        let long = "x".repeat(200);
        s.process(long.as_bytes());
        s.process("\r\n漢字ok".as_bytes());
        let text = s.text();
        let rows: Vec<&str> = text.split('\n').collect();
        assert_eq!(rows[0], long);
        assert_eq!(rows[1], "漢字ok");
    }

    #[test]
    fn scrolled_off_rows_leave_the_visible_snapshot() {
        let mut s = screen();
        for i in 0..60 {
            s.process(format!("line {i}\r\n").as_bytes());
        }
        let text = s.text();
        assert!(!text.contains("line 10\n"));
        assert!(text.starts_with("line 11\n"));
        assert!(text.contains("line 59"));
    }

    #[test]
    fn cursor_position_query_is_answered_through_the_writer_queue() {
        let sink = ReplySink::default();
        let (tx, rx) = sync_channel(4);
        sink.set(tx).unwrap();
        let mut s = Screen::new(DEFAULT_ROWS, DEFAULT_COLUMNS, sink);
        s.process(b"ab\x1b[6n");
        assert_eq!(rx.try_recv().unwrap(), b"\x1b[1;3R");
    }

    #[test]
    fn replies_are_dropped_when_no_agent_or_queue_full() {
        let mut s = screen();
        s.process(b"\x1b[6n"); // no queue yet: nothing to do, no panic

        let sink = ReplySink::default();
        let (tx, rx) = sync_channel(1);
        sink.set(tx).unwrap();
        let mut s = Screen::new(DEFAULT_ROWS, DEFAULT_COLUMNS, sink);
        s.process(b"\x1b[6n\x1b[6n");
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());
    }
}
