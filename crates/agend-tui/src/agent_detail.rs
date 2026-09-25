//! Agent Detail: state as the source reports it, backend, current task and
//! recent events; `t` opens its terminal, `→`/`Enter` its current task.
//!
//! Must NOT: derive status itself; it shows what the daemon reports.

use agend_core::protocol::client::DaemonEvent;

use crate::app::{Ctx, Target};
use crate::i18n::Text;
use crate::ui::{Row, state_label};

pub fn rows(ctx: &Ctx, agent_id: &str) -> Vec<Row> {
    let Some(agent) = ctx.fleet.agent(agent_id) else {
        return vec![Row::line("", agent_id)];
    };
    let mut rows = vec![
        Row::line("", &agent.id).bold(true),
        Row::line(
            "",
            ctx.fmt(Text::AgentMeta, &[&agent.team_id, agent.backend.as_str()]),
        ),
        Row::line(
            "",
            ctx.fmt(Text::AgentStateLine, &[&state_label(ctx.lang, agent.state)]),
        ),
        Row::blank(),
    ];
    match agent.task_id.as_deref() {
        Some(id) => {
            let title = ctx.fleet.task(id).map_or("", |t| t.title.as_str());
            rows.push(
                Row::item(
                    "",
                    ctx.fmt(Text::CurrentTask, &[id, title]),
                    Target::Task(id.to_owned()),
                )
                .agent(Some(&agent.id)),
            );
        }
        None => rows.push(Row::line("", ctx.tr(Text::NoCurrentTask))),
    }
    rows.push(Row::line("", ctx.tr(Text::ViewTerminal)).dim());
    rows.push(Row::blank());
    rows.push(Row::rule(ctx.tr(Text::RecentEvents)));
    let events: Vec<_> = ctx
        .fleet
        .events()
        .iter()
        .rev()
        .filter(|e| match &e.event {
            DaemonEvent::InstanceChanged { data } => data.instance_id == agent_id,
            DaemonEvent::AttentionRequired { data } => {
                data.ask.as_ref().is_some_and(|ask| ask_from(ask, agent_id))
            }
            DaemonEvent::AskUpdated { data } => ask_from(data, agent_id),
            _ => false,
        })
        .collect();
    if events.is_empty() {
        rows.push(Row::line("", ctx.tr(Text::NoneYet)));
    }
    for event in events {
        rows.push(
            Row::line(
                "",
                format!(
                    "· #{} {}",
                    event.event_id,
                    crate::terminal::describe(&event.event)
                ),
            )
            .dim(),
        );
    }
    rows
}

fn ask_from(ask: &agend_core::protocol::ask::AskThread, agent: &str) -> bool {
    ask.entries.iter().any(|e| {
        matches!(e, agend_core::protocol::ask::AskEntry::Question { from, .. } if from == agent)
    })
}
