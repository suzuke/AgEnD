//! The thread history as the driver's event log (gate 7 P7), and which user
//! message is which of our messages (P5). Pure functions over what
//! `thread/turns/list` returns and the `messages` rows.
//!
//! Each turn, in order, becomes `BusyChanged{true}`, one `MessageConfirmed`
//! per user message that is one of ours, and, once the turn ended,
//! `TurnCompleted` (summary: the turn's status) and `BusyChanged{false}`; a
//! running turn stops after what it has so far. A cursor is
//! `<turn id>:<slot>`: slot 0 is the start, slot `k + 1` item `k` of the
//! turn, then the two end slots. Slots are fixed by the turn's items, not by
//! which of them are ours, so a cursor stays valid when a message is matched
//! later; re-reading from a cursor only grows (DRV-7).
//!
//! Matching a user message to a row (P5): its `clientId` equals the message
//! id. An explicit foreign clientId never falls back to text. Without an id,
//! the same turn id and text can match; a queued crash-window row with no
//! turn id can match only once a send was actually attempted. Never-attempted
//! queued text is not delivery evidence. Input-enabled instances require the
//! daemon's own clientId and never use text fallback. Each row matches at most
//! one item.
//!
//! Must NOT: do I/O.

use agend_core::model::DeliveryState;
use agend_core::traits::{DriverEvent, DriverEventKind};
use serde_json::Value;

use crate::delivery::render;
use crate::store::Message;

/// Validate a native history page before treating it as an idle observation.
pub fn turn_page(page: &Value) -> Result<(&[Value], Option<&str>), String> {
    let data = page["data"]
        .as_array()
        .ok_or("thread/turns/list omitted its data array")?;
    let cursor = match page.get("nextCursor") {
        Some(Value::Null) => None,
        Some(Value::String(cursor)) if !cursor.is_empty() => Some(cursor.as_str()),
        _ => return Err("thread/turns/list omitted or malformed nextCursor".into()),
    };
    Ok((data, cursor))
}

/// Only explicit terminal statuses prove that no returned turn is running.
/// Unknown/malformed statuses fail closed; an empty newly started thread is idle.
pub fn all_turns_terminal(turns: &[Value]) -> bool {
    turns.iter().all(|turn| {
        turn["id"].as_str().is_some_and(|id| !id.is_empty())
            && matches!(
                turn["status"].as_str(),
                Some("completed" | "failed" | "interrupted")
            )
    })
}

/// One user message of the thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserItem {
    pub turn_id: String,
    pub client_id: Option<String>,
    pub text: String,
}

