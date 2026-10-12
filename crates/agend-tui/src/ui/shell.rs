//! Cell-based dashboard shell. No backend state is inferred by the renderer.
use super::*;
use crate::app::shell::{Hit, Nav};

use crate::theme::{AMBER, BG, BLUE, FG, GREEN, HEADER, MUTED, RED, SELECT, SIDE};

fn base() -> Style {
    Style::default().fg(FG).bg(BG)
}
fn text(buf: &mut Buffer, rect: Rect, value: &str, style: Style) {
    put(
        buf,
        rect.x,
        rect.y,
        rect.width,
        &truncate(value, rect.width as usize),
        style,
    );
}
fn row_rect(x: u16, y: u16, width: u16) -> Rect {
    Rect::new(x, y, width, 1)
}

pub(super) fn render(frame: &mut Frame<'_>, app: &mut App) -> bool {
    if !app.shell_visible() {
        app.shell.hits.clear();
        return false;
    }
    let area = frame.area();
    let buf = frame.buffer_mut();
    buf.set_style(area, base());
    app.shell.hits.clear();
    app.shell.painted_size = (area.width, area.height);
    app.shell.painted_rows = app.rows();
    app.shell.painted_nav = app.nav_entries();
    let side = Rect::new(0, 0, 26, area.height - 2);
    buf.set_style(side, base().bg(SIDE));
    text(
        buf,
        row_rect(2, 0, 22),
        "AgEnD",
        base().bg(SIDE).fg(BLUE).bold(),
    );
    for y in 0..area.height - 2 {
        text(buf, row_rect(25, y, 1), "│", base().fg(MUTED));
    }
    sidebar(buf, app, side);
    let main = Rect::new(28, 2, area.width - 30, area.height - 5);
    text(
        buf,
        row_rect(28, 0, area.width - 30),
        &if app.screen() == &Screen::Home {
            format!("AgEnD / {}", app.lang.tr(Text::Overview))
        } else {
            breadcrumb(app, (area.width - 30) as usize)
        },
        base().bg(HEADER).bold(),
    );
    app.body_height = main.height as usize;
    if app.screen() == &Screen::NeedsYou {
        attention(buf, app, main);
    } else {
        let rows = app.rows();
        let offset = app
            .view()
            .offset
            .min(rows.len().saturating_sub(main.height as usize));
        let selected = app.view().selected.clone();
        for (i, row) in rows
            .iter()
            .skip(offset)
            .take(main.height as usize)
            .enumerate()
        {
            let rect = row_rect(main.x, main.y + i as u16, main.width);
            paint_row(
                buf,
                rect,
                row,
                row.target.is_some() && row.target == selected,
                !app.shell.focused,
            );
            if let Some(target) = &row.target {
                app.shell.hits.push((rect, Hit::Row(target.clone())));
            }
        }
    }
    let message = match (&app.input, &app.message) {
        (Some((key, value)), _) => app.lang.fmt(Text::AnswerPrompt, &[key, value]),
        (_, Some(value)) => value.clone(),
        _ if app.screen() == &Screen::NeedsYou => app.lang.tr(Text::QueueKeys).into(),
        _ if app.shell.focused => app.lang.tr(Text::SidebarKeys).into(),
        _ if matches!(app.screen(), Screen::Team { .. }) => app.lang.tr(Text::TeamKeys).into(),
        _ => String::new(),
    };
    text(
        buf,
        row_rect(0, area.height - 2, area.width),
        &message,
        base().fg(AMBER),
    );
    text(
        buf,
        row_rect(0, area.height - 1, area.width),
        app.lang.tr(Text::ShellKeys),
        base().bg(SIDE).fg(BLUE),
    );
    true
}

