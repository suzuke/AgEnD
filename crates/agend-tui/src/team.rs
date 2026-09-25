//! Team page with three tabs: goals, agents, pipeline. Tabs switch with
//! `1`/`2`/`3` or Tab/Shift-Tab, never with `←`/`→` (those are levels).
//!
//! Must NOT: mix members of other teams.

use agend_core::pipeline::stage::StageKind;

use crate::app::{Ctx, Tab, Target};
use crate::home::team_counts;
use crate::i18n::Text;
use crate::ui::{Row, goal_row, state_label};

pub fn rows(ctx: &Ctx, team: &str, tab: Tab) -> Vec<Row> {
    let tabs = [
        (Tab::Goals, Text::TabGoals),
        (Tab::Agents, Text::TabAgents),
        (Tab::Pipeline, Text::TabPipeline),
    ]
    .iter()
    .map(|(t, text)| {
        if *t == tab {
            format!("[{}]", ctx.tr(*text))
        } else {
            format!(" {} ", ctx.tr(*text))
        }
    })
    .collect::<Vec<_>>()
    .join("  ");
    let mut rows = vec![
        Row::line("", team).right(team_counts(ctx, team)).bold(true),
        Row::line("", tabs),
        Row::separator(),
    ];
    match tab {
        Tab::Goals => goals(ctx, team, &mut rows),
        Tab::Agents => agents(ctx, team, &mut rows),
        Tab::Pipeline => pipeline(ctx, team, &mut rows),
    }
    rows
}

fn goals(ctx: &Ctx, team: &str, rows: &mut Vec<Row>) {
    let before = rows.len();
    // Active goals first, then finished ones.
    let (active, done): (Vec<_>, Vec<_>) = ctx.fleet.tasks_of(team).partition(|t| !t.is_done());
    for task in active.into_iter().chain(done) {
        rows.push(goal_row(ctx.fleet, ctx.lang, "", task));
    }
    if rows.len() == before {
        rows.push(Row::line("", ctx.tr(Text::TeamNoGoals)));
    }
}

fn agents(ctx: &Ctx, team: &str, rows: &mut Vec<Row>) {
    let members: Vec<_> = ctx.fleet.agents_of(team).collect();
    if members.is_empty() {
        rows.push(Row::line("", ctx.tr(Text::TeamNoAgents)));
        return;
    }
    let header = format!(
        "{:<13}{:<12}{:<10}{}",
        ctx.tr(Text::ColState),
        ctx.tr(Text::ColName),
        ctx.tr(Text::ColBackend),
        ctx.tr(Text::ColTask)
    );
    rows.push(Row::line("", header).dim());
    for agent in members {
        let task = agent
            .task_id
            .as_deref()
            .map(|id| match ctx.fleet.task(id) {
                Some(task) => format!("{id} {}", task.title),
                None => id.to_owned(),
            })
            .unwrap_or_else(|| "—".into());
        let state = state_label(ctx.lang, ctx.fleet.agent_state(agent));
        rows.push(
            Row::item(
                "",
                format!(
                    "{}{:<12}{:<10}{task}",
                    pad(&state, 13),
                    agent.id,
                    agent.backend.as_str()
                ),
                Target::Agent(agent.id.clone()),
            )
            .agent(Some(&agent.id)),
        );
    }
}

/// Tasks grouped by the kind of their current stage, in workflow order,
/// then finished tasks.
fn pipeline(ctx: &Ctx, team: &str, rows: &mut Vec<Row>) {
    let order = [
        StageKind::Work,
        StageKind::Submit,
        StageKind::Command,
        StageKind::Approval,
        StageKind::Merge,
        StageKind::Fanout,
    ];
    let tasks: Vec<_> = ctx.fleet.tasks_of(team).collect();
    if tasks.is_empty() {
        rows.push(Row::line("", ctx.tr(Text::TeamNoGoals)));
        return;
    }
    let column = |kind: Option<StageKind>| -> Vec<_> {
        tasks
            .iter()
            .filter(|t| t.current_stage().map(|i| t.stages[i].kind) == kind)
            .collect()
    };
    let columns = order
        .iter()
        .map(|k| (k.as_str(), column(Some(*k))))
        .chain([(ctx.tr(Text::PipelineDone), column(None))]);
    for (name, tasks) in columns {
        if tasks.is_empty() {
            continue;
        }
        rows.push(Row::line("", format!("── {name} ({})", tasks.len())).dim());
        for task in tasks {
            let agent = task
                .current_stage()
                .and_then(|i| task.stages[i].agent.as_deref())
                .or(task.holder.as_deref());
            rows.push(
                Row::item(
                    "",
                    format!("  {} {}", task.id, task.title),
                    Target::Task(task.id.clone()),
                )
                .right(agent.unwrap_or("—"))
                .agent(agent),
            );
        }
    }
}

/// Pad to `width` display columns.
fn pad(s: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    format!("{s}{}", " ".repeat(width.saturating_sub(s.width())))
}