impl UserItem {
    /// A `userMessage` item of `turn_id`; `None` for any other item.
    pub fn from_item(turn_id: &str, item: &Value) -> Option<UserItem> {
        if item["type"] != "userMessage" {
            return None;
        }
        let text = item["content"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        Some(UserItem {
            turn_id: turn_id.to_owned(),
            client_id: item["clientId"].as_str().map(str::to_owned),
            text,
        })
    }
}

/// The text a row was sent as.
pub fn text_of(row: &Message) -> String {
    render(&row.from_instance, row.task_id.as_deref(), &row.body)
}

/// For each item, the id of the row it is (see the module doc). `rows` is
/// every row to the instance, in `seq` order; `failed` rows never match.
pub fn match_items(items: &[UserItem], rows: &[Message]) -> Vec<Option<String>> {
    match_with(items, rows, true)
}

/// Input-enabled instances require the daemon's own client id as evidence.
/// Manual text cannot resolve a lost send reply, even after a recorded attempt.
pub fn match_identified_items(items: &[UserItem], rows: &[Message]) -> Vec<Option<String>> {
    match_with(items, rows, false)
}

fn match_with(items: &[UserItem], rows: &[Message], allow_text: bool) -> Vec<Option<String>> {
    let texts: Vec<String> = rows.iter().map(text_of).collect();
    let mut taken = vec![false; rows.len()];
    items
        .iter()
        .map(|item| {
            let usable = |i: usize| !taken[i] && rows[i].state != DeliveryState::Failed;
            let by_client = item
                .client_id
                .as_deref()
                .and_then(|c| (0..rows.len()).find(|&i| usable(i) && rows[i].id == c));
            let found = by_client.or_else(|| {
                // Human/frontend client IDs are distinct from daemon IDs.
                // Their text must not steal a daemon message attribution.
                if !allow_text || item.client_id.is_some() {
                    return None;
                }
                (0..rows.len()).find(|&i| {
                    usable(i)
                        && texts[i] == item.text
                        && match rows[i].turn_id.as_deref() {
                            Some(turn) => turn == item.turn_id,
                            None => {
                                rows[i].state == DeliveryState::Queued
                                    && rows[i].attempted_at_unix_ms.is_some()
                            }
                        }
                })
            })?;
            taken[found] = true;
            Some(rows[found].id.clone())
        })
        .collect()
}

/// Every user message of `turns`, with its (turn, item) position.
pub fn user_items(turns: &[Value]) -> Vec<((usize, usize), UserItem)> {
    let mut out = Vec::new();
    for (t, turn) in turns.iter().enumerate() {
        let id = turn["id"].as_str().unwrap_or_default();
        for (k, item) in turn["items"].as_array().into_iter().flatten().enumerate() {
            if let Some(user) = UserItem::from_item(id, item) {
                out.push(((t, k), user));
            }
        }
    }
    out
}

fn ended(turn: &Value) -> bool {
    !matches!(turn["status"].as_str(), None | Some("inProgress"))
}

/// The events of `turns` (P7), given which user messages are ours
/// (`matched[(turn, item)]` = message id).
pub fn expand(turns: &[Value], matched: &[((usize, usize), String)]) -> Vec<DriverEvent> {
    let mut events = Vec::new();
    for (t, turn) in turns.iter().enumerate() {
        let id = turn["id"].as_str().unwrap_or_default();
        let at = |slot: usize| format!("{id}:{slot}");
        events.push(DriverEvent {
            cursor: at(0),
            kind: DriverEventKind::BusyChanged { busy: true },
        });
        let items = turn["items"].as_array().map_or(0, Vec::len);
        for k in 0..items {
            if let Some((_, message)) = matched.iter().find(|(pos, _)| *pos == (t, k)) {
                events.push(DriverEvent {
                    cursor: at(k + 1),
                    kind: DriverEventKind::MessageConfirmed {
                        message_id: message.clone(),
                    },
                });
            }
        }
        if ended(turn) {
            events.push(DriverEvent {
                cursor: at(items + 1),
                kind: DriverEventKind::TurnCompleted {
                    summary: turn["status"].as_str().map(str::to_owned),
                },
            });
            events.push(DriverEvent {
                cursor: at(items + 2),
                kind: DriverEventKind::BusyChanged { busy: false },
            });
        }
    }
    events
}

/// `(turn index, slot)` of `cursor` in `turns`; `None` when its turn is not
/// there (history rewritten, U10) or the cursor is malformed.
fn position(turns: &[Value], cursor: &str) -> Option<(usize, usize)> {
    let (turn, slot) = cursor.rsplit_once(':')?;
    let slot = slot.parse().ok()?;
    let t = turns.iter().position(|x| x["id"] == turn)?;
    Some((t, slot))
}

/// The events after `cursor` (all of them without one, or when its turn is
/// gone: re-read from the start, the caller dedupes by message id).
pub fn after(turns: &[Value], events: Vec<DriverEvent>, cursor: Option<&str>) -> Vec<DriverEvent> {
    let Some(from) = cursor.and_then(|c| position(turns, c)) else {
        return events;
    };
    events
        .into_iter()
        .filter(|e| position(turns, &e.cursor).is_some_and(|at| at > from))
        .collect()
}

/// Read-only execution evidence for an identified, already confirmed message.
/// A failed/interrupted turn may still have confirmed its input.
pub fn message_outcome(
    turns: &[Value],
    row: &Message,
) -> agend_core::protocol::client::MessageOutcomeState {
    use agend_core::protocol::client::MessageOutcomeState as State;
    if row.state != DeliveryState::Confirmed {
        return State::Unknown;
    }
    let Some(id) = row.turn_id.as_deref() else {
        return State::Unknown;
    };
    let matches: Vec<_> = turns
        .iter()
        .filter(|t| t["id"].as_str() == Some(id))
        .collect();
    if matches.len() != 1 {
        return State::Unknown;
    }
    let turn = matches[0];
    let Some(items) = turn["items"].as_array() else {
        return State::Unknown;
    };
    // The isolated queue canary has exactly one input per turn. Multiple
    // inputs make attribution of the assistant response ambiguous.
    let users: Vec<_> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| item["type"] == "userMessage")
        .collect();
    if users.len() != 1
        || !UserItem::from_item(id, users[0].1).is_some_and(|u| {
            u.client_id.as_deref() == Some(row.id.as_str()) && u.text == text_of(row)
        })
    {
        return State::Unknown;
    }
    match turn["status"].as_str() {
        Some("failed" | "interrupted") => State::Failed,
        Some("inProgress") => State::Running,
        Some("completed")
            if turn.get("error") == Some(&Value::Null)
                && items[users[0].0 + 1..].iter().any(|i| {
                    i["type"] == "agentMessage"
                        && i["text"].as_str().is_some_and(|s| !s.trim().is_empty())
                }) =>
        {
            State::Completed
        }
        _ => State::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::policy::busy::BusyLevel;
    use serde_json::json;

    fn row(id: &str, state: DeliveryState, turn: Option<&str>, body: &str) -> Message {
        Message {
            seq: 0,
            id: id.into(),
            from_instance: "operator".into(),
            to_instance: "g7-1".into(),
            task_id: None,
            body: body.into(),
            level: BusyLevel::Queue,
            state,
            turn_id: turn.map(str::to_owned),
            created_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            attempted_at_unix_ms: None,
        }
    }

    fn user(turn: &str, client: Option<&str>, body: &str) -> UserItem {
        UserItem {
            turn_id: turn.into(),
            client_id: client.map(str::to_owned),
            text: render("operator", None, body),
        }
    }

    /// The fallback when codex gives no `clientId` (U2): same turn and text;
    /// two identical texts in one turn go to the two rows in order.
    #[test]
    fn without_a_client_id_the_turn_and_text_decide() {
        use DeliveryState::*;
        let mut crashed = row("m-4", Queued, None, "crash window");
        crashed.attempted_at_unix_ms = Some(0);
        let rows = [
            row("m-1", Sent, Some("t-1"), "hi"),
            row("m-2", Sent, Some("t-1"), "hi"),
            row("m-3", Sent, Some("t-2"), "hi"),
            crashed,
            row("m-5", Failed, Some("t-1"), "hi"),
        ];
        let items = [
            user("t-1", None, "hi"),
            user("t-1", None, "hi"),
            user("t-1", None, "hi"),
            user("t-2", None, "other"),
            user("t-9", None, "crash window"),
            user("t-2", Some("m-3"), "whatever"),
        ];
        assert_eq!(
            match_items(&items, &rows),
            [
                Some("m-1".into()),
                Some("m-2".into()),
                None,
                None,
                Some("m-4".into()),
                Some("m-3".into()),
            ]
        );
    }

    #[test]
    fn manual_text_is_not_evidence_for_a_never_attempted_queued_row() {
        let mut queued = row("m-unsent", DeliveryState::Queued, None, "same text");
        let manual = user("manual-turn", None, "same text");
        assert_eq!(
            match_items(std::slice::from_ref(&manual), std::slice::from_ref(&queued)),
            [None]
        );
        // A lost send reply retains the old no-clientId compatibility path.
        queued.attempted_at_unix_ms = Some(7);
        assert_eq!(match_items(&[manual], &[queued]), [Some("m-unsent".into())]);
    }

    #[test]
    fn explicit_foreign_client_ids_cannot_fall_back_to_a_daemon_rows_text() {
        let sent = row("daemon-id", DeliveryState::Sent, Some("t-1"), "same text");
        let manual = user("t-1", Some("manual-frontend-id"), "same text");
        assert_eq!(match_items(&[manual], std::slice::from_ref(&sent)), [None]);
        let daemon = user("t-1", Some("daemon-id"), "server-rendered text");
        assert_eq!(match_items(&[daemon], &[sent]), [Some("daemon-id".into())]);
    }

    #[test]
    fn input_scope_requires_own_client_ids_even_after_attempt_or_with_known_turn() {
        let mut attempted = row("queued", DeliveryState::Queued, None, "same text");
        attempted.attempted_at_unix_ms = Some(7);
        let rows = [
            attempted,
            row("sent", DeliveryState::Sent, Some("t-1"), "same text"),
        ];
        let items = [
            user("t-1", None, "same text"),
            user("t-1", Some("foreign"), "same text"),
            user("t-1", Some("queued"), "same text"),
            user("t-1", Some("queued"), "same text"),
            user("t-1", Some("sent"), "same text"),
        ];
        assert_eq!(
            match_identified_items(&items, &rows),
            [None, None, Some("queued".into()), None, Some("sent".into())]
        );
    }

    /// Cursors are fixed by slot: matching a message later adds an event
    /// but no cursor changes; `after` a cursor is only newer slots.
    #[test]
    fn cursors_are_slots_so_later_matches_only_add_events() {
        let turns = [
            json!({"id": "t-1", "status": "completed", "items": [
                {"type": "userMessage", "clientId": null, "content": [{"type": "text", "text": "x"}]},
                {"type": "agentMessage", "text": "y"}]}),
            json!({"id": "t-2", "status": "inProgress", "items": [
                {"type": "userMessage", "clientId": "m-2", "content": [{"type": "text", "text": "z"}]}]}),
        ];
        let none = expand(&turns, &[((1, 0), "m-2".into())]);
        let cursors: Vec<&str> = none.iter().map(|e| e.cursor.as_str()).collect();
        assert_eq!(cursors, ["t-1:0", "t-1:3", "t-1:4", "t-2:0", "t-2:1"]);
        let more = expand(&turns, &[((0, 0), "m-1".into()), ((1, 0), "m-2".into())]);
        let cursors: Vec<&str> = more.iter().map(|e| e.cursor.as_str()).collect();
        assert_eq!(
            cursors,
            ["t-1:0", "t-1:1", "t-1:3", "t-1:4", "t-2:0", "t-2:1"]
        );
        let newer = after(&turns, more.clone(), Some("t-1:3"));
        let cursors: Vec<&str> = newer.iter().map(|e| e.cursor.as_str()).collect();
        assert_eq!(cursors, ["t-1:4", "t-2:0", "t-2:1"]);
        assert_eq!(after(&turns, more.clone(), Some("gone:3")), more);
    }
}
