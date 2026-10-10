//! Correlate durable ACK, native tool completion and Stop without reading a transcript.
use agend_core::protocol::client::is_uuid_v4;
use agend_core::runtime_records::{ClaudeDelivery, DriverEvent};
use serde_json::Value;

pub(crate) fn completed(delivery: &ClaudeDelivery, events: &[DriverEvent]) -> Option<String> {
    if delivery.abandoned_at_unix_ms.is_some() {
        return None;
    }
    let attempt = delivery.attempt.as_ref()?;
    let confirmed = attempt.confirmed_at_unix_ms?;
    if confirmed < attempt.started_at_unix_ms {
        return None;
    }
    let rows: Vec<_> = events
        .iter()
        .filter(|e| {
            e.event.instance_id == delivery.instance_id
                && e.event.session_id == attempt.session_id
                && !e.event.replayed
                && e.event.occurred_at_unix_ms >= attempt.started_at_unix_ms
                && e.ingested_at_unix_ms >= e.event.occurred_at_unix_ms
                && e.ingested_at_unix_ms - e.event.occurred_at_unix_ms <= 5000
        })
        .filter_map(|e| {
            serde_json::from_str::<Value>(&e.event.payload)
                .ok()
                .map(|v| (e, v))
        })
        .collect();
    let receipt = |v: &Value| {
        v["message_id"] == delivery.message_id
            && v["delivery_id"] == attempt.delivery_id
            && v["session_id"] == attempt.session_id
    };
    let ack: Vec<_> = rows
        .iter()
        .filter(|(e, v)| {
            e.event.kind == "AgendAck" && receipt(v) && e.event.occurred_at_unix_ms == confirmed
        })
        .collect();
    if ack.len() != 1 {
        return None;
    }
    let posts: Vec<_> = rows
        .iter()
        .filter(|(e, v)| {
            e.event.kind == "PostToolUse"
                && v["hook_event_name"] == "PostToolUse"
                && v["session_id"] == attempt.session_id
                && v["tool_name"] == "mcp__agend__agend_ack"
                && e.seq > ack[0].0.seq
                && e.event.occurred_at_unix_ms >= confirmed
                && v["tool_input"]["receipts"]
                    .as_array()
                    .is_some_and(|r| r.len() == 1 && receipt(&r[0]))
        })
        .collect();
    if posts.len() != 1 {
        return None;
    }
    let (post, native) = posts[0];
    let prompt = native["prompt_id"].as_str().filter(|s| is_uuid_v4(s))?;
    if rows.iter().any(|(e, v)| {
        e.event.kind == "PostToolUse"
            && e.seq != post.seq
            && v["tool_name"] == "mcp__agend__agend_ack"
            && v["prompt_id"] == prompt
    }) {
        return None;
    }
    let stops: Vec<_> = rows
        .iter()
        .filter(|(e, v)| e.event.kind == "Stop" && e.seq > post.seq && v["prompt_id"] == prompt)
        .collect();
    if stops.len() != 1 {
        return None;
    }
    let (stop, native) = stops[0];
    if native["session_id"] != attempt.session_id
        || native["hook_event_name"] != "Stop"
        || !native["stop_hook_active"].is_boolean()
        || native["last_assistant_message"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
        || stop.event.occurred_at_unix_ms < post.event.occurred_at_unix_ms
        || events.iter().any(|e| {
            e.event.instance_id == delivery.instance_id
                && e.event.session_id == attempt.session_id
                && matches!(e.event.kind.as_str(), "SessionStart" | "SessionEnd")
                && ((e.seq > ack[0].0.seq && e.seq < stop.seq)
                    || (e.event.occurred_at_unix_ms >= attempt.started_at_unix_ms
                        && e.event.occurred_at_unix_ms <= stop.event.occurred_at_unix_ms))
        })
    {
        return None;
    }
    Some(prompt.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::runtime_records::{ClaudeAttempt, ClaudeRoute, NewDriverEvent};
    fn fixture() -> (ClaudeDelivery, Vec<DriverEvent>) {
        let value: Value =
            serde_json::from_str(include_str!("fixtures/ack-stop-outcome.json")).unwrap();
        let d = &value["delivery"];
        let text = |key: &str| d[key].as_str().unwrap().to_owned();
        let delivery = ClaudeDelivery {
            message_id: text("id"),
            instance_id: text("to_instance"),
            abandoned_at_unix_ms: None,
            abandonment_reason: None,
            attempt: Some(ClaudeAttempt {
                delivery_id: text("delivery_id"),
                session_id: text("session_id"),
                route: ClaudeRoute::parse(d["route"].as_str().unwrap()).unwrap(),
                started_at_unix_ms: d["started_at_unix_ms"].as_u64().unwrap(),
                sent_at_unix_ms: d["sent_at_unix_ms"].as_u64(),
                confirmed_at_unix_ms: d["confirmed_at_unix_ms"].as_u64(),
            }),
        };
        let events = value["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| DriverEvent {
                seq: e["seq"].as_i64().unwrap(),
                ingested_at_unix_ms: e["ingested_at_unix_ms"].as_u64().unwrap(),
                event: NewDriverEvent {
                    id: e["id"].as_str().unwrap().into(),
                    instance_id: e["instance_id"].as_str().unwrap().into(),
                    session_id: e["session_id"].as_str().unwrap().into(),
                    kind: e["kind"].as_str().unwrap().into(),
                    payload: e["payload"].to_string(),
                    occurred_at_unix_ms: e["occurred_at_unix_ms"].as_u64().unwrap(),
                    replayed: e["replayed"] == 1,
                },
            })
            .collect();
        (delivery, events)
    }
    #[test]
    fn captured_ack_tool_and_stop_bind_one_completed_prompt() {
        let (d, events) = fixture();
        assert_eq!(
            completed(&d, &events).as_deref(),
            Some("9a9acc3a-7936-42c6-91da-96feec7f4b9e")
        );
        for omitted in 0..3 {
            let mut changed = events.clone();
            changed.remove(omitted);
            assert!(completed(&d, &changed).is_none());
        }
        for index in 0..3 {
            let mut changed = events.clone();
            changed[index].event.replayed = true;
            assert!(completed(&d, &changed).is_none());
            let mut changed = events.clone();
            changed[index].event.session_id = "foreign".into();
            assert!(completed(&d, &changed).is_none());
        }
        for (index, field, value) in [
            (1, "prompt_id", "foreign"),
            (2, "prompt_id", "foreign"),
            (2, "last_assistant_message", " "),
            (2, "session_id", "foreign"),
        ] {
            let mut changed = events.clone();
            let mut v: Value = serde_json::from_str(&changed[index].event.payload).unwrap();
            v[field] = Value::String(value.into());
            changed[index].event.payload = v.to_string();
            assert!(completed(&d, &changed).is_none());
        }
        let mut changed = events.clone();
        changed[2].event.occurred_at_unix_ms = events[1].event.occurred_at_unix_ms - 1;
        assert!(completed(&d, &changed).is_none());
        let mut changed = events.clone();
        changed.push(events[2].clone());
        assert!(completed(&d, &changed).is_none());
        let mut changed = events.clone();
        let mut payload: Value = serde_json::from_str(&changed[2].event.payload).unwrap();
        payload["stop_hook_active"] = Value::Bool(true);
        changed[2].event.payload = payload.to_string();
        assert!(completed(&d, &changed).is_some());
        let mut changed = events.clone();
        let mut ended = events[1].clone();
        ended.seq += 1;
        ended.event.kind = "SessionEnd".into();
        ended.event.replayed = true;
        changed.push(ended);
        assert!(completed(&d, &changed).is_none());
        for kind in ["SessionStart", "SessionEnd"] {
            let mut changed = events.clone();
            changed[1].seq += 1;
            changed[2].seq += 1;
            let mut barrier = events[0].clone();
            barrier.seq += 1;
            barrier.event.kind = kind.into();
            barrier.event.occurred_at_unix_ms += 1;
            barrier.ingested_at_unix_ms = barrier.event.occurred_at_unix_ms + 6000;
            barrier.event.replayed = true;
            changed.push(barrier);
            assert!(
                completed(&d, &changed).is_none(),
                "{kind} between ACK and tool completion"
            );
        }
        let mut unconfirmed = d.clone();
        unconfirmed.attempt.as_mut().unwrap().confirmed_at_unix_ms = None;
        assert!(completed(&unconfirmed, &events).is_none());
    }
}
