//! Client protocol contract (rules CLP-1..12 in CONTRACTS.md, gate 8 P9):
//! what TUI and CLI tests rely on from the testkit fake daemon must hold for
//! the real `agend daemon` too. The same cases run against [`FakeDaemonFixture`]
//! and against the real daemon (`crates/agend/tests/client_protocol.rs`).
//!
//! The driver is [`ProbeClient`], not `agend-client`, so a client bug cannot
//! hide behind the same code. Every message is a core protocol type encoded
//! with `serde_json` (#1493).
//!
//! Not pinned: agent commands (`command`) — the fake serves them, the real
//! daemon answers `not_supported` until gate 9; `no_terminal` for an unknown
//! instance (the fake answers a screen for any id).
//!
//! Must NOT: hand-write wire shapes except for the malformed lines the rules
//! are about.

pub mod proxy;

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::client::{
    AgentState, AnswerAskData, AttentionAction, AttentionRequiredData, ClientHello, ClientRequest,
    ClientResponse, CommandResult, DaemonEvent, EventData, FleetView, InstanceData, InstanceView,
    RequestIdData, ResolveAttentionData, SUPPORTED_VERSIONS, SubscribeEventsData, TaskChangedData,
    TerminalInputData, V1, error_code,
};

use super::{Case, CaseResult, Report, ensure, run_suite};
use crate::fake_daemon::{FakeDaemon, ProbeClient};
use crate::tempdir::TempDir;

/// Events a slow-client case sends (gate 8 P8).
pub const BURST_EVENTS: usize = 2000;
/// A wait for something that must happen.
const WITHIN: Duration = Duration::from_secs(10);
/// No line for this long ends a collection.
const QUIET: Duration = Duration::from_millis(700);

pub trait ClientProtocolFixture {
    /// The server's socket.
    fn socket(&self) -> PathBuf;
    /// Makes at least one new event happen; returns once it is published.
    fn emit(&mut self) -> Result<(), String>;
    /// Publishes `n` events at once; `Err` when this server cannot (the real
    /// daemon binary has no way to make thousands of real events).
    fn burst(&mut self, n: usize) -> Result<(), String>;
    /// Restarts the server on the same socket and returns once it accepts
    /// connections again.
    fn restart(&mut self) -> Result<(), String>;
    /// The `attention_id` of a needs-you item the operator can resolve with
    /// `retry` (a new item per call).
    fn retry_item(&mut self) -> Result<String, String>;
    /// An instance whose terminal can be subscribed to.
    fn terminal_instance(&self) -> String;
}

pub fn cases<F: ClientProtocolFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            rule: "CLP-1",
            name: "hello_comes_first",
            check: |fx| hello_comes_first(&fx),
        },
        Case {
            rule: "CLP-2",
            name: "versions_are_negotiated",
            check: |fx| versions_are_negotiated(&fx),
        },
        Case {
            rule: "CLP-3",
            name: "fleet_then_events_after_as_of",
            check: |mut fx| fleet_then_events_after_as_of(&mut fx),
        },
        Case {
            rule: "CLP-4",
            name: "old_or_future_cursor_is_a_gap",
            check: |mut fx| old_or_future_cursor_is_a_gap(&mut fx),
        },
        Case {
            rule: "CLP-5",
            name: "no_cursor_replays_the_retained_events",
            check: |mut fx| no_cursor_replays_the_retained_events(&mut fx),
        },
        Case {
            rule: "CLP-6",
            name: "unknown_or_invalid_requests_keep_the_connection",
            check: |fx| unknown_or_invalid_requests_keep_the_connection(&fx),
        },
        Case {
            rule: "CLP-7",
            name: "two_clients_get_the_same_events",
            check: |mut fx| two_clients_get_the_same_events(&mut fx),
        },
        Case {
            rule: "CLP-8",
            name: "slow_clients_are_closed_others_are_not",
            check: |mut fx| slow_clients(&mut fx).map(|_| ()),
        },
        Case {
            rule: "CLP-9",
            name: "a_restart_ends_connections_and_starts_newer_ids",
            check: |mut fx| a_restart_ends_connections_and_starts_newer_ids(&mut fx),
        },
        Case {
            rule: "CLP-10",
            name: "refused_requests_change_nothing",
            check: |fx| refused_requests_change_nothing(&fx),
        },
        Case {
            rule: "CLP-11",
            name: "only_the_operator_resolves_listed_actions",
            check: |mut fx| only_the_operator_resolves_listed_actions(&mut fx),
        },
        Case {
            rule: "CLP-12",
            name: "terminal_starts_with_the_screen",
            check: |fx| terminal_starts_with_the_screen(&fx),
        },
    ]
}

