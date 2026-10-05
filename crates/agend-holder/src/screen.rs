//! Maintains the terminal screen with alacritty_terminal and serves snapshots.
//! A reconnecting daemon gets the current screen, never a byte replay (a
//! replay can start in the middle of an escape sequence).
//!
//! The legacy snapshot is plain text of the visible screen: each row with trailing
//! spaces removed, rows joined with `\n` (P5). Scrollback (1,000 rows) stays in
//! memory. Structured frames (holder 1.1) include cells, modes, cursor and a
//! requested viewport, with process generation and monotonic revision.
//!
//! Terminal queries the agent prints (for example cursor position `ESC[6n`)
//! produce replies from alacritty; they are handed to the PTY writer queue
//! without blocking and dropped if the queue is full (P6, third writer).
//!
//! Must NOT: classify the screen (that is `agend_core::screen`).

use std::sync::atomic::{AtomicU64, Ordering};

use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

mod frame;
mod history;
mod narrow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    InvalidSize,
    TooLarge,
}

impl FrameError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidSize => "invalid_size",
            Self::TooLarge => "frame_too_large",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidSize => "viewport rows must be between 1 and the PTY height",
            Self::TooLarge => "terminal frame exceeds 8 MiB; nothing was truncated",
        }
    }
}

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
pub type ReplySink = Arc<OnceLock<crate::pty::WriteQueue>>;

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
    generation: String,
    revision: u64,
    history: history::History,
    sample: Option<(std::time::Instant, frame::FrameSnapshot)>,
}

impl Screen {
    pub fn new(rows: u16, columns: u16, replies: ReplySink) -> Self {
        let config = Config {
            scrolling_history: SCROLLBACK_ROWS,
            ..Config::default()
        };
        static NEXT_SCREEN: AtomicU64 = AtomicU64::new(0);
        let generation = format!(
            "{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT_SCREEN.fetch_add(1, Ordering::Relaxed)
        );
        Self {
            generation,
            revision: 0,
            history: history::History::new(rows),
            sample: None,
            term: Term::new(config, &Size { rows, columns }, QueryReplies(replies)),
            parser: Processor::new(),
            rows,
            columns,
        }
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn size(&self) -> (u16, u16) {
        (self.rows, self.columns)
    }

    pub fn process(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            let mut tracked = history::Tracked {
                term: &mut self.term,
                history: &mut self.history,
            };
            self.parser.advance(&mut tracked, bytes);
            self.revision += 1;
        }
    }

