//! Drawing: every screen is a list of [`Row`]s drawn into the body between a
//! breadcrumb header and a message + help footer. The selected row is marked
//! with `›` and drawn in reverse video from the marker to the right edge; the
//! row's border glyphs (`▌ ┃ ┏ ┗ ├─ └─`) and a `─`/`━` border run inside it
//! (a team header's top line) are never highlighted (DEMO-01 round 4). Widths are display columns (CJK = 2).
//!
//! Must NOT: change state (only `App::key` and `App::tick` do).

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{App, Connection, Screen, Target};
use crate::i18n::Text;
use crate::source::{AgentState, Fleet, StageState, TaskInfo};

pub const MIN_WIDTH: u16 = 70;
pub const MIN_HEIGHT: u16 = 20;

/// Padding characters that draw a border line, never highlighted.
const FRAME_FILL: [char; 2] = ['─', '━'];

/// One line of a screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    /// Frame glyphs on the left; never highlighted.
    pub border: String,
    /// Reserve a column for the `›` selection marker after the border.
    pub marker: bool,
    pub text: String,
    /// Right-aligned text.
    pub right: String,
    /// Padding between text and right (`━` for rules, space otherwise).
    pub fill: char,
    pub bold: bool,
    pub dim: bool,
    /// What `→`/`Enter` opens; `None` means not selectable.
    pub target: Option<Target>,
    /// Whose terminal `t` opens on this row.
    pub agent: Option<String>,
}

impl Row {
    pub fn blank() -> Row {
        Row {
            fill: ' ',
            ..Row::default()
        }
    }

    /// A section rule: `━━ Text ━━━━━`.
    pub fn rule(text: &str) -> Row {
        Row {
            text: format!("━━ {text} "),
            fill: '━',
            ..Row::default()
        }
    }

    /// A plain horizontal line.
    pub fn separator() -> Row {
        Row {
            fill: '─',
            ..Row::default()
        }
    }

    /// A text line inside a border, aligned with selectable rows.
    pub fn line(border: &str, text: impl Into<String>) -> Row {
        Row {
            border: border.into(),
            marker: true,
            text: text.into(),
            fill: ' ',
            ..Row::default()
        }
    }

    pub fn item(border: &str, text: impl Into<String>, target: Target) -> Row {
        Row {
            target: Some(target),
            ..Row::line(border, text)
        }
    }

    pub fn right(mut self, right: impl Into<String>) -> Row {
        self.right = right.into();
        self
    }

    pub fn agent(mut self, agent: Option<&str>) -> Row {
        self.agent = agent.map(Into::into);
        self
    }

    pub fn bold(mut self, bold: bool) -> Row {
        self.bold = bold;
        self
    }

    pub fn dim(mut self) -> Row {
        self.dim = true;
        self
    }

    /// The row as plain text of exactly `width` columns (wide characters
    /// count twice), split into (border, rest).
    pub fn layout(&self, width: usize, selected: bool) -> (String, String) {
        let (border, head, pad, right) = self.segments(width, selected);
        (border, head + &pad + &right)
    }

    /// The row split into (border, marker + text, padding, right text),
    /// `width` columns in total.
    fn segments(&self, width: usize, selected: bool) -> (String, String, String, String) {
        let border = truncate(&self.border, width);
        let mut head = String::new();
        if self.marker {
            head.push(if selected { '›' } else { ' ' });
        }
        let avail = width.saturating_sub(border.width() + head.width());
        let right = truncate(&self.right, avail.saturating_sub(4));
        let gap = if right.is_empty() { 0 } else { 1 };
        let text = truncate(&self.text, avail.saturating_sub(right.width() + gap));
        head.push_str(&text);
        let pad = avail.saturating_sub(text.width() + right.width());
        let pad = std::iter::repeat_n(self.fill, pad).collect();
        (border, head, pad, right)
    }
}

/// Cut `s` to at most `max` columns, ending with `…` when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        out.push(c);
        used += w;
    }
    if max > 0 {
        out.push('…');
    }
    out
}