pub fn run<F: ClientProtocolFixture>(implementation: &str, make: impl FnMut() -> F) -> Report {
    run_suite("ClientProtocol", implementation, &cases::<F>(), make)
}

/// Only the cases of `rules`.
pub fn run_rules<F: ClientProtocolFixture>(
    implementation: &str,
    rules: &[&str],
    make: impl FnMut() -> F,
) -> Report {
    let cases: Vec<Case<F>> = cases::<F>()
        .into_iter()
        .filter(|c| rules.contains(&c.rule))
        .collect();
    run_suite("ClientProtocol", implementation, &cases, make)
}

// ---- driver helpers ----

fn io(what: &str) -> impl Fn(io::Error) -> String + '_ {
    move |e| format!("{what}: {e}")
}

fn operator<F: ClientProtocolFixture>(fx: &F) -> Result<ProbeClient, String> {
    ProbeClient::hello(&fx.socket(), None)
        .map(|(c, _)| c)
        .map_err(io("hello"))
}

fn send(c: &mut ProbeClient, request: &ClientRequest) -> Result<(), String> {
    c.send(request).map_err(io("send"))
}

/// Lines until `done` accepts one (returned) or `within` passes; events on
/// the way are collected.
fn until(
    c: &mut ProbeClient,
    within: Duration,
    done: impl Fn(&ClientResponse) -> bool,
    events: &mut Vec<EventData>,
) -> Result<ClientResponse, String> {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("no expected reply within {within:?}"));
        }
        match c.recv_within(left) {
            Ok(Some(reply)) if done(&reply) => return Ok(reply),
            Ok(Some(ClientResponse::Event { data })) => events.push(data),
            Ok(Some(_)) => {}
            Ok(None) => return Err("the server closed the connection".into()),
            Err(e) if timed_out(&e) => {}
            Err(e) => return Err(format!("read: {e}")),
        }
    }
}

fn timed_out(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// What a connection sent until it was quiet for [`QUIET`] (or `max` passed).
struct Collected {
    events: Vec<EventData>,
    errors: Vec<String>,
    closed: bool,
}

fn collect(c: &mut ProbeClient, max: Duration) -> Result<Collected, String> {
    let deadline = Instant::now() + max;
    let mut out = Collected {
        events: Vec::new(),
        errors: Vec::new(),
        closed: false,
    };
    while !Instant::now().ge(&deadline) {
        match c.recv_within(QUIET.min(deadline.saturating_duration_since(Instant::now()))) {
            Ok(Some(ClientResponse::Event { data })) => out.events.push(data),
            Ok(Some(ClientResponse::Error { data })) => out.errors.push(data.code),
            Ok(Some(_)) => {}
            Ok(None) => {
                out.closed = true;
                break;
            }
            Err(e) if timed_out(&e) => break,
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    Ok(out)
}

fn ids(events: &[EventData]) -> Vec<u64> {
    events.iter().map(|e| e.event_id).collect()
}

fn consecutive(ids: &[u64]) -> bool {
    ids.windows(2).all(|w| w[1] == w[0] + 1)
}

fn get_fleet(c: &mut ProbeClient, id: &str) -> Result<FleetView, String> {
    send(
        c,
        &ClientRequest::GetFleet {
            data: RequestIdData {
                request_id: id.into(),
            },
        },
    )?;
    let mut skipped = Vec::new();
    match until(
        c,
        WITHIN,
        |r| {
            matches!(r, ClientResponse::Fleet { data } if data.request_id == id)
                || matches!(r, ClientResponse::Error { .. })
        },
        &mut skipped,
    )? {
        ClientResponse::Fleet { data } => Ok(data.fleet),
        other => Err(format!("get_fleet answered {other:?}")),
    }
}

fn subscribe(c: &mut ProbeClient, after: Option<u64>) -> Result<(), String> {
    send(
        c,
        &ClientRequest::SubscribeEvents {
            data: SubscribeEventsData {
                after_event_id: after,
            },
        },
    )
}

/// The error code a connection answers next (events are skipped).
fn next_error(c: &mut ProbeClient, within: Duration) -> Result<(String, Option<String>), String> {
    let mut skipped = Vec::new();
    match until(
        c,
        within,
        |r| matches!(r, ClientResponse::Error { .. }),
        &mut skipped,
    )? {
        ClientResponse::Error { data } => Ok((data.code, data.request_id)),
        _ => unreachable!("filtered"),
    }
}

fn closes_within(c: &mut ProbeClient, within: Duration) -> Result<(), String> {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("the connection stayed open for {within:?}"));
        }
        match c.recv_within(left) {
            Ok(None) => return Ok(()),
            Ok(Some(_)) => {}
            Err(e) if timed_out(&e) => {}
            Err(_) => return Ok(()),
        }
    }
}

