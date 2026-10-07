//! AgEnD TUI: the attention-first dashboard. Hierarchy Fleet -> Team -> Task or
//! Agent; everything is grouped by team, and the repo appears only in Task
//! Detail. `<-`/`->` always mean one level up/down; `t` opens the selected
//! row's agent terminal and `/` jumps anywhere (DEMO-01).
//!
//! Screens read only a `source::Fleet` and act only through a
//! `source::Source`, so they do not know whether the data comes from the
//! scripted fake or a daemon (the real one or the testkit fake) through
//! `agend-client` (`source::client::ClientSource`, gate 11 B).
//!
//! [`run`] is the interactive loop (`agend app`, the `tui_fake` example):
//! raw mode and the alternate screen, a tick every [`TICK`], the terminal
//! restored on the way out. It builds no async runtime (D11).
//!
//! Must NOT: talk to the daemon except through a `Source`, or hold state the
//! daemon does not also have, apart from what the protocol cannot carry yet
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

use std::io;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyEvent};

use crate::i18n::Language;
use crate::source::Source;

/// How often the interactive loop ticks (gate 11 B P5: with the 200 ms
/// refresh, output reaches the screen within 300 ms plus a round trip).
pub const TICK: Duration = Duration::from_millis(100);
// Keep full-terminal fleet and lifecycle maintenance at 20 Hz. Already
// received terminal updates are drained independently between these ticks.
const FULL_TERMINAL_TICK: Duration = Duration::from_millis(50);
// Reading an already decoded frame does not request another holder sample.
// Check the local mailbox between fleet ticks without redrawing empty polls.
const FULL_MAILBOX_POLL: Duration = Duration::from_millis(10);

/// Runs the TUI on this terminal until `q`.
pub fn run(source: Box<dyn Source>, lang: Language) -> io::Result<()> {
    run_with(source, lang, |_| false)
}

/// [`run`], with `intercept` seeing each key first; it returns true for a
/// key it handled (the demo's own keys).
pub fn run_with(
    source: Box<dyn Source>,
    lang: Language,
    mut intercept: impl FnMut(&KeyEvent) -> bool,
) -> io::Result<()> {
    let mut app = App::new(source, lang);
    let mut terminal = ratatui::init();
    let result = (|| -> io::Result<()> {
        let _outer_modes = crate::terminal::native::OuterModes::enter(io::stdout())?;
        let size = terminal.size()?;
        app.resize(size.width, size.height);
        let mut native = crate::terminal::native_render::CellRenderer::default();
        let mut next_tick = Instant::now();
        let mut redraw = true;
        while !app.quit {
            if Instant::now() >= next_tick {
                app.tick();
                let interval = if app.term.as_ref().is_some_and(|term| term.full.is_some()) {
                    FULL_TERMINAL_TICK
                } else {
                    TICK
                };
                next_tick = Instant::now() + interval;
                redraw = true;
            } else if app.is_connected()
                && app.term.as_ref().is_some_and(|term| term.full.is_some())
            {
                redraw |= app.pump_full_terminal_ready();
            }
            if redraw {
                let completed = terminal.draw(|frame| render_frame(frame, &mut app))?;
                let paint = native.prepare(&app, completed.buffer);
                paint.write(terminal.backend_mut())?;
                redraw = false;
            }
            let mut wait = next_tick.saturating_duration_since(Instant::now());
            if app.term.as_ref().is_some_and(|term| term.full.is_some()) {
                wait = wait.min(FULL_MAILBOX_POLL);
            }
            if event::poll(wait)? {
                match event::read()? {
                    Event::Key(key) if !intercept(&key) => app.key(key),
                    Event::Key(_) => {}
                    event => app.event(event),
                }
                redraw = true;
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}

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
    terminal
        .draw(|frame| render_frame(frame, app))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let text = buffer_text(&buffer);
    (buffer, text)
}

fn render_frame(frame: &mut ratatui::Frame<'_>, app: &mut App) {
    // Native resize notifications can be delayed/coalesced. Ratatui reads the
    // current backend size before drawing; use that same size for control.
    let area = frame.area();
    app.resize(area.width, area.height);
    ui::render(frame, app);
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