/// Agent state as glyph + word (never color only).
pub fn state_label(lang: crate::i18n::Language, state: AgentState) -> String {
    let (glyph, text) = match state {
        AgentState::Working => ("●", Text::Working),
        AgentState::Idle => ("○", Text::Idle),
        AgentState::NeedsYou => ("!", Text::NeedsYouState),
        AgentState::Stuck => ("⚠", Text::Stuck),
        AgentState::Unknown => ("?", Text::Unknown),
    };
    format!("{glyph} {}", lang.tr(text))
}

/// `[■■□□□] 2/5`: derived from stage states, never stored.
pub fn stage_bar(task: &TaskInfo) -> String {
    let done = task.stages_done();
    let total = task.stages.len();
    let bar: String = task
        .stages
        .iter()
        .map(|s| {
            if s.state == StageState::Done {
                '■'
            } else {
                '□'
            }
        })
        .collect();
    format!("[{bar}] {done}/{total}")
}

/// Glyph and short status of a task: done, waiting for you, or running.
pub fn task_status(fleet: &Fleet, lang: crate::i18n::Language, task: &TaskInfo) -> (char, String) {
    match task.current_stage() {
        None => ('✓', lang.tr(Text::StatusDone).to_owned()),
        Some(_) if fleet.needs_you_for_task(&task.id).is_some() => {
            ('!', lang.tr(Text::StatusWaitingYou).to_owned())
        }
        Some(i) => ('●', lang.fmt(Text::StatusRunning, &[&task.stages[i].name])),
    }
}

/// A goal row (Home team block and team Goals tab).
pub fn goal_row(fleet: &Fleet, lang: crate::i18n::Language, border: &str, task: &TaskInfo) -> Row {
    let (glyph, status) = task_status(fleet, lang, task);
    Row::item(
        border,
        format!("{glyph} {}", task.title),
        Target::Task(task.id.clone()),
    )
    .right(format!("{:<12} {status}  {}", stage_bar(task), task.id))
    .agent(task.holder.as_deref())
}

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    let lang = app.lang;
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let w = area.width.to_string();
        let h = area.height.to_string();
        put(
            buf,
            0,
            0,
            area.width,
            &lang.fmt(Text::TooSmall, &[&w, &h]),
            Style::default(),
        );
        return;
    }
    let width = area.width as usize;
    let body_top = 2u16;
    let body_height = area.height as usize - 4;
    app.body_height = body_height;

    put(
        buf,
        0,
        0,
        area.width,
        &breadcrumb(app, width),
        Style::default().add_modifier(Modifier::BOLD),
    );

    let lang_key = lang.tr(Text::LangKey);
    let (rows, selected, offset, help) = if let Connection::Disconnected {
        reason,
        attempts,
        last_error,
    } = &app.connection
    {
        let mut rows = vec![
            Row::rule(lang.tr(Text::Disconnected)),
            Row::blank(),
            Row::line("", lang.fmt(Text::DiscReason, &[reason])).bold(true),
            Row::line("", lang.tr(Text::DiscRetrying)),
        ];
        if let Some(error) = last_error {
            rows.push(Row::line(
                "",
                lang.fmt(Text::DiscAttempt, &[&attempts.to_string(), error]),
            ));
        }
        rows.push(Row::line("", lang.tr(Text::DiscSafe)).dim());
        (rows, None, 0, lang.fmt(Text::HelpDisconnected, &[lang_key]))
    } else if app.finder.is_some() {
        let finder = app.finder.clone().unwrap_or_default();
        let results = app.finder_rows();
        let count = results.iter().filter(|r| r.target.is_some()).count();
        let selected = results
            .iter()
            .filter_map(|r| r.target.clone())
            .nth(finder.selected);
        let mut rows = vec![
            Row::rule(lang.tr(Text::FinderTitle)),
            Row {
                marker: false,
                ..Row::line("/ ", format!("{}▏", finder.query))
            }
            .right(lang.fmt(Text::FinderCount, &[&count.to_string()])),
            Row::blank(),
        ];
        if count == 0 {
            rows.push(Row::line("", lang.tr(Text::FinderNone)));
        }
        rows.extend(results);
        let index = selected
            .as_ref()
            .and_then(|t| rows.iter().position(|r| r.target.as_ref() == Some(t)))
            .unwrap_or(0);
        let offset = (index + 1).saturating_sub(body_height);
        (rows, selected, offset, lang.tr(Text::HelpFinder).to_owned())
    } else {
        let view = app.view();
        (app.rows(), view.selected.clone(), view.offset, app.help())
    };

    let offset = offset.min(rows.len().saturating_sub(body_height));
    for (line, row) in rows.iter().skip(offset).take(body_height).enumerate() {
        let is_selected = row.target.is_some() && row.target == selected;
        draw_row(buf, body_top + line as u16, area.width, row, is_selected);
    }

    let below = rows.len().saturating_sub(offset + body_height);
    let message = match (&app.input, &app.message) {
        (Some((key, text)), _) => lang.fmt(Text::AnswerPrompt, &[key, text]),
        (None, Some(message)) => message.clone(),
        (None, None) if below > 0 => lang.fmt(Text::MoreBelow, &[&below.to_string()]),
        (None, None) => String::new(),
    };
    let footer = area.height - 2;
    put(
        buf,
        0,
        footer,
        area.width,
        &message,
        Style::default().add_modifier(Modifier::BOLD),
    );
    put(
        buf,
        0,
        footer + 1,
        area.width,
        &help,
        Style::default().add_modifier(Modifier::DIM),
    );
}