fn resolve(
    c: &mut ProbeClient,
    request_id: &str,
    attention_id: &str,
    action: AttentionAction,
    events: &mut Vec<EventData>,
) -> Result<ClientResponse, String> {
    send(
        c,
        &ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: request_id.into(),
                attention_id: attention_id.into(),
                action,
            },
        },
    )?;
    until(
        c,
        WITHIN,
        |r| match r {
            ClientResponse::CommandResult { data } => data.request_id == request_id,
            ClientResponse::Error { data } => data.request_id.as_deref() == Some(request_id),
            _ => false,
        },
        events,
    )
}

fn error_code_of(reply: &ClientResponse) -> Option<&str> {
    match reply {
        ClientResponse::Error { data } => Some(&data.code),
        _ => None,
    }
}

// ---- cases ----

fn hello_comes_first<F: ClientProtocolFixture>(fx: &F) -> CaseResult {
    let first = ClientRequest::GetFleet {
        data: RequestIdData {
            request_id: "clp-1".into(),
        },
    };
    for (what, raw) in [
        ("get_fleet", None),
        ("a line that is not JSON", Some("not json")),
    ] {
        let mut c = ProbeClient::connect(&fx.socket()).map_err(io("connect"))?;
        match raw {
            Some(line) => c.send_raw(line).map_err(io("send"))?,
            None => send(&mut c, &first)?,
        }
        let reply = c.recv_within(WITHIN).map_err(io("read"))?;
        let code = reply.as_ref().and_then(error_code_of);
        ensure(code == Some(error_code::HELLO_REQUIRED), || {
            format!("{what} before hello answered {reply:?}, expected hello_required")
        })?;
        closes_within(&mut c, WITHIN).map_err(|e| format!("after hello_required: {e}"))?;
    }
    Ok(())
}

fn hello_with(
    fx_socket: PathBuf,
    hello: ClientHello,
) -> Result<(ProbeClient, ClientResponse), String> {
    let mut c = ProbeClient::connect(&fx_socket).map_err(io("connect"))?;
    let reply = c
        .request(&ClientRequest::Hello { data: hello })
        .map_err(io("hello"))?;
    Ok((c, reply))
}

fn versions_are_negotiated<F: ClientProtocolFixture>(fx: &F) -> CaseResult {
    let (mut c, reply) = hello_with(
        fx.socket(),
        ClientHello {
            supported: vec![ProtocolVersion::new(2, 0)],
            caller: None,
        },
    )?;
    ensure(
        error_code_of(&reply) == Some(error_code::VERSION_MISMATCH),
        || format!("hello 2.0 answered {reply:?}, expected version_mismatch"),
    )?;
    closes_within(&mut c, WITHIN).map_err(|e| format!("after version_mismatch: {e}"))?;
    let selected = |reply: &ClientResponse| match reply {
        ClientResponse::Hello { data } => Some(data.selected),
        _ => None,
    };
    let (_, reply) = hello_with(
        fx.socket(),
        ClientHello {
            supported: vec![V1],
            caller: None,
        },
    )?;
    ensure(selected(&reply) == Some(V1), || {
        format!("a 1.0 client got {reply:?}, expected 1.0 selected")
    })?;
    let (_, reply) = hello_with(
        fx.socket(),
        ClientHello {
            supported: SUPPORTED_VERSIONS.to_vec(),
            caller: Some("clp-agent".into()),
        },
    )?;
    ensure(selected(&reply) == Some(SUPPORTED_VERSIONS[0]), || {
        format!("a 1.1 hello with a caller got {reply:?}")
    })
}

