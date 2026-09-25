//! Task Detail: the task's team, repo and holder, its stages as a tree with
//! each stage's state and agent, the needs-you items on it, and its recent
//! events. The repo appears only here.
//!
//! Must NOT: offer actions the caller's identity is not allowed to take.

use crate::app::{Ctx, Target};
use crate::home::needs_you_row;
use crate::i18n::Text;
use crate::source::{StageState, event_task};
use crate::ui::{Row, stage_bar, task_status};

pub fn rows(ctx: &Ctx, task_id: &str) -> Vec<Row> {
    let Some(task) = ctx.fleet.task(task_id) else {
        return vec![Row::line("", task_id)];
    };
    let (glyph, status) = task_status(ctx.fleet, ctx.lang, task);
    let repo = task.repo.as_deref().unwrap_or(ctx.tr(Text::NoRepo));
    let holder = task.holder.as_deref().unwrap_or("—");
    let done = task.stages_done().to_string();
    let total = task.stages.len().to_string();
    let mut rows = vec![
        Row::line("", &task.title).bold(true),
        Row::line(
            "",
            ctx.fmt(Text::TaskMeta, &[&task.team_id, repo, &task.id, holder]),
        ),
        Row::line(
            "",
            ctx.fmt(
                Text::TaskStatus,
                &[&format!("{glyph} {status}"), &done, &total],
            ),
        )
        .right(stage_bar(task)),
        Row::blank(),
        Row::rule(ctx.tr(Text::Stages)),
    ];
    let last = task.stages.len().saturating_sub(1);
    for (i, stage) in task.stages.iter().enumerate() {
        let (glyph, state) = match stage.state {
            StageState::Done => ("✓", ctx.tr(Text::StageDone)),
            StageState::Running => ("●", ctx.tr(Text::StageRunning)),
            StageState::NotStarted => ("·", ctx.tr(Text::StageNotStarted)),
        };
        let branch = if i == last { "  └─" } else { "  ├─" };
        let agent = stage.agent.as_deref();
        rows.push(
            Row::item(
                branch,
                format!(
                    "{glyph} {}. {} ({})",
                    i + 1,
                    stage.name,
                    stage.kind.as_str()
                ),
                Target::Stage(task.id.clone(), i),
            )
            .right(format!("{state}  {}", agent.unwrap_or("")))
            .agent(agent),
        );
    }
    let items: Vec<_> = ctx
        .fleet
        .needs_you()
        .into_iter()
        .filter(|a| a.task_id() == Some(task_id))
        .collect();
    rows.push(Row::blank());
    rows.push(Row::rule(ctx.tr(Text::CrumbNeedsYou)));
    if items.is_empty() {
        rows.push(Row::line("", ctx.tr(Text::NoneYet)));
    }
    for item in items {
        rows.push(needs_you_row(ctx, item));
    }
    rows.push(Row::blank());
    rows.push(Row::rule(ctx.tr(Text::RecentEvents)));
    let events: Vec<_> = ctx
        .fleet
        .events()
        .iter()
        .rev()
        .filter(|e| event_task(&e.event) == Some(task_id))
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