fn sidebar(buf: &mut Buffer, app: &mut App, area: Rect) {
    let mut rows: Vec<(String, Option<Nav>, bool)> = vec![
        (app.lang.tr(Text::Overview).into(), Some(Nav::Home), false),
        (
            format!(
                "{}  {}",
                app.lang.tr(Text::CrumbNeedsYou),
                app.fleet.needs_you().len()
            ),
            Some(Nav::Attention),
            false,
        ),
        (String::new(), None, false),
        (app.lang.tr(Text::Teams).into(), None, false),
    ];
    for team in &app.fleet.catalog.teams {
        let closed = app.shell.collapsed.contains(team);
        let agents: Vec<_> = app.fleet.agents_of(team).collect();
        rows.push((
            format!(
                "{} {}  {}",
                if closed { "▸" } else { "▾" },
                team,
                agents.len()
            ),
            Some(Nav::Team(team.clone())),
            true,
        ));
        if !closed {
            if agents.is_empty() {
                rows.push((app.lang.tr(Text::TeamNoAgents).into(), None, false));
            }
            for agent in agents {
                rows.push((
                    format!("  {} {}", glyph(app.fleet.agent_state(agent)), agent.id),
                    Some(Nav::Agent(agent.id.clone())),
                    false,
                ));
            }
        }
        rows.push((String::new(), None, false));
    }
    let orphans: Vec<_> = app
        .fleet
        .catalog
        .agents
        .iter()
        .filter(|a| !app.fleet.catalog.teams.contains(&a.team_id))
        .collect();
    if !orphans.is_empty() {
        rows.push((app.lang.tr(Text::Ungrouped).into(), None, true));
        for agent in orphans {
            rows.push((
                format!("  {} {}", glyph(app.fleet.agent_state(agent)), agent.id),
                Some(Nav::Agent(agent.id.clone())),
                false,
            ));
        }
    }
    let current = app.current_nav();
    let cursor = if app.shell.focused {
        app.shell.selected.as_ref().unwrap_or(&current)
    } else {
        &current
    };
    let index = rows
        .iter()
        .position(|(_, n, _)| n.as_ref() == Some(cursor))
        .unwrap_or(0);
    let height = area.height.saturating_sub(4) as usize;
    let offset = (index + 1).saturating_sub(height);
    for (i, (label, nav, header)) in rows.iter().skip(offset).take(height).enumerate() {
        let rect = row_rect(1, 2 + i as u16, 23);
        let focused = app.shell.focused && nav.as_ref() == Some(cursor);
        let active = nav.as_ref() == Some(&current);
        let style = base()
            .bg(if focused {
                SELECT
            } else if *header {
                HEADER
            } else {
                SIDE
            })
            .fg(if focused || active {
                BLUE
            } else if nav.is_none() {
                MUTED
            } else {
                FG
            });
        buf.set_style(rect, style);
        text(
            buf,
            rect,
            &format!("{}{}", if active { "›" } else { " " }, label),
            style,
        );
        if let Some(Nav::Agent(id)) = nav
            && let Some(agent) = app.fleet.agent(id)
        {
            let state = app.fleet.agent_state(agent);
            let color = match state {
                AgentState::Working => GREEN,
                AgentState::NeedsYou | AgentState::Stuck => AMBER,
                AgentState::Failed => RED,
                _ => MUTED,
            };
            text(buf, row_rect(4, rect.y, 1), glyph(state), style.fg(color));
        }
        if let Some(nav) = nav {
            app.shell.hits.push((rect, Hit::Nav(nav.clone())));
        }
    }
    text(
        buf,
        row_rect(2, area.height - 1, 22),
        &format!("● {}", app.lang.tr(Text::Connected)),
        base().bg(SIDE).fg(MUTED),
    );
}
fn glyph(state: AgentState) -> &'static str {
    match state {
        AgentState::Working => "●",
        AgentState::Idle => "○",
        AgentState::NeedsYou => "!",
        AgentState::Failed => "×",
        AgentState::Stuck => "!",
        _ => "◌",
    }
}

fn paint_row(buf: &mut Buffer, rect: Rect, row: &Row, selected: bool, focus: bool) {
    let rule = FRAME_FILL.contains(&row.fill);
    let style = base()
        .bg(if selected {
            SELECT
        } else if rule {
            HEADER
        } else {
            BG
        })
        .fg(if selected && focus {
            BLUE
        } else if row.dim {
            MUTED
        } else {
            FG
        });
    let style = if row.bold || rule {
        style.bold()
    } else {
        style
    };
    buf.set_style(rect, style);
    let mut clean = row.clone();
    clean.border.clear();
    if rule {
        clean.fill = ' ';
    }
    let (_, head, pad, right) = clean.segments(rect.width as usize, selected);
    text(buf, rect, &format!("{head}{pad}{right}"), style);
}