fn fleet_then_events_after_as_of<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    let mut c = operator(fx)?;
    let view = get_fleet(&mut c, "clp-3")?;
    ensure(view.teams.iter().any(|t| t.team_id == "general"), || {
        format!("the fleet view has no general team: {view:?}")
    })?;
    fx.emit()?; // between get_fleet and subscribe
    subscribe(&mut c, Some(view.as_of_event_id))?;
    fx.emit()?;
    let got = collect(&mut c, WITHIN)?;
    let ids = ids(&got.events);
    ensure(
        ids.len() >= 2 && ids[0] == view.as_of_event_id + 1 && consecutive(&ids),
        || {
            format!(
                "after as_of {} expected consecutive ids from {} (at least 2), got {ids:?} (errors {:?})",
                view.as_of_event_id,
                view.as_of_event_id + 1,
                got.errors
            )
        },
    )
}

fn old_or_future_cursor_is_a_gap<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    let mut c = operator(fx)?;
    let view = get_fleet(&mut c, "clp-4")?;
    fx.emit()?;
    fx.emit()?;
    subscribe(&mut c, Some(view.as_of_event_id))?;
    let before = collect(&mut c, WITHIN)?;
    let old = *ids(&before.events)
        .last()
        .ok_or("no event before the restart")?;
    drop(c);
    fx.restart()?;
    for _ in 0..3 {
        fx.emit()?;
    }
    let mut c = operator(fx)?;
    let latest = get_fleet(&mut c, "clp-4b")?.as_of_event_id;
    let future = latest + 1_000_000;
    for (what, cursor) in [
        ("a cursor from before the restart", old),
        ("a cursor newer than the newest id", future),
    ] {
        let mut c = operator(fx)?;
        subscribe(&mut c, Some(cursor))?;
        let got = collect(&mut c, WITHIN)?;
        ensure(
            got.errors == [error_code::EVENT_GAP] && got.events.is_empty(),
            || {
                format!(
                    "{what} ({cursor}; the server's newest is {latest}) got errors {:?} and events {:?}, expected only event_gap",
                    got.errors,
                    ids(&got.events)
                )
            },
        )?;
    }
    Ok(())
}

fn no_cursor_replays_the_retained_events<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    fx.emit()?;
    fx.emit()?;
    let mut c = operator(fx)?;
    let view = get_fleet(&mut c, "clp-5")?;
    subscribe(&mut c, None)?;
    let got = collect(&mut c, WITHIN)?;
    let ids = ids(&got.events);
    let first = *ids.first().ok_or_else(|| {
        format!(
            "subscribing without a cursor replayed nothing (errors {:?})",
            got.errors
        )
    })?;
    ensure(
        consecutive(&ids)
            && ids.last() >= Some(&view.as_of_event_id)
            && first < view.as_of_event_id,
        || {
            format!(
                "replay {ids:?} does not cover the retained events up to {}",
                view.as_of_event_id
            )
        },
    )?;
    // The replay began at the oldest retained event: "oldest − 1" continues,
    // one before that is a gap.
    let mut ok = operator(fx)?;
    subscribe(&mut ok, Some(first - 1))?;
    let continued = collect(&mut ok, WITHIN)?;
    ensure(
        continued.errors.is_empty() && !continued.events.is_empty(),
        || {
            format!(
                "cursor {} (oldest − 1) did not continue: errors {:?}",
                first - 1,
                continued.errors
            )
        },
    )?;
    let mut gap = operator(fx)?;
    subscribe(&mut gap, Some(first - 2))?;
    let (code, _) = next_error(&mut gap, WITHIN)?;
    ensure(code == error_code::EVENT_GAP, || {
        format!("cursor {} (before the oldest − 1) got {code}", first - 2)
    })
}

