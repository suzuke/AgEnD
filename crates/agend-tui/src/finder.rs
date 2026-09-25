//! `/` quick jump over agents, tasks and teams: case-insensitive substring
//! match on ids and titles. An overlay, not a level: `←` from what it
//! opened returns to where `/` was pressed.
//!
//! Must NOT: open anything itself (the app does, like any other key).

use crate::app::{Finder, Target};
use crate::i18n::Language;
use crate::source::Fleet;
use crate::ui::Row;

pub fn rows(fleet: &Fleet, _lang: Language, finder: &Finder) -> Vec<Row> {
    let query = finder.query.to_lowercase();
    let hit = |fields: &[&str]| fields.iter().any(|f| f.to_lowercase().contains(&query));
    let mut rows = Vec::new();
    for agent in &fleet.catalog.agents {
        if hit(&[&agent.id]) {
            let context = format!(
                "{} · {}",
                agent.team_id,
                agent.task_id.as_deref().unwrap_or("—")
            );
            rows.push(
                Row::item(
                    "",
                    format!("@ {}", agent.id),
                    Target::Agent(agent.id.clone()),
                )
                .right(context),
            );
        }
    }
    for task in &fleet.catalog.tasks {
        if hit(&[&task.id, &task.title]) {
            rows.push(
                Row::item(
                    "",
                    format!("# {} {}", task.id, task.title),
                    Target::Task(task.id.clone()),
                )
                .right(task.team_id.clone()),
            );
        }
    }
    for team in &fleet.catalog.teams {
        if hit(&[team]) {
            rows.push(Row::item(
                "",
                format!("⌂ {team}"),
                Target::Team(team.clone()),
            ));
        }
    }
    rows
}