fn attention(buf: &mut Buffer, app: &mut App, main: Rect) {
    let items = app.fleet.needs_you();
    if items.is_empty() {
        text(buf, main, app.lang.tr(Text::NothingNeedsYou), base());
        return;
    }
    let key = match &app.view().selected {
        Some(Target::Item(k) | Target::Choice(k, _)) => k.clone(),
        _ => items[0].key(),
    };
    let index = items.iter().position(|a| a.key() == key).unwrap_or(0);
    let list_width = 25;
    let detail = Rect::new(
        main.x + list_width + 2,
        main.y,
        main.width - list_width - 2,
        main.height,
    );
    let list_height = main.height.saturating_sub(2) as usize;
    let visible = (list_height / 3).max(1);
    let offset = (index + 1).saturating_sub(visible);
    text(
        buf,
        row_rect(main.x, main.y, list_width),
        app.lang.tr(Text::CrumbNeedsYou),
        base().fg(AMBER),
    );
    for (i, item) in items.iter().skip(offset).take(visible).enumerate() {
        let y = main.y + 2 + (i * 3) as u16;
        let rect = Rect::new(main.x, y, list_width, 2);
        let selected = item.key() == key;
        let style = base()
            .bg(if selected { SELECT } else { BG })
            .fg(if selected { BLUE } else { FG });
        buf.set_style(rect, style);
        text(
            buf,
            rect,
            &format!("{} {}", if selected { "›" } else { " " }, item.question().0),
            style,
        );
        text(
            buf,
            row_rect(main.x + 2, y + 1, list_width - 2),
            &format!(
                "{} / {}",
                app.fleet.item_agent(item).unwrap_or_else(|| "—".into()),
                item.task_id().unwrap_or("—")
            ),
            style.fg(MUTED),
        );
        app.shell
            .hits
            .push((rect, Hit::Row(Target::Item(item.key()))));
    }
    for y in main.y..main.y + main.height {
        text(buf, row_rect(detail.x - 2, y, 1), "│", base().fg(MUTED));
    }
    text(
        buf,
        row_rect(detail.x, detail.y, detail.width),
        &format!("{} / {}", index + 1, items.len()),
        base().fg(MUTED),
    );
    let ctx = crate::app::Ctx {
        fleet: &app.fleet,
        lang: app.lang,
        read: &app.read,
        selected: app.view().selected.as_ref(),
    };
    let item = items[index];
    let mut rows = vec![Row::line("", item.question().0).bold(true), Row::blank()];
    rows.extend(crate::attention::details(&ctx, item));
    let mut wrapped = Vec::new();
    for row in rows {
        for line in wrap(
            row.text.trim_start(),
            detail.width.saturating_sub(2) as usize,
        ) {
            wrapped.push(Row {
                text: line,
                border: String::new(),
                ..row.clone()
            });
        }
    }
    let height = detail.height.saturating_sub(4) as usize;
    if app.shell.detail_key.as_ref() != Some(&key) {
        app.shell.detail_offset = 0;
        app.shell.detail_key = Some(key);
    }
    // Reconcile against wrapping and viewport changes as well as selection.
    // Manual scrolling remains independent while that anchor is unchanged.
    let size = (detail.width, detail.height);
    let choice_row = match app.view().selected {
        Some(Target::Choice(_, _)) => wrapped.iter().position(|r| r.target == app.view().selected),
        _ => None,
    };
    if (app.shell.detail_selection != app.view().selected
        || app.shell.detail_size != size
        || app.shell.detail_choice_row != choice_row)
        && let Some(position) = choice_row
    {
        if position < app.shell.detail_offset {
            app.shell.detail_offset = position;
        } else if position >= app.shell.detail_offset + height {
            app.shell.detail_offset = position + 1 - height;
        }
    }
    app.shell.detail_selection = app.view().selected.clone();
    app.shell.detail_size = size;
    app.shell.detail_choice_row = choice_row;
    app.shell.detail_offset = app
        .shell
        .detail_offset
        .min(wrapped.len().saturating_sub(height));
    for (i, row) in wrapped
        .iter()
        .skip(app.shell.detail_offset)
        .take(height)
        .enumerate()
    {
        let rect = row_rect(detail.x, detail.y + 2 + i as u16, detail.width);
        paint_row(
            buf,
            rect,
            row,
            row.target.is_some() && row.target == app.view().selected,
            !app.shell.focused,
        );
        if let Some(target) = &row.target {
            app.shell.hits.push((rect, Hit::Row(target.clone())));
        }
    }
    let y = main.y + main.height - 1;
    let prev = row_rect(detail.x, y, detail.width / 2);
    let next = row_rect(
        detail.x + detail.width / 2,
        y,
        detail.width - detail.width / 2,
    );
    text(
        buf,
        prev,
        app.lang.tr(Text::PreviousItem),
        base().fg(if index > 0 { BLUE } else { MUTED }),
    );
    text(
        buf,
        next,
        app.lang.tr(Text::NextItem),
        base().fg(if index + 1 < items.len() { BLUE } else { MUTED }),
    );
    if index > 0 {
        app.shell.hits.push((prev, Hit::Previous));
    }
    if index + 1 < items.len() {
        app.shell.hits.push((next, Hit::Next));
    }
}

/// Wrap by display columns, including newlines and wide characters.
fn wrap(value: &str, width: usize) -> Vec<String> {
    let mut result = Vec::new();
    for paragraph in value.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for c in paragraph.chars() {
            let n = c.width().unwrap_or(0);
            if used + n > width && !line.is_empty() {
                result.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push(c);
            used += n;
        }
        result.push(line);
    }
    result
}