fn unknown_or_invalid_requests_keep_the_connection<F: ClientProtocolFixture>(fx: &F) -> CaseResult {
    let mut c = operator(fx)?;
    c.send_raw(r#"{"type":"clp_future_request","data":{}}"#)
        .map_err(io("send"))?;
    let (code, _) = next_error(&mut c, WITHIN)?;
    ensure(code == error_code::UNKNOWN_REQUEST, || {
        format!("an unknown request got {code}, expected unknown_request")
    })?;
    c.send_raw("not json").map_err(io("send"))?;
    let (code, _) = next_error(&mut c, WITHIN)?;
    ensure(code == error_code::INVALID_REQUEST, || {
        format!("a line that is not JSON got {code}, expected invalid_request")
    })?;
    get_fleet(&mut c, "clp-6")
        .map(|_| ())
        .map_err(|e| format!("the connection did not stay usable: {e}"))
}

fn two_clients_get_the_same_events<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    let mut a = operator(fx)?;
    let as_of = get_fleet(&mut a, "clp-7")?.as_of_event_id;
    let mut b = operator(fx)?;
    subscribe(&mut a, Some(as_of))?;
    subscribe(&mut b, Some(as_of))?;
    fx.emit()?;
    fx.emit()?;
    let got_a = collect(&mut a, WITHIN)?.events;
    let got_b = collect(&mut b, WITHIN)?.events;
    ensure(got_a.len() >= 2 && got_a == got_b, || {
        format!(
            "the two clients differ:\n  a {:?}\n  b {:?}",
            ids(&got_a),
            ids(&got_b)
        )
    })
}

/// The slow-client scenario (gate 8 P8): a client that reads everything,
/// one that reads one line per 10 ms and one that never reads, then
/// [`BURST_EVENTS`] events. Returns lines for the demo.
pub fn slow_clients<F: ClientProtocolFixture>(fx: &mut F) -> Result<Vec<String>, String> {
    let mut normal = operator(fx)?;
    let as_of = get_fleet(&mut normal, "clp-8")?.as_of_event_id;
    let mut slow = operator(fx)?;
    let mut none = operator(fx)?;
    for c in [&mut normal, &mut slow, &mut none] {
        subscribe(c, Some(as_of))?;
    }
    // Let the subscriptions land before the burst.
    std::thread::sleep(Duration::from_millis(200));
    let reader = std::thread::spawn(move || -> Result<Vec<u64>, String> {
        let mut ids = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        while ids.len() < BURST_EVENTS && Instant::now() < deadline {
            match normal.recv_within(Duration::from_secs(5)) {
                Ok(Some(ClientResponse::Event { data })) => ids.push(data.event_id),
                Ok(Some(ClientResponse::Error { data })) => {
                    return Err(format!("the normal client got {}", data.code));
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    return Err(format!(
                        "the normal client was closed after {} events",
                        ids.len()
                    ));
                }
                Err(e) => return Err(format!("the normal client: {e}")),
            }
        }
        Ok(ids)
    });
    let slow_reader = std::thread::spawn(move || -> (usize, Vec<String>, bool) {
        let (mut events, mut errors) = (0, Vec::new());
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            match slow.recv_within(Duration::from_secs(10)) {
                Ok(Some(ClientResponse::Event { .. })) => events += 1,
                Ok(Some(ClientResponse::Error { data })) => errors.push(data.code),
                Ok(Some(_)) => {}
                Ok(None) => return (events, errors, true),
                Err(e) => {
                    errors.push(format!("read error: {e}"));
                    return (events, errors, false);
                }
            }
        }
        (events, errors, false)
    });
    // In steps, so a client that keeps reading keeps up.
    let step = 50;
    for _ in 0..BURST_EVENTS / step {
        fx.burst(step)?;
        std::thread::sleep(Duration::from_millis(10));
    }
    let burst_done = Instant::now();
    let ids = reader.join().map_err(|_| "the normal reader panicked")??;
    let expected: Vec<u64> = (as_of + 1..=as_of + BURST_EVENTS as u64).collect();
    ensure(ids == expected, || {
        format!(
            "the normal client got {} events, not {BURST_EVENTS} in order from {}",
            ids.len(),
            as_of + 1
        )
    })?;
    let (slow_events, slow_errors, slow_closed) =
        slow_reader.join().map_err(|_| "the slow reader panicked")?;
    ensure(
        slow_errors == [error_code::EVENT_GAP] && slow_closed && slow_events < BURST_EVENTS,
        || {
            format!(
                "the slow reader got {slow_events} events, errors {slow_errors:?}, closed {slow_closed}; expected event_gap and closed"
            )
        },
    )?;
    // The client that never reads: closed by the 5 s write timeout, so it
    // never gets event_gap. Read only after the timeout has passed.
    std::thread::sleep(Duration::from_millis(6500).saturating_sub(burst_done.elapsed()));
    let got = collect(&mut none, WITHIN)?;
    ensure(got.closed && got.errors.is_empty(), || {
        format!(
            "the client that never reads: closed {}, errors {:?}, {} events; expected closed without event_gap",
            got.closed,
            got.errors,
            got.events.len()
        )
    })?;
    Ok(vec![
        format!("normal reader: all {BURST_EVENTS} events, in order"),
        format!("slow reader: event_gap after {slow_events} events, then closed"),
        format!(
            "no reader: closed after 5 s write timeout ({} events were buffered; no event_gap)",
            got.events.len()
        ),
    ])
}

