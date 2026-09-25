//! AgEnD TUI: the attention-first dashboard. Hierarchy Fleet -> Team -> Task or
//! Agent; everything is grouped by team, and the repo appears only in Task
//! Detail. `<-`/`->` always mean one level up/down; `t` opens the selected
//! row's agent terminal and `/` jumps anywhere (DEMO-01).
//!
//! Screens read only a `source::Fleet` and act only through a
//! `source::Source`, so they do not know whether the data comes from the
//! scripted fake, the testkit fake daemon, or (gate 11 proper) the real
//! daemon through `agend-client`.
//!
//! Must NOT: talk to the daemon except through a `Source`, or hold state the
//! daemon does not also have, apart from what protocol v1 cannot carry yet
//! (read marks; listed in docs/gates/gate-11-tui.md).

pub mod agent_detail;
pub mod app;
pub mod attention;
pub mod finder;
pub mod home;
pub mod i18n;
pub mod source;
pub mod task_detail;
pub mod team;
pub mod terminal;
pub mod ui;

pub use app::App;

/// Render the app off-screen and return it as text, one line per row with
/// trailing spaces trimmed. Used by tests and the acceptance demo.
pub fn render_to_string(app: &mut App, width: u16, height: u16) -> String {
    render_buffer(app, width, height).1
}

/// Like [`render_to_string`], also returning the buffer (to inspect styles).
pub fn render_buffer(app: &mut App, width: u16, height: u16) -> (ratatui::buffer::Buffer, String) {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    terminal.draw(|frame| ui::render(frame, app)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let text = buffer_text(&buffer);
    (buffer, text)
}

pub fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
    use unicode_width::UnicodeWidthStr;
    let mut out = String::new();
    for y in 0..buffer.area.height {
        let mut row = String::new();
        let mut skip = 0;
        for x in 0..buffer.area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let symbol = buffer[(x, y)].symbol();
            row.push_str(symbol);
            skip = symbol.width().saturating_sub(1);
        }
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}
