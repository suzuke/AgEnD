//! Stage nested ask tags as well as response tags. Core's derived AskEntry
//! would otherwise build an unchecked Content tree for its options vector.

use super::{data, parse};
use agend_core::protocol::{ask::*, client::*};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::{io, time::Instant};

#[derive(Deserialize)]
struct Reply {
    request_id: String,
    fleet: View,
}

#[derive(Deserialize)]
struct View {
    #[serde(default)]
    read_keys: Vec<String>,
    as_of_event_id: u64,
    teams: Vec<TeamView>,
    tasks: Vec<TaskView>,
    instances: Vec<InstanceView>,
    attention: Vec<Attention>,
}

#[derive(Deserialize)]
struct Attention {
    reason: String,
    task_id: Option<String>,
    #[serde(default)]
    ask: Option<Box<RawValue>>,
    #[serde(default)]
    recap: Option<ContextRecap>,
    #[serde(default)]
    attention_id: Option<String>,
    #[serde(default)]
    unblocks: Option<u32>,
    #[serde(default)]
    waiting_since_unix_ms: Option<u64>,
    #[serde(default)]
    if_ignored: Option<String>,
    #[serde(default)]
    actions: Vec<AttentionAction>,
    #[serde(default)]
    instance_id: Option<String>,
}

#[derive(Deserialize)]
struct Thread {
    ask_id: String,
    task_id: Option<String>,
    entries: Vec<Box<RawValue>>,
}

#[derive(Deserialize)]
struct Tag {
    entry: String,
}

#[derive(Deserialize)]
struct Question {
    from: String,
    text: String,
    #[serde(default)]
    options: Vec<String>,
}

#[derive(Deserialize)]
struct Answer {
    from: String,
    source: AnswerSource,
    reply: Box<RawValue>,
}

#[derive(Deserialize)]
struct Resolution {
    from: String,
    summary: String,
}

#[derive(Deserialize)]
struct ReplyTag {
    reply: String,
}

#[derive(Deserialize)]
struct Choice {
    option: String,
}

#[derive(Deserialize)]
struct Text {
    text: String,
}

fn answer(bytes: &[u8], deadline: Instant) -> io::Result<AskReply> {
    let tag: ReplyTag = parse(bytes, deadline)?;
    Ok(match tag.reply.as_str() {
        "choice" => AskReply::Choice {
            option: parse::<Choice>(bytes, deadline)?.option,
        },
        "text" => AskReply::Text {
            text: parse::<Text>(bytes, deadline)?.text,
        },
        _ => AskReply::Unknown,
    })
}

fn entry(bytes: &[u8], deadline: Instant) -> io::Result<AskEntry> {
    let tag: Tag = parse(bytes, deadline)?;
    Ok(match tag.entry.as_str() {
        "question" | "follow_up" => {
            let question: Question = parse(bytes, deadline)?;
            if tag.entry == "question" {
                AskEntry::Question {
                    from: question.from,
                    text: question.text,
                    options: question.options,
                }
            } else {
                AskEntry::FollowUp {
                    from: question.from,
                    text: question.text,
                    options: question.options,
                }
            }
        }
        "answer" => {
            let value: Answer = parse(bytes, deadline)?;
            AskEntry::Answer {
                from: value.from,
                source: value.source,
                reply: answer(value.reply.get().as_bytes(), deadline)?,
            }
        }
        "resolution" => {
            let value: Resolution = parse(bytes, deadline)?;
            AskEntry::Resolution {
                from: value.from,
                summary: value.summary,
            }
        }
        _ => AskEntry::Unknown,
    })
}

fn thread(value: &RawValue, deadline: Instant) -> io::Result<AskThread> {
    let value: Thread = parse(value.get().as_bytes(), deadline)?;
    let mut entries = Vec::new();
    for raw in value.entries {
        entries.push(entry(raw.get().as_bytes(), deadline)?);
    }
    Ok(AskThread {
        ask_id: value.ask_id,
        task_id: value.task_id,
        entries,
    })
}

pub(super) fn decode(value: Option<Box<RawValue>>, deadline: Instant) -> io::Result<FleetData> {
    let value: Reply = data(value, deadline)?;
    let mut attention = Vec::new();
    for item in value.fleet.attention {
        super::super::remaining(deadline)?;
        attention.push(AttentionRequiredData {
            reason: item.reason,
            task_id: item.task_id,
            ask: item.ask.map(|raw| thread(&raw, deadline)).transpose()?,
            recap: item.recap,
            attention_id: item.attention_id,
            unblocks: item.unblocks,
            waiting_since_unix_ms: item.waiting_since_unix_ms,
            if_ignored: item.if_ignored,
            actions: item.actions,
            instance_id: item.instance_id,
        });
    }
    Ok(FleetData {
        request_id: value.request_id,
        fleet: FleetView {
            read_keys: value.fleet.read_keys,
            as_of_event_id: value.fleet.as_of_event_id,
            teams: value.fleet.teams,
            tasks: value.fleet.tasks,
            instances: value.fleet.instances,
            attention,
        },
    })
}