fn a_restart_ends_connections_and_starts_newer_ids<F: ClientProtocolFixture>(
    fx: &mut F,
) -> CaseResult {
    let mut c = operator(fx)?;
    let view = get_fleet(&mut c, "clp-9")?;
    subscribe(&mut c, Some(view.as_of_event_id))?;
    fx.emit()?;
    let seen = collect(&mut c, WITHIN)?;
    let newest = ids(&seen.events)
        .into_iter()
        .max()
        .unwrap_or(0)
        .max(view.as_of_event_id);
    fx.restart()?;
    closes_within(&mut c, WITHIN)
        .map_err(|e| format!("the old connection after the restart: {e}"))?;
    let mut fresh = operator(fx)?;
    let after = get_fleet(&mut fresh, "clp-9b")?;
    ensure(after.as_of_event_id > newest, || {
        format!(
            "after the restart as_of {} is not newer than {newest} from before",
            after.as_of_event_id
        )
    })
}

fn refused_requests_change_nothing<F: ClientProtocolFixture>(fx: &F) -> CaseResult {
    let mut c = operator(fx)?;
    let before = get_fleet(&mut c, "clp-10")?;
    subscribe(&mut c, Some(before.as_of_event_id))?;
    send(
        &mut c,
        &ClientRequest::TerminalInput {
            data: TerminalInputData {
                instance_id: fx.terminal_instance(),
                bytes_base64: "aGk=".into(),
            },
        },
    )?;
    let (code, _) = next_error(&mut c, WITHIN)?;
    ensure(code == error_code::NOT_SUPPORTED, || {
        format!("terminal_input got {code}, expected not_supported")
    })?;
    send(
        &mut c,
        &ClientRequest::AnswerAsk {
            data: AnswerAskData {
                request_id: "clp-10a".into(),
                ask_id: "clp-no-such-ask".into(),
                source: agend_core::protocol::ask::AnswerSource::Cli,
                reply: agend_core::protocol::ask::AskReply::Text { text: "x".into() },
            },
        },
    )?;
    let (code, id) = next_error(&mut c, WITHIN)?;
    ensure(
        code == error_code::UNKNOWN_ASK && id.as_deref() == Some("clp-10a"),
        || format!("answer_ask for no ask got {code} ({id:?}), expected unknown_ask"),
    )?;
    let quiet = collect(&mut c, QUIET)?;
    ensure(quiet.events.is_empty(), || {
        format!("refused requests made events {:?}", ids(&quiet.events))
    })?;
    let after = get_fleet(&mut c, "clp-10b")?;
    ensure(after == before, || {
        format!("the fleet view changed:\n  before {before:?}\n  after  {after:?}")
    })
}

