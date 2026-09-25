//! "Needs you" items. "Read" and "resolved" are separate: viewing only drops
//! the bold; choosing an option (or answering in free text) sends the answer
//! to the daemon, and the item leaves the list when the daemon's
//! `ask_updated` event shows the question answered.
//!
//! The selected item expands to show who asks about which task, what happens
//! if it is left alone (DEMO-01 §4B), the context recap (D37), the
//! conversation so far (D35) and the options.
//!
//! Must NOT: resolve an item just because it was viewed.

use agend_core::protocol::ask::{AnswerSource, AskEntry, AskReply};

use crate::app::{Ctx, Target};
use crate::home::{asker_or_holder, needs_you_row, team_of};
use crate::i18n::Text;
use crate::source::Attention;
use crate::ui::Row;

const BAR: &str = "┃ ";

pub fn rows(ctx: &Ctx) -> Vec<Row> {
    let items = ctx.fleet.needs_you();
    let mut rows = vec![
        Row::rule(&ctx.fmt(Text::NeedsYouOpen, &[&items.len().to_string()])),
        Row::blank(),
    ];
    if items.is_empty() {
        rows.push(Row::line("", ctx.tr(Text::NothingNeedsYou)));
        rows.push(Row::blank());
        rows.push(Row::item("", ctx.tr(Text::BackToHome), Target::Home));
        return rows;
    }
    let expanded = match ctx.selected {
        Some(Target::Item(key) | Target::Choice(key, _)) => Some(key.as_str()),
        _ => None,
    };
    for item in items {
        rows.push(needs_you_row(ctx, item));
        if expanded == Some(item.key().as_str()) {
            rows.extend(details(ctx, item));
            rows.push(Row::blank());
        }
    }
    rows
}

fn details(ctx: &Ctx, item: &Attention) -> Vec<Row> {
    let key = item.key();
    let agent = asker_or_holder(ctx.fleet, item);
    let from = agent.clone().unwrap_or_else(|| "—".into());
    let task = item.task_id().unwrap_or("—");
    let team = team_of(ctx.fleet, item).unwrap_or_else(|| ctx.tr(Text::NoTeam).into());
    let line = |text: String| Row::line(BAR, format!("  {text}"));
    let mut rows = vec![line(ctx.fmt(Text::AskFrom, &[&from, task, &team])).dim()];
    if let Some(text) = ctx.fleet.catalog.if_ignored.get(task) {
        rows.push(line(ctx.fmt(Text::IfIgnored, &[text])));
    }
    if let Some(recap) = &item.data.recap {
        rows.push(line(ctx.fmt(Text::RecapGoal, &[&recap.goal])));
        if !recap.decisions.is_empty() {
            let decided = recap.decisions.join("; ");
            rows.push(line(ctx.fmt(Text::RecapDecided, &[&decided])));
        }
        rows.push(line(ctx.fmt(Text::RecapAsking, &[&recap.asking])));
        rows.push(line(ctx.fmt(Text::RecapNext, &[&recap.next])));
    }
    let Some(ask) = &item.data.ask else {
        rows.push(line(ctx.tr(Text::NoAction).into()).dim());
        return rows;
    };
    for entry in &ask.entries {
        let text = match entry {
            AskEntry::Question { from, text, .. } | AskEntry::FollowUp { from, text, .. } => {
                format!("{from}: {text}")
            }
            AskEntry::Answer { source, reply, .. } => {
                let reply = match reply {
                    AskReply::Choice { option } => option.as_str(),
                    AskReply::Text { text } => text.as_str(),
                    AskReply::Unknown => "?",
                };
                ctx.fmt(Text::EntryAnswer, &[source_name(*source), reply])
            }
            AskEntry::Resolution { summary, .. } => ctx.fmt(Text::EntryResolution, &[summary]),
            AskEntry::Unknown => continue,
        };
        rows.push(line(text));
    }
    for (n, option) in item.question().1.iter().enumerate() {
        rows.push(
            Row::item(
                BAR,
                format!("  [{}] {option}", n + 1),
                Target::Choice(key.clone(), n),
            )
            .agent(agent.as_deref()),
        );
    }
    rows.push(line(ctx.tr(Text::AnswerOwnWords).into()).dim());
    rows
}

fn source_name(source: AnswerSource) -> &'static str {
    match source {
        AnswerSource::Tui => "tui",
        AnswerSource::Telegram => "telegram",
        AnswerSource::Cli => "cli",
        AnswerSource::Unknown => "?",
    }
}
