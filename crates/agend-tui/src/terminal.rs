//! Attach view of a single agent's terminal, opened with `t` from any row
//! that has an agent: the holder's screen, kept current by the app (gate 11
//! B P5), from the left edge, following its last non-blank row (T19; no
//! resize, columns past the edge are cut). The
//! title says whether it is live, the last screen of a stopped agent, ended
//! (retrying), or taking input; while typing the frame is highlighted.
//! Split panes are out of scope for v2.0.
//!
//! Must NOT: connect to a holder directly.

use agend_core::protocol::client::DaemonEvent;

use crate::app::{Ctx, Term, TermMode};
use crate::i18n::Text;
use crate::ui::Row;

pub fn rows(ctx: &Ctx, agent: &str, screen: &str, term: Option<&Term>) -> Vec<Row> {
    let title = match term {
        Some(t) if t.typing => Text::TerminalTyping,
        Some(t) if t.mode == TermMode::Stopped => Text::TerminalStopped,
        Some(t) if t.mode == TermMode::Ended => Text::TerminalEnded,
        _ => Text::TerminalLive,
    };
    let typing = term.is_some_and(|t| t.typing);
    let mut rows = vec![Row::rule(&ctx.fmt(title, &[agent])).accent(typing)];
    // The holder sends every row; blank rows after the last output are not
    // shown, so following the end shows the newest output (T19).
    let lines: Vec<&str> = screen.lines().collect();
    let used = lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map_or(0, |i| i + 1);
    rows.extend(
        lines[..used]
            .iter()
            .map(|line| Row::line("│", *line).accent(typing)),
    );
    rows
}

/// One line describing an event, for "recent events" lists. Event
/// summaries come from the daemon and are shown as they are.
pub fn describe(event: &DaemonEvent) -> String {
    match event {
        DaemonEvent::AttentionRequired { data } => match &data.ask {
            Some(ask) => format!("ask {} opened", ask.ask_id),
            None => data.reason.clone(),
        },
        DaemonEvent::TaskChanged { data } => format!("{}: {}", data.task_id, data.summary),
        DaemonEvent::MessageReceived { data } => format!("message from {}", data.from),
        DaemonEvent::InstanceChanged { data } => format!("{}: {}", data.instance_id, data.summary),
        DaemonEvent::AskUpdated { data } => format!("ask {} updated", data.ask_id),
        DaemonEvent::AttentionResolved { data } => {
            format!("{} resolved: {}", data.attention_id, data.action.as_str())
        }
        DaemonEvent::Unknown => "unknown event".into(),
    }
}