fn only_the_operator_resolves_listed_actions<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    let item = fx.retry_item()?;
    let mut ignore = Vec::new();
    let (mut agent, _) =
        ProbeClient::hello(&fx.socket(), Some("clp-agent")).map_err(io("agent hello"))?;
    for (n, id) in [item.as_str(), "clp-no-such-item"].into_iter().enumerate() {
        let reply = resolve(
            &mut agent,
            &format!("clp-11a{n}"),
            id,
            AttentionAction::Retry,
            &mut ignore,
        )?;
        ensure(error_code_of(&reply) == Some(error_code::FORBIDDEN), || {
            format!("an agent resolving {id} got {reply:?}, expected forbidden")
        })?;
    }
    let mut op = operator(fx)?;
    let view = get_fleet(&mut op, "clp-11")?;
    let listed = |view: &FleetView| {
        view.attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(item.as_str()))
    };
    ensure(listed(&view), || format!("{item} is not in the fleet view"))?;
    for (n, (id, action)) in [
        ("clp-no-such-item", AttentionAction::Retry),
        (item.as_str(), AttentionAction::Unknown),
    ]
    .into_iter()
    .enumerate()
    {
        let reply = resolve(&mut op, &format!("clp-11b{n}"), id, action, &mut ignore)?;
        ensure(
            error_code_of(&reply) == Some(error_code::UNKNOWN_ATTENTION),
            || format!("resolving {id} with {action:?} got {reply:?}, expected unknown_attention"),
        )?;
    }
    subscribe(&mut op, Some(view.as_of_event_id))?;
    let mut events = Vec::new();
    let reply = resolve(
        &mut op,
        "clp-11c",
        &item,
        AttentionAction::Retry,
        &mut events,
    )?;
    ensure(
        matches!(&reply, ClientResponse::CommandResult { data } if data.result == CommandResult::Accepted),
        || format!("the operator resolving {item} got {reply:?}, expected accepted"),
    )?;
    events.extend(collect(&mut op, WITHIN)?.events);
    let resolved = events.iter().any(|e| {
        matches!(&e.event, DaemonEvent::AttentionResolved { data }
            if data.attention_id == item && data.action == AttentionAction::Retry)
    });
    ensure(resolved, || {
        format!("no attention_resolved for {item} in {:?}", ids(&events))
    })?;
    let after = get_fleet(&mut op, "clp-11d")?;
    ensure(!listed(&after), || {
        format!("{item} is still listed after it was resolved")
    })
}

fn terminal_starts_with_the_screen<F: ClientProtocolFixture>(fx: &F) -> CaseResult {
    let instance = fx.terminal_instance();
    let mut c = operator(fx)?;
    send(
        &mut c,
        &ClientRequest::SubscribeTerminal {
            data: InstanceData {
                instance_id: instance.clone(),
            },
        },
    )?;
    let mut skipped = Vec::new();
    let reply = until(
        &mut c,
        WITHIN,
        |r| !matches!(r, ClientResponse::Event { .. }),
        &mut skipped,
    )?;
    ensure(
        matches!(&reply, ClientResponse::TerminalSnapshot { data } if data.instance_id == instance),
        || format!("subscribe_terminal {instance} answered {reply:?} first, expected its screen"),
    )
}

// ---- the fake daemon as a fixture ----

/// The testkit fake daemon behind the CLP contract: an instance with a
/// terminal, `emit` publishes a `task_changed`, `restart` starts a new fake
/// on the same socket.
pub struct FakeDaemonFixture {
    daemon: Option<FakeDaemon>,
    dir: TempDir,
    base: Arc<AtomicU64>,
    items: u64,
}

pub const FAKE_INSTANCE: &str = "clp-1";

impl FakeDaemonFixture {
    pub fn new() -> FakeDaemonFixture {
        let dir = TempDir::new("clp").expect("temp dir");
        let mut fx = FakeDaemonFixture {
            daemon: None,
            dir,
            base: Arc::new(AtomicU64::new(0)),
            items: 0,
        };
        fx.boot().expect("fake daemon");
        fx
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }

    fn boot(&mut self) -> io::Result<()> {
        let daemon = FakeDaemon::start_at(&self.path())?;
        self.base.store(daemon.event_id_start(), Ordering::SeqCst);
        daemon.set_instance(InstanceView {
            instance_id: FAKE_INSTANCE.into(),
            team_id: "general".into(),
            backend: "claude".into(),
            state: AgentState::Unknown,
        });
        self.daemon = Some(daemon);
        Ok(())
    }

