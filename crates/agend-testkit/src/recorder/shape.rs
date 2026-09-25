//! Shape comparison of two transcripts (real recording vs fake run). This
//! module is the ONE place that defines what "same shape" means; the rules
//! are listed in the testkit README ("conformance check") and here:
//!
//! 1. Streams: messages are split by channel and direction (`via` + `from`),
//!    e.g. `ws/backend`, `http/client`. Order is compared inside a stream,
//!    never across streams (a client request racing a backend event is
//!    timing, not protocol).
//! 2. Kind of a message: JSON-RPC `request <method>`, `notify <method>`,
//!    `result <method>`, `error <method>` (responses are matched to their
//!    request by id); HTTP `<METHOD> <path>` with id segments as `:id`, and
//!    `<status> <request kind>` for the response; SSE the event `type`;
//!    hooks `hook_event_name` (reply: `reply <event>`); keys `text` or the
//!    key name; screen markers the marker name.
//! 3. Dropped from both sides first: kinds in [`IGNORED`] (machine, account
//!    or timer noise the fakes do not model; each with its reason) and
//!    model choices ([`model_choice`]: reasoning parts and items, whose
//!    presence the model decides; also removed inside message bodies,
//!    [`without_reasoning`]).
//! 4. [`DELIBERATE`]: the last message of a kind is dropped from the real
//!    side of one scenario where the fake deliberately differs (documented
//!    in the fake's module docs).
//! 5. Kinds in [`UNORDERED`] (bookkeeping the backend emits asynchronously)
//!    are compared as a set: each kind must occur on both sides, with the
//!    same shape merged over all its occurrences. Everything else keeps its
//!    order; runs of the same kind collapse into one (streaming deltas) with
//!    their shapes merged (union of fields and types).
//! 6. Shape of a message: field names and nesting, and value types
//!    (`null`, `bool`, `number`, `string`). Values are ignored except under
//!    the discriminator keys in [`DISCRIMINATORS`], which keep their value
//!    (`status: "interrupted"` differs from `status: "completed"`).
//! 7. Arrays: the set of distinct element shapes; an empty array matches any
//!    array (the element type is unknown). Object keys and path segments
//!    that are ids ([`is_id`]) become `:id`.
//!
//! Must NOT: compare ids, timestamps or model text.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::Value;

use super::{Entry, Side};

/// Keys whose value is part of the shape.
pub const DISCRIMINATORS: &[&str] = &[
    "type",
    "status",
    "role",
    "kind",
    "source",
    "hook_event_name",
    "decision",
    "stop_hook_active",
    "method",
];

/// `(backend, stream, kind, why)`.
pub type Rule = (&'static str, &'static str, &'static str, &'static str);

/// Kinds the fakes do not model: dropped from real and fake before comparing.
pub const IGNORED: &[Rule] = &[
    (
        "codex",
        "ws/backend",
        "notify mcpServer/startupStatus/updated",
        "starts the user's own MCP servers from ~/.codex/config.toml; depends on the machine",
    ),
    (
        "codex",
        "ws/backend",
        "notify account/updated",
        "account state, pushed when the server refreshes it",
    ),
    (
        "codex",
        "ws/backend",
        "notify account/rateLimits/updated",
        "account usage, pushed when the server refreshes it",
    ),
    (
        "codex",
        "ws/backend",
        "notify remoteControl/status/changed",
        "machine setting (remote control)",
    ),
    (
        "opencode",
        "sse/backend",
        "plugin.added",
        "plugin catalog loaded at the first prompt; depends on the machine",
    ),
    (
        "opencode",
        "sse/backend",
        "catalog.updated",
        "model catalog loaded at the first prompt; depends on the machine",
    ),
    (
        "opencode",
        "sse/backend",
        "reference.updated",
        "catalog loading at the first prompt",
    ),
    (
        "opencode",
        "sse/backend",
        "integration.updated",
        "catalog loading at the first prompt",
    ),
    (
        "opencode",
        "sse/backend",
        "server.heartbeat",
        "timer (every few seconds)",
    ),
];

