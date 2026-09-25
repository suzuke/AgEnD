//! Home screen: the cross-team "needs you" list on top, then one bordered
//! block per team with its agent-state counts, active goals and latest
//! changes (DEMO-01 round 2 layout).
//!
//! Must NOT: show repos (repo is only in Task Detail).

use agend_core::protocol::client::DaemonEvent;

use crate::app::{Ctx, Target};
use crate::i18n::Text;
use crate::source::{AgentState, Attention, Fleet};
use crate::ui::{Row, goal_row, state_label};

/// How many recent task changes each team block shows.
const RECENT_PER_TEAM: usize = 2;

pub fn rows(ctx: &Ctx) -> Vec<Row> {
    let needs_you = ctx.fleet.needs_you();
    let mut rows = vec![Row::rule(
        &ctx.fmt(Text::NeedsYouN, &[&needs_you.len().to_string()]),
    )];
    if needs_you.is_empty() {
        rows.push(Row::line("", ctx.tr(Text::NothingNeedsYou)));
    }
    for item in &needs_you {
        rows.push(needs_you_row(ctx, item));
    }
    for team in &ctx.fleet.catalog.teams {
        rows.push(Row::blank());
        rows.extend(team_block(ctx, team));
    }
    rows
}

/// `▌ ! question   [new]  team · T-id`, bold until viewed.
pub fn needs_you_row(ctx: &Ctx, item: &Attention) -> Row {
    let key = item.key();
    let unread = !ctx.read.contains(&item.read_key());
    let task = item.task_id().unwrap_or("—");
    let team = team_of(ctx.fleet, item).unwrap_or_else(|| ctx.tr(Text::NoTeam).to_owned());
    let new = if unread { ctx.tr(Text::New) } else { "" };
    Row::item("▌", format!("! {}", item.question().0), Target::Item(key))
        .right(format!("{new}  {team} · {task}"))
        .bold(unread)
        .agent(asker_or_holder(ctx.fleet, item).as_deref())
}

pub fn team_of(fleet: &Fleet, item: &Attention) -> Option<String> {
    let task = fleet.task(item.task_id()?)?;
    Some(task.team_id.clone())
}

/// The agent `t` opens for a needs-you item: who asked, else the task holder.
pub fn asker_or_holder(fleet: &Fleet, item: &Attention) -> Option<String> {
    item.asker()
        .map(str::to_owned)
        .or_else(|| fleet.task(item.task_id()?)?.holder.clone())
}

/// `● 1 working  ! 1 needs you …`, zero counts left out.
pub fn team_counts(ctx: &Ctx, team: &str) -> String {
    let states = [
        AgentState::Working,
        AgentState::NeedsYou,
        AgentState::Stuck,
        AgentState::Idle,
        AgentState::Unknown,
    ];
    states
        .iter()
        .filter_map(|state| {
            let n = ctx
                .fleet
                .agents_of(team)
                .filter(|a| a.state == *state)
                .count();
            let label = state_label(ctx.lang, *state);
            let (glyph, word) = label.split_once(' ').unwrap_or((&label, ""));
            (n > 0).then(|| format!("{glyph} {n} {word}"))
        })
        .collect::<Vec<_>>()
        .join("  ")
}

fn team_block(ctx: &Ctx, team: &str) -> Vec<Row> {
    let mut rows = vec![Row {
        border: "┏".into(),
        marker: true,
        text: format!("{team} "),
        right: format!(" {}", team_counts(ctx, team)),
        fill: '─',
        target: Some(Target::Team(team.to_owned())),
        ..Row::default()
    }];
    let active: Vec<_> = ctx.fleet.tasks_of(team).filter(|t| !t.is_done()).collect();
    if active.is_empty() {
        rows.push(Row::line("┃ ", format!("  {}", ctx.tr(Text::NoActiveGoals))).dim());
    }
    for task in active {
        rows.push(goal_row(ctx.fleet, ctx.lang, "┃ ", task));
    }
    let recent: Vec<_> = ctx
        .fleet
        .events()
        .iter()
        .rev()
        .filter_map(|e| match &e.event {
            DaemonEvent::TaskChanged { data } => {
                let task = ctx.fleet.task(&data.task_id)?;
                (task.team_id == team).then_some((task, data.summary.as_str()))
            }
            _ => None,
        })
        .take(RECENT_PER_TEAM)
        .collect();
    for (task, summary) in recent {
        let glyph = if task.is_done() { "✓" } else { "→" };
        // Not selectable: the task's own goal row (or the team page) opens it,
        // and a second row with the same target would break stable selection.
        rows.push(
            Row::line(
                "┃ ",
                format!("{glyph} {} {summary} — {}", task.id, task.title),
            )
            .dim(),
        );
    }
    rows.push(Row {
        border: "┗".into(),
        fill: '━',
        ..Row::default()
    });
    rows
}