    pub fn daemon(&self) -> &FakeDaemon {
        self.daemon.as_ref().expect("booted")
    }

    /// The current fake's event id base (changes on restart).
    pub fn base(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.base)
    }
}

impl Default for FakeDaemonFixture {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientProtocolFixture for FakeDaemonFixture {
    fn socket(&self) -> PathBuf {
        self.path()
    }

    fn emit(&mut self) -> Result<(), String> {
        self.daemon().emit(DaemonEvent::TaskChanged {
            data: TaskChangedData {
                task_id: "T-clp".into(),
                summary: "changed".into(),
                task: None,
            },
        });
        Ok(())
    }

    fn burst(&mut self, n: usize) -> Result<(), String> {
        for _ in 0..n {
            self.emit()?;
        }
        Ok(())
    }

    fn restart(&mut self) -> Result<(), String> {
        self.daemon = None;
        // A real restart takes far longer than the 1 ms of an id base step.
        std::thread::sleep(Duration::from_millis(5));
        self.boot().map_err(|e| format!("restart the fake: {e}"))
    }

    fn retry_item(&mut self) -> Result<String, String> {
        self.items += 1;
        let id = format!("instance-failed:clp-f{}", self.items);
        self.daemon().add_attention(AttentionRequiredData {
            reason: "clp item".into(),
            task_id: None,
            ask: None,
            recap: None,
            attention_id: Some(id.clone()),
            unblocks: Some(0),
            waiting_since_unix_ms: Some(0),
            if_ignored: Some("it stays".into()),
            actions: vec![AttentionAction::Retry],
            instance_id: None,
        });
        Ok(id)
    }

    fn terminal_instance(&self) -> String {
        FAKE_INSTANCE.into()
    }
}

// ---- mutants: the fake behind a misbehaving proxy ----

/// A fixture seen through a [`proxy::Proxy`].
pub struct Proxied<F> {
    pub inner: F,
    proxy: proxy::Proxy,
}

impl<F: ClientProtocolFixture> Proxied<F> {
    pub fn new(inner: F, options: proxy::Options) -> Proxied<F> {
        let proxy = proxy::Proxy::start(inner.socket(), options).expect("proxy");
        Proxied { inner, proxy }
    }
}

impl<F: ClientProtocolFixture> ClientProtocolFixture for Proxied<F> {
    fn socket(&self) -> PathBuf {
        self.proxy.socket().to_path_buf()
    }
    fn emit(&mut self) -> Result<(), String> {
        self.inner.emit()
    }
    fn burst(&mut self, n: usize) -> Result<(), String> {
        self.inner.burst(n)
    }
    fn restart(&mut self) -> Result<(), String> {
        self.inner.restart()
    }
    fn retry_item(&mut self) -> Result<String, String> {
        self.inner.retry_item()
    }
    fn terminal_instance(&self) -> String {
        self.inner.terminal_instance()
    }
}

/// The negative check of gate 8 P9: the fake daemon with event ids counted
/// from 1 again (seen through a proxy that renumbers them). CLP-4 must fail.
pub fn fake_with_ids_from_one() -> Proxied<FakeDaemonFixture> {
    let fake = FakeDaemonFixture::new();
    let base = fake.base();
    let transform: proxy::Transform = Arc::new(move |_, direction, line| {
        let base = base.load(Ordering::SeqCst);
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&line) else {
            return vec![line];
        };
        let shift = |v: &mut serde_json::Value, up: bool| {
            if let Some(n) = v.as_u64() {
                *v = serde_json::json!(if up { n + base } else { n.saturating_sub(base) });
            }
        };
        match (direction, value["type"].as_str()) {
            (proxy::Direction::ToClient, Some("event")) => {
                shift(&mut value["data"]["event_id"], false)
            }
            (proxy::Direction::ToClient, Some("fleet")) => {
                shift(&mut value["data"]["fleet"]["as_of_event_id"], false)
            }
            (proxy::Direction::ToServer, Some("subscribe_events")) => {
                shift(&mut value["data"]["after_event_id"], true)
            }
            _ => return vec![line],
        }
        vec![value.to_string()]
    });
    Proxied::new(fake, proxy::Options::rewrite(transform))
}
