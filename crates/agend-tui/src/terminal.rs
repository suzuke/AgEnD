//! Attach view of a single agent's terminal, opened with `t` from any row
//! that has an agent. For now it shows the protocol's `terminal_snapshot`
//! read-only; live bytes and input (`terminal_bytes`, `terminal_input`) come
//! with gate 11 proper. Split panes are out of scope for v2.0.
//!
//! Must NOT: connect to a holder directly.

use agend_core::protocol::client::DaemonEvent;

use crate::app::Ctx;
use crate::i18n::Text;
use crate::ui::Row;

pub fn rows(ctx: &Ctx, agent: &str, screen: &str) -> Vec<Row> {
    let mut rows = vec![Row::rule(&ctx.fmt(Text::TerminalTitle, &[agent]))];
    rows.extend(screen.lines().map(|line| Row::line("│", line)));
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
        DaemonEvent::Unknown => "unknown event".into(),
    }
}