/// Bookkeeping emitted asynchronously around a turn: compared as a set of
/// kinds with merged shapes, not in order.
pub const UNORDERED: &[Rule] = &[
    (
        "codex",
        "ws/backend",
        "notify thread/queue/changed",
        "queue bookkeeping; the server also re-announces the queue while a turn streams",
    ),
    (
        "opencode",
        "sse/backend",
        "session.created",
        "emitted lazily; races the first prompt",
    ),
    (
        "opencode",
        "sse/backend",
        "session.updated",
        "title and summary updates; the title comes from the small model and races the turn",
    ),
    (
        "opencode",
        "sse/backend",
        "session.diff",
        "file diff summary, computed asynchronously",
    ),
    (
        "opencode",
        "sse/backend",
        "message.updated",
        "message info updates (summary, completion) race the part events; turn state is read from session.status / session.idle and the parts",
    ),
];

/// `(backend, scenario, stream, kind, why)`: the fake deliberately omits
/// the LAST message of this kind that the real CLI sent in this scenario.
pub const DELIBERATE: &[(&str, &str, &str, &str, &str)] = &[
    (
        "claude",
        "busy",
        "hook/backend",
        "Stop",
        "fake-claude never acts on a channel message that arrived while busy (D16 worst case, gate-02 A6); real claude 2.1.282 answers it after the turn",
    ),
    (
        "claude",
        "busy",
        "hook/client",
        "reply Stop",
        "the reply to that Stop hook",
    ),
];

/// A copy without reasoning elements (`{"type": "reasoning", …}` inside an
/// array: opencode parts, codex items), which are the model's choice.
pub fn without_reasoning(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .filter(|v| v["type"] != "reasoning")
                .map(without_reasoning)
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), without_reasoning(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Messages whose presence is the model's choice (reasoning): dropped.
pub fn model_choice(backend: &str, entry: &Entry, reasoning_parts: &mut BTreeSet<String>) -> bool {
    let msg = &entry.msg;
    match (backend, entry.via.as_str()) {
        ("opencode", "sse") => {
            let props = &msg["properties"];
            match msg["type"].as_str() {
                Some("message.part.updated") if props["part"]["type"] == "reasoning" => {
                    if let Some(id) = props["part"]["id"].as_str() {
                        reasoning_parts.insert(id.to_owned());
                    }
                    true
                }
                Some("message.part.delta") => props["partID"]
                    .as_str()
                    .is_some_and(|id| reasoning_parts.contains(id)),
                _ => false,
            }
        }
        ("codex", "ws") => {
            let method = msg["method"].as_str().unwrap_or_default();
            method.starts_with("item/reasoning/")
                || (matches!(method, "item/started" | "item/completed")
                    && msg["params"]["item"]["type"] == "reasoning")
        }
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Shape {
    Null,
    Bool,
    Number,
    Str,
    /// A discriminator's value.
    Lit(String),
    Array(BTreeSet<Shape>),
    Object(BTreeMap<String, Shape>),
    Union(BTreeSet<Shape>),
}

impl Shape {
    pub fn of(value: &Value) -> Shape {
        Shape::of_keyed(value, None)
    }

    fn of_keyed(value: &Value, key: Option<&str>) -> Shape {
        let literal = key.is_some_and(|k| DISCRIMINATORS.contains(&k));
        match value {
            Value::Null => Shape::Null,
            Value::Bool(b) if literal => Shape::Lit(b.to_string()),
            Value::Bool(_) => Shape::Bool,
            Value::Number(_) => Shape::Number,
            Value::String(s) if literal => Shape::Lit(s.clone()),
            Value::String(_) => Shape::Str,
            Value::Array(items) => Shape::Array(items.iter().map(Shape::of).collect()),
            Value::Object(map) => Shape::Object(
                map.iter()
                    .map(|(k, v)| {
                        let key = if is_id(k) {
                            ":id".to_owned()
                        } else {
                            k.clone()
                        };
                        (key, Shape::of_keyed(v, Some(k)))
                    })
                    .collect(),
            ),
        }
    }

    /// Union of two shapes (used when a run of messages collapses).
    pub fn merge(self, other: Shape) -> Shape {
        if self == other {
            return self;
        }
        match (self, other) {
            (Shape::Object(mut a), Shape::Object(b)) => {
                for (k, v) in b {
                    let merged = match a.remove(&k) {
                        Some(old) => old.merge(v),
                        None => v,
                    };
                    a.insert(k, merged);
                }
                Shape::Object(a)
            }
            (Shape::Array(mut a), Shape::Array(b)) => {
                a.extend(b);
                Shape::Array(a)
            }
            (a, b) => {
                let mut set = BTreeSet::new();
                for s in [a, b] {
                    match s {
                        Shape::Union(inner) => set.extend(inner),
                        s => {
                            set.insert(s);
                        }
                    }
                }
                Shape::Union(set)
            }
        }
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Shape::Null => f.write_str("null"),
            Shape::Bool => f.write_str("bool"),
            Shape::Number => f.write_str("number"),
            Shape::Str => f.write_str("string"),
            Shape::Lit(v) => write!(f, "{v:?}"),
            Shape::Array(items) => {
                f.write_str("[")?;
                for (i, s) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{s}")?;
                }
                f.write_str("]")
            }
            Shape::Object(map) => {
                f.write_str("{")?;
                for (i, (k, s)) in map.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: {s}")?;
                }
                f.write_str("}")
            }
            Shape::Union(items) => {
                for (i, s) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{s}")?;
                }
                Ok(())
            }
        }
    }
}