fn draw_row(buf: &mut Buffer, y: u16, width: u16, row: &Row, selected: bool) {
    let (border, head, pad, right) = row.segments(width as usize, selected);
    let mut style = Style::default();
    if row.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if row.dim {
        style = style.add_modifier(Modifier::DIM);
    }
    let content = if selected {
        style.add_modifier(Modifier::REVERSED)
    } else {
        style
    };
    // A `─`/`━` padding run is part of the frame (a team header's top
    // border), so it keeps the unselected style like the border itself.
    let pad_style = if FRAME_FILL.contains(&row.fill) {
        style
    } else {
        content
    };
    let mut x = 0u16;
    for (text, style) in [
        (border, Style::default()),
        (head, content),
        (pad, pad_style),
        (right, content),
    ] {
        put(buf, x, y, width.saturating_sub(x), &text, style);
        x += text.width() as u16;
    }
}

fn put(buf: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    let area = Rect::new(x, y, width, 1).intersection(buf.area);
    if area.width > 0 {
        buf.set_stringn(area.x, area.y, text, area.width as usize, style);
    }
}

/// `AgEnD › team › T-45`, cut from the left so the current level stays readable.
fn breadcrumb(app: &App, width: usize) -> String {
    let lang = app.lang;
    let mut parts = vec!["AgEnD".to_owned()];
    for view in app.stack().iter().skip(1) {
        parts.push(match &view.screen {
            Screen::Home => continue,
            Screen::NeedsYou => lang.tr(Text::CrumbNeedsYou).to_owned(),
            Screen::Team { team, .. } => team.clone(),
            Screen::Task { task } => task.clone(),
            Screen::Agent { agent } => agent.clone(),
            Screen::Terminal { agent, .. } => format!("{agent} {}", lang.tr(Text::CrumbTerminal)),
        });
    }
    let mut crumb = parts.join(" › ");
    while crumb.width() > width && parts.len() > 1 {
        parts.remove(0);
        crumb = format!("… › {}", parts.join(" › "));
    }
    crumb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_right_aligns_and_counts_cjk_as_two_columns() {
        let row = Row::line("┃ ", "完成審查").right("T-88");
        let (border, rest) = row.layout(20, false);
        assert_eq!(border, "┃ ");
        assert_eq!(border.width() + rest.width(), 20);
        assert!(rest.ends_with("T-88"));
    }

    #[test]
    fn long_text_is_cut_with_an_ellipsis_but_right_text_stays() {
        let row = Row::line("", "a very long title that does not fit").right("T-1");
        let (_, rest) = row.layout(20, true);
        assert!(rest.starts_with('›'));
        assert!(rest.contains('…'));
        assert!(rest.ends_with("T-1"));
        assert_eq!(rest.width(), 20);
    }
}