    pub fn resize(&mut self, rows: u16, columns: u16) {
        if (rows, columns) == self.size() {
            return;
        }
        self.sample = None;
        self.rows = rows;
        self.columns = columns;
        if columns == 1 {
            narrow::prepare(&mut self.term);
        }
        self.term.resize(Size { rows, columns });
        self.history.resize(&self.term);
        self.revision += 1;
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
    use std::sync::mpsc::{SyncSender, sync_channel};
    use std::time::Duration;
    struct Collect(SyncSender<Vec<u8>>);
    impl std::io::Write for Collect {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.send(bytes.to_vec()).map_err(std::io::Error::other)?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn screen() -> Screen {
        Screen::new(DEFAULT_ROWS, DEFAULT_COLUMNS, ReplySink::default())
    }

    #[test]
    fn tracking_keeps_the_direct_parser_screen_modes_and_cursor() {
        use agend_core::protocol::terminal::TerminalViewport;
        let mut tracked = Screen::new(4, 9, ReplySink::default());
        let mut direct = Screen::new(4, 9, ReplySink::default());
        let corpus = "abc漢é\r\n\x1b[31;1mred\x1b[0m\r\n0123456789\r\n\x1b[2;4r\x1b[4;1H\n\x1b[2S\x1b[r\x1b[2J\x1b[?1049hALT\x1b[?1049l\x1b[3J\x1b[?2004h\x1b[?1000h\x1b[6 q\x1bc";
        for byte in corpus.as_bytes() {
            tracked.process(&[*byte]);
            direct.parser.advance(&mut direct.term, &[*byte]);
            assert_eq!(tracked.text(), direct.text());
            let viewport = TerminalViewport { top: None, rows: 4 };
            let a = tracked.frame(viewport).unwrap();
            let b = direct.frame(viewport).unwrap();
            assert_eq!(a.cells, b.cells);
            assert_eq!(a.cursor, b.cursor);
            assert_eq!(a.modes, b.modes);
        }
    }

    #[test]
    fn single_column_resize_handles_wide_live_history_and_inactive_normal_grid() {
        use agend_core::protocol::terminal::{TerminalSize, TerminalViewport};
        // This exact regression used to loop in upstream reflow. Keep a bounded
        // watchdog so a regression fails the test process instead of hanging CI.
        let (done, receive) = std::sync::mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            if matches!(
                receive.recv_timeout(Duration::from_secs(5)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                eprintln!("single-column parser resize did not complete");
                std::process::abort();
            }
        });
        for alternate in [false, true] {
            let mut screen = Screen::new(6, 12, ReplySink::default());
            screen.process(
                "\x1b[31;4:3m界é\r\n界abc\r\n界def\r\n界ghi\r\n界jkl\r\n界mno\r\n界pqr\r\n"
                    .as_bytes(),
            );
            if alternate {
                screen.process("\x1b[?1049h\x1b[4:5m界Z\x1b[2;5H\x1b[?2004h".as_bytes());
            }
            let before = screen
                .frame(TerminalViewport { top: None, rows: 6 })
                .unwrap();
            screen.resize(6, 1);
            let after = screen
                .frame(TerminalViewport { top: None, rows: 6 })
                .unwrap();
            assert_eq!(
                after.size,
                TerminalSize {
                    rows: 6,
                    columns: 1
                }
            );
            assert_eq!(after.generation, before.generation);
            assert!(after.revision > before.revision);
            assert_eq!(after.alternate_screen, alternate);
            assert_eq!(after.modes, before.modes);
            assert_eq!(after.cursor.column, 0);
            assert!(after.cells.iter().flatten().all(|cell| cell.width <= 1));
            // New CJK and combining input must not index a nonexistent spacer.
            screen.process("\x1b[H界é\r\nQ".as_bytes());
            assert!(screen.text().contains('Q'));
            if alternate {
                screen.process(b"\x1b[?1049l");
                let normal = screen
                    .frame(TerminalViewport { top: None, rows: 6 })
                    .unwrap();
                assert!(!normal.alternate_screen);
                assert!(normal.cells.iter().flatten().all(|cell| cell.width <= 1));
                assert!(normal.history_oldest > before.live_top);
            }
            screen.resize(6, 12);
            screen.process("\x1b[H界".as_bytes());
            let restored = screen
                .frame(TerminalViewport { top: None, rows: 6 })
                .unwrap();
            assert_eq!(restored.cells[0][0].text, "界");
            assert_eq!(restored.cells[0][0].width, 2);
        }
        done.send(()).unwrap();
        watchdog.join().unwrap();
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
        sink.set(crate::pty::start_writer(Box::new(Collect(tx))))
            .unwrap();
        let mut s = Screen::new(DEFAULT_ROWS, DEFAULT_COLUMNS, sink);
        s.process(b"ab\x1b[6n");
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            b"\x1b[1;3R"
        );
    }

    #[test]
    fn replies_are_dropped_when_no_agent_or_queue_full() {
        let mut s = screen();
        s.process(b"\x1b[6n"); // no queue yet: nothing to do, no panic

        let sink = ReplySink::default();
        let (tx, rx) = sync_channel(1);
        sink.set(crate::pty::start_writer(Box::new(Collect(tx))))
            .unwrap();
        let mut s = Screen::new(DEFAULT_ROWS, DEFAULT_COLUMNS, sink);
        s.process(&b"\x1b[6n".repeat(crate::pty::WRITE_QUEUE + 4));
        assert!(rx.recv_timeout(Duration::from_secs(1)).is_ok());
        drop(rx);
    }
}