/// One normalised message (or collapsed run).
#[derive(Clone, Debug)]
pub struct Item {
    pub stream: String,
    pub kind: String,
    pub shape: Shape,
    pub count: usize,
}

/// A transcript after rules 1–5.
#[derive(Default)]
pub struct Normal {
    /// Per stream, in order, runs collapsed.
    pub ordered: BTreeMap<String, Vec<Item>>,
    /// Per stream, the [`UNORDERED`] kinds with their merged shape.
    pub unordered: BTreeMap<String, BTreeMap<String, Shape>>,
}

/// The kind of each entry (rule 2), with its stream.
pub fn kinds(entries: &[Entry]) -> Vec<(String, String)> {
    let mut requests: BTreeMap<(String, &'static str, String), String> = BTreeMap::new();
    let mut last_http = String::new();
    let mut last_hook = String::new();
    entries
        .iter()
        .map(|entry| {
            let msg = &entry.msg;
            let kind = match entry.via.as_str() {
                "http" if entry.from == Side::Client => {
                    last_http = format!(
                        "{} {}",
                        msg["method"].as_str().unwrap_or("?"),
                        route(msg["path"].as_str().unwrap_or("?"))
                    );
                    last_http.clone()
                }
                "http" => format!("{} {last_http}", msg["status"]),
                "sse" => msg["type"].as_str().unwrap_or("?").to_owned(),
                "hook" if entry.from == Side::Backend => {
                    last_hook = msg["hook_event_name"].as_str().unwrap_or("?").to_owned();
                    last_hook.clone()
                }
                "hook" => format!("reply {last_hook}"),
                "key" => msg["key"].as_str().unwrap_or("text").to_owned(),
                "screen" => msg["marker"].as_str().unwrap_or("?").to_owned(),
                via => json_rpc_kind(via, entry.from, msg, &mut requests),
            };
            (format!("{}/{}", entry.via, entry.from.name()), kind)
        })
        .collect()
}

fn listed(rules: &[Rule], backend: &str, stream: &str, kind: &str) -> bool {
    rules
        .iter()
        .any(|(b, s, k, _)| *b == backend && *s == stream && *k == kind)
}

/// Rules 1–5. `scenario` is `Some` for the real side (rule 4).
pub fn normalise(backend: &str, scenario: Option<&str>, entries: &[Entry]) -> Normal {
    let mut reasoning = BTreeSet::new();
    let mut kept: Vec<(String, String, &Entry)> = kinds(entries)
        .into_iter()
        .zip(entries)
        .filter(|((stream, kind), entry)| {
            !listed(IGNORED, backend, stream, kind) && !model_choice(backend, entry, &mut reasoning)
        })
        .map(|((stream, kind), entry)| (stream, kind, entry))
        .collect();
    for (b, sc, stream, kind, _) in DELIBERATE {
        if *b == backend
            && scenario == Some(*sc)
            && let Some(i) = kept.iter().rposition(|(s, k, _)| s == stream && k == kind)
        {
            kept.remove(i);
        }
    }
    let mut out = Normal::default();
    for (stream, kind, entry) in kept {
        let shape = Shape::of(&without_reasoning(&entry.msg));
        if listed(UNORDERED, backend, &stream, &kind) {
            let kinds = out.unordered.entry(stream).or_default();
            let merged = match kinds.remove(&kind) {
                Some(old) => old.merge(shape),
                None => shape,
            };
            kinds.insert(kind, merged);
            continue;
        }
        let items = out.ordered.entry(stream.clone()).or_default();
        match items.last_mut() {
            Some(last) if last.kind == kind => {
                last.shape = std::mem::replace(&mut last.shape, Shape::Null).merge(shape);
                last.count += 1;
            }
            _ => items.push(Item {
                stream,
                kind,
                shape,
                count: 1,
            }),
        }
    }
    out
}

fn json_rpc_kind(
    via: &str,
    from: Side,
    msg: &Value,
    requests: &mut BTreeMap<(String, &'static str, String), String>,
) -> String {
    let id = msg.get("id").map(Value::to_string);
    if let Some(method) = msg["method"].as_str() {
        return match id {
            Some(id) => {
                requests.insert((via.to_owned(), from.name(), id), method.to_owned());
                format!("request {method}")
            }
            None => format!("notify {method}"),
        };
    }
    let requester = match from {
        Side::Client => Side::Backend,
        Side::Backend => Side::Client,
    };
    let method = id
        .and_then(|id| requests.get(&(via.to_owned(), requester.name(), id)))
        .map_or("?", String::as_str);
    if msg.get("error").is_some() {
        format!("error {method}")
    } else {
        format!("result {method}")
    }
}

/// `/session/<ses-1>/message` → `/session/:id/message` (see [`is_id`]);
/// the query string is dropped.
fn route(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    path.split('/')
        .map(|seg| if is_id(seg) { ":id" } else { seg })
        .collect::<Vec<_>>()
        .join("/")
}

/// A redacted placeholder, or a word with `_` and a digit (`ses_fake0001`,
/// not `prompt_async`): an id in a path segment or an object key.
fn is_id(s: &str) -> bool {
    s.starts_with('<') || (s.contains('_') && s.chars().any(|c| c.is_ascii_digit()))
}

/// Every difference between the real recording of `scenario` and the fake
/// run, as readable lines; empty when they have the same shape.
pub fn compare(backend: &str, scenario: &str, real: &[Entry], fake: &[Entry]) -> Vec<String> {
    let real = normalise(backend, Some(scenario), real);
    let fake = normalise(backend, None, fake);
    let mut diffs = Vec::new();
    let names: BTreeSet<&String> = real.ordered.keys().chain(fake.ordered.keys()).collect();
    let empty = Vec::new();
    for name in names {
        let r = real.ordered.get(name).unwrap_or(&empty);
        let f = fake.ordered.get(name).unwrap_or(&empty);
        for i in 0..r.len().max(f.len()) {
            match (r.get(i), f.get(i)) {
                (Some(a), Some(b)) if a.kind == b.kind => {
                    let mut at = Vec::new();
                    shape_diff(&a.shape, &b.shape, "", &mut at);
                    for d in at {
                        diffs.push(format!("{name} #{i} `{}`: {d}", a.kind));
                    }
                }
                (a, b) => {
                    let rest = |items: &[Item]| {
                        items[i.min(items.len())..]
                            .iter()
                            .map(|x| x.kind.clone())
                            .collect::<Vec<_>>()
                            .join(" → ")
                    };
                    diffs.push(format!(
                        "{name} #{i}: real `{}`, fake `{}`\n    real from here: {}\n    fake from here: {}",
                        a.map_or("(end)", |x| x.kind.as_str()),
                        b.map_or("(end)", |x| x.kind.as_str()),
                        rest(r),
                        rest(f),
                    ));
                    break;
                }
            }
        }
    }
    let none = BTreeMap::new();
    let names: BTreeSet<&String> = real.unordered.keys().chain(fake.unordered.keys()).collect();
    for name in names {
        let r = real.unordered.get(name).unwrap_or(&none);
        let f = fake.unordered.get(name).unwrap_or(&none);
        let kinds: BTreeSet<&String> = r.keys().chain(f.keys()).collect();
        for kind in kinds {
            match (r.get(kind), f.get(kind)) {
                (Some(a), Some(b)) => {
                    let mut at = Vec::new();
                    shape_diff(a, b, "", &mut at);
                    for d in at {
                        diffs.push(format!("{name} (any order) `{kind}`: {d}"));
                    }
                }
                (Some(_), None) => diffs.push(format!("{name}: fake never sends `{kind}`")),
                (None, _) => diffs.push(format!("{name}: fake sends `{kind}`, real never does")),
            }
        }
    }
    diffs
}

fn shape_diff(real: &Shape, fake: &Shape, path: &str, out: &mut Vec<String>) {
    match (real, fake) {
        (Shape::Object(a), Shape::Object(b)) => {
            for (k, v) in a {
                let p = format!("{path}.{k}");
                match b.get(k) {
                    Some(w) => shape_diff(v, w, &p, out),
                    None => out.push(format!("fake lacks {p} ({v})")),
                }
            }
            for (k, w) in b {
                if !a.contains_key(k) {
                    out.push(format!("fake has extra {path}.{k} ({w})"));
                }
            }
        }
        (Shape::Array(a), Shape::Array(b)) if a.is_empty() || b.is_empty() => {}
        (Shape::Array(a), Shape::Array(b)) if a.len() == 1 && b.len() == 1 => {
            let (x, y) = (a.iter().next(), b.iter().next());
            if let (Some(x), Some(y)) = (x, y) {
                shape_diff(x, y, &format!("{path}[]"), out);
            }
        }
        (a, b) if a != b => out.push(format!("{} is {a} in real, {b} in fake", or_root(path))),
        _ => {}
    }
}

fn or_root(path: &str) -> &str {
    if path.is_empty() { "(message)" } else { path }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn e(from: Side, via: &str, msg: Value) -> Entry {
        Entry {
            from,
            via: via.into(),
            msg,
        }
    }

    #[test]
    fn values_and_ids_are_ignored_but_types_fields_and_discriminators_are_not() {
        let real = [
            e(
                Side::Client,
                "ws",
                json!({"id": 1, "method": "turn/start", "params": {"text": "hi"}}),
            ),
            e(
                Side::Backend,
                "ws",
                json!({"id": 1, "result": {"turn": {"id": "a", "status": "inProgress"}}}),
            ),
            e(
                Side::Backend,
                "ws",
                json!({"method": "d", "params": {"delta": "x"}}),
            ),
            e(
                Side::Backend,
                "ws",
                json!({"method": "d", "params": {"delta": "y"}}),
            ),
        ];
        let same = [
            e(
                Side::Client,
                "ws",
                json!({"id": 7, "method": "turn/start", "params": {"text": "other"}}),
            ),
            e(
                Side::Backend,
                "ws",
                json!({"method": "d", "params": {"delta": "z"}}),
            ),
            e(
                Side::Backend,
                "ws",
                json!({"id": 7, "result": {"turn": {"id": "b", "status": "inProgress"}}}),
            ),
            e(
                Side::Backend,
                "ws",
                json!({"method": "d", "params": {"delta": "z"}}),
            ),
        ];
        // Order within the backend stream differs: result first vs delta first.
        assert!(!compare("x", "s", &real, &same).is_empty());
        let same = [same[0].clone(), same[2].clone(), same[1].clone()];
        assert_eq!(compare("x", "s", &real, &same), Vec::<String>::new());
        let wrong = [
            real[0].clone(),
            e(
                Side::Backend,
                "ws",
                json!({"id": 1, "result": {"turn": {"id": 3, "status": "completed"}}}),
            ),
            real[2].clone(),
        ];
        let diffs = compare("x", "s", &real, &wrong);
        assert!(
            diffs
                .iter()
                .any(|d| d.contains(".result.turn.id is string in real, number in fake")),
            "{diffs:?}"
        );
        assert!(
            diffs.iter().any(|d| d.contains(".result.turn.status")),
            "{diffs:?}"
        );
    }

    #[test]
    fn http_routes_hide_ids_and_empty_arrays_match_any_array() {
        let real = [
            e(
                Side::Client,
                "http",
                json!({"method": "GET", "path": "/session/<ses-1>/message", "body": null}),
            ),
            e(
                Side::Backend,
                "http",
                json!({"status": 200, "body": [{"info": {"id": "x"}}]}),
            ),
        ];
        let fake = [
            e(
                Side::Client,
                "http",
                json!({"method": "GET", "path": "/session/<ses-4>/message", "body": null}),
            ),
            e(Side::Backend, "http", json!({"status": 200, "body": []})),
        ];
        assert_eq!(compare("x", "s", &real, &fake), Vec::<String>::new());
        let items = normalise("x", None, &real);
        assert_eq!(
            items.ordered["http/backend"][0].kind,
            "200 GET /session/:id/message"
        );
    }
}
