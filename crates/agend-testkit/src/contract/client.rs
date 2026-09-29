//! Client protocol contract (rules CLP-1..12 in CONTRACTS.md, gate 8 P9;
//! CLP-13..17, gate 9 P10; CLP-18..20, gate 11 B P1, P5, P6):
//! what TUI and CLI tests rely on from the testkit fake daemon must hold for
//! the real `agend daemon` too. The same cases run against [`FakeDaemonFixture`]
//! and against the real daemon (`crates/agend/tests/client_protocol.rs`).
//!
//! The driver is [`ProbeClient`], not `agend-client`, so a client bug cannot
//! hide behind the same code. Every message is a core protocol type encoded
//! with `serde_json` (#1493).
//!
//! Not pinned: agent commands other than `status`, `send` and `inbox` — the
//! fake serves them, the real daemon answers `not_supported` until gate 10;
//! a successful `daemon_restart` (the CLI tests pin it against both); a
//! codex instance's `terminal_input` (`not_supported` until U17; tested on
//! each side, since the fixture has no codex instance).
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
    AgentCommand, AgentState, AnswerAskData, AttentionAction, AttentionRequiredData,
    ClientCommandData, ClientHello, ClientRequest, ClientResponse, CommandResult, DaemonEvent,
    EventData, FleetView, InstanceData, InstanceView, MAX_MESSAGE_BYTES, MessageLevel,
    OperatorCommand, OperatorData, RequestIdData, ResolveAttentionData, SUPPORTED_VERSIONS,
    SubscribeEventsData, TaskChangedData, TerminalInputData, V1, error_code,
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
    /// Two instances that can message each other as agents (gate 9).
    fn agents(&self) -> (String, String);
    /// A name no instance has yet (a new one per call).
    fn fresh_name(&mut self) -> String;
    /// Makes `instance`'s terminal print something new (the real daemon's
    /// counter prints every second by itself).
    fn make_output(&mut self, instance: &str) -> Result<(), String>;
    /// What reached `instance`'s terminal as input, once `expect` shows up
    /// (or after [`WITHIN`]): the fake's recorded input, the real
    /// terminal's screen (the PTY echoes what is typed).
    fn typed(&mut self, instance: &str, expect: &str) -> Result<String, String>;
    /// A `failed` instance (no live terminal).
    fn stopped_instance(&mut self) -> Result<String, String>;
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
        Case {
            rule: "CLP-13",
            name: "agent_and_operator_requests_are_refused_the_other_way",
            check: |mut fx| permissions_both_ways(&mut fx),
        },
        Case {
            rule: "CLP-14",
            name: "the_operator_adds_and_removes_instances",
            check: |mut fx| operator_adds_and_removes_instances(&mut fx),
        },
        Case {
            rule: "CLP-15",
            name: "a_restart_to_a_broken_binary_changes_nothing",
            check: |fx| restart_to_a_broken_binary_changes_nothing(&fx),
        },
        Case {
            rule: "CLP-16",
            name: "task_cancel_is_not_supported_and_changes_nothing",
            check: |fx| task_cancel_changes_nothing(&fx),
        },
        Case {
            rule: "CLP-17",
            name: "one_message_per_id_and_inbox_after_an_own_message",
            check: |fx| one_message_per_id(&fx),
        },
        Case {
            rule: "CLP-18",
            name: "subscribing_again_after_new_output_shows_it",
            check: |mut fx| subscribing_again_shows_new_output(&mut fx),
        },
        Case {
            rule: "CLP-19",
            name: "no_terminal_for_an_unknown_instance_ends_the_old_stream",
            check: |mut fx| no_terminal_ends_the_old_stream(&mut fx),
        },
        Case {
            rule: "CLP-20",
            name: "only_the_operator_types_and_only_into_a_terminal",
            check: |mut fx| only_the_operator_types(&mut fx),
        },
        Case {
            rule: "CLP-21",
            name: "typing_into_a_failed_instance_is_no_terminal",
            check: |mut fx| typing_into_a_failed_instance(&mut fx),
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
    send(&mut c, &terminal_input(NO_INSTANCE, b"hi"))?;
    let (code, _) = next_error(&mut c, WITHIN)?;
    ensure(code == error_code::NO_TERMINAL, || {
        format!("terminal_input for {NO_INSTANCE} got {code}, expected no_terminal")
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

// ---- gate 9 cases ----

/// A binary that cannot be run (CLP-13, CLP-15).
const NO_BINARY: &str = "/nonexistent/agend-clp";

fn command_request(request_id: &str, command: AgentCommand) -> ClientRequest {
    ClientRequest::Command {
        data: ClientCommandData {
            request_id: request_id.into(),
            command,
        },
    }
}

fn operator_request(request_id: &str, command: OperatorCommand) -> ClientRequest {
    ClientRequest::Operator {
        data: OperatorData {
            request_id: request_id.into(),
            command,
        },
    }
}

/// Sends `request` and returns the reply carrying `request_id`.
fn ask(
    c: &mut ProbeClient,
    request_id: &str,
    request: &ClientRequest,
) -> Result<ClientResponse, String> {
    send(c, request)?;
    let mut skipped = Vec::new();
    until(
        c,
        WITHIN,
        |r| match r {
            ClientResponse::CommandResult { data } => data.request_id == request_id,
            ClientResponse::Error { data } => data.request_id.as_deref() == Some(request_id),
            _ => false,
        },
        &mut skipped,
    )
}

fn agent_client<F: ClientProtocolFixture>(fx: &F, name: &str) -> Result<ProbeClient, String> {
    ProbeClient::hello(&fx.socket(), Some(name))
        .map(|(c, _)| c)
        .map_err(io("agent hello"))
}

fn add_command(name: &str) -> OperatorCommand {
    OperatorCommand::InstanceAdd {
        instance_id: name.into(),
        backend: "claude".into(),
        working_directory: None,
        program: Some("/bin/sh".into()),
        args: vec!["-c".into(), "sleep 60".into()],
    }
}

fn result_of(reply: &ClientResponse) -> Option<&CommandResult> {
    match reply {
        ClientResponse::CommandResult { data } => Some(&data.result),
        _ => None,
    }
}

fn permissions_both_ways<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    let (a, b) = fx.agents();
    let fresh = fx.fresh_name();
    let mut op = operator(fx)?;
    let before = get_fleet(&mut op, "clp-13")?;
    let mut agent = agent_client(fx, &a)?;
    let refused = [
        add_command(&fresh),
        OperatorCommand::InstanceRemove {
            instance_id: b.clone(),
        },
        OperatorCommand::DaemonRestart {
            binary: Some(NO_BINARY.into()),
        },
        OperatorCommand::TaskCancel {
            task_id: "t-clp".into(),
        },
    ];
    for (n, command) in refused.into_iter().enumerate() {
        let id = format!("clp-13a{n}");
        let reply = ask(&mut agent, &id, &operator_request(&id, command.clone()))?;
        ensure(error_code_of(&reply) == Some(error_code::FORBIDDEN), || {
            format!("agent {a} sending {command:?} got {reply:?}, expected forbidden")
        })?;
    }
    let commands = [
        AgentCommand::Status,
        AgentCommand::Send {
            to: b.clone(),
            message: "clp-13".into(),
            level: None,
            message_id: None,
        },
        AgentCommand::Done {
            task_id: "t-clp".into(),
            identity: None,
        },
    ];
    for (n, command) in commands.into_iter().enumerate() {
        let id = format!("clp-13b{n}");
        let reply = ask(&mut op, &id, &command_request(&id, command.clone()))?;
        ensure(error_code_of(&reply) == Some(error_code::FORBIDDEN), || {
            format!("the operator sending {command:?} got {reply:?}, expected forbidden")
        })?;
    }
    let after = get_fleet(&mut op, "clp-13c")?;
    ensure(after == before, || {
        format!("refused requests changed the fleet view:\n  before {before:?}\n  after  {after:?}")
    })
}

fn operator_adds_and_removes_instances<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    let name = fx.fresh_name();
    let mut op = operator(fx)?;
    let reply = ask(
        &mut op,
        "clp-14a",
        &operator_request("clp-14a", add_command(&name)),
    )?;
    let Some(CommandResult::InstanceAdded { data }) = result_of(&reply) else {
        return Err(format!(
            "instance_add {name} got {reply:?}, expected instance_added"
        ));
    };
    ensure(
        data.instance_id == name && !data.working_directory.is_empty(),
        || format!("instance_added {data:?} does not name {name} and its directory"),
    )?;
    let dir = data.working_directory.clone();
    let view = get_fleet(&mut op, "clp-14b")?;
    let listed = view.instances.iter().find(|i| i.instance_id == name);
    ensure(
        listed.is_some_and(|i| i.working_directory.as_deref() == Some(dir.as_str())),
        || format!("the fleet view shows {listed:?}, expected {name} in {dir}"),
    )?;
    let reply = ask(
        &mut op,
        "clp-14c",
        &operator_request("clp-14c", add_command(&name)),
    )?;
    ensure(
        error_code_of(&reply) == Some(error_code::INSTANCE_EXISTS),
        || format!("adding {name} again got {reply:?}, expected instance_exists"),
    )?;
    let reply = ask(
        &mut op,
        "clp-14d",
        &operator_request("clp-14d", add_command("Not_A_Name")),
    )?;
    ensure(
        error_code_of(&reply) == Some(error_code::INVALID_REQUEST),
        || format!("adding an invalid name got {reply:?}, expected invalid_request"),
    )?;
    let remove = || OperatorCommand::InstanceRemove {
        instance_id: name.clone(),
    };
    let reply = ask(&mut op, "clp-14e", &operator_request("clp-14e", remove()))?;
    ensure(result_of(&reply) == Some(&CommandResult::Accepted), || {
        format!("removing {name} got {reply:?}, expected accepted")
    })?;
    let view = get_fleet(&mut op, "clp-14f")?;
    ensure(
        !view.instances.iter().any(|i| i.instance_id == name),
        || format!("{name} is still in the fleet view after it was removed"),
    )?;
    let reply = ask(&mut op, "clp-14g", &operator_request("clp-14g", remove()))?;
    ensure(
        error_code_of(&reply) == Some(error_code::UNKNOWN_INSTANCE),
        || format!("removing {name} again got {reply:?}, expected unknown_instance"),
    )
}

fn restart_to_a_broken_binary_changes_nothing<F: ClientProtocolFixture>(fx: &F) -> CaseResult {
    let mut op = operator(fx)?;
    let before = get_fleet(&mut op, "clp-15")?;
    subscribe(&mut op, Some(before.as_of_event_id))?;
    let restart = OperatorCommand::DaemonRestart {
        binary: Some(NO_BINARY.into()),
    };
    let reply = ask(&mut op, "clp-15a", &operator_request("clp-15a", restart))?;
    ensure(
        error_code_of(&reply) == Some(error_code::PREFLIGHT_FAILED),
        || format!("daemon_restart to {NO_BINARY} got {reply:?}, expected preflight_failed"),
    )?;
    let quiet = collect(&mut op, QUIET)?;
    ensure(quiet.events.is_empty() && !quiet.closed, || {
        format!(
            "after the failed restart: events {:?}, connection closed {}",
            ids(&quiet.events),
            quiet.closed
        )
    })?;
    let after = get_fleet(&mut op, "clp-15b")?;
    ensure(after == before, || {
        format!("the fleet view changed:\n  before {before:?}\n  after  {after:?}")
    })
}

fn task_cancel_changes_nothing<F: ClientProtocolFixture>(fx: &F) -> CaseResult {
    let mut op = operator(fx)?;
    let before = get_fleet(&mut op, "clp-16")?;
    subscribe(&mut op, Some(before.as_of_event_id))?;
    let cancel = OperatorCommand::TaskCancel {
        task_id: "t-clp".into(),
    };
    let reply = ask(&mut op, "clp-16a", &operator_request("clp-16a", cancel))?;
    ensure(
        error_code_of(&reply) == Some(error_code::NOT_SUPPORTED),
        || format!("task_cancel got {reply:?}, expected not_supported (gate 10)"),
    )?;
    let quiet = collect(&mut op, QUIET)?;
    ensure(quiet.events.is_empty(), || {
        format!("task_cancel made events {:?}", ids(&quiet.events))
    })?;
    let after = get_fleet(&mut op, "clp-16b")?;
    ensure(after == before, || {
        format!("the fleet view changed:\n  before {before:?}\n  after  {after:?}")
    })
}

fn inbox_of(c: &mut ProbeClient, id: &str, after: Option<&str>) -> Result<ClientResponse, String> {
    let command = AgentCommand::Inbox {
        after_message_id: after.map(str::to_owned),
    };
    ask(c, id, &command_request(id, command))
}

fn message_ids(reply: &ClientResponse) -> Option<Vec<String>> {
    match result_of(reply)? {
        CommandResult::Messages { data } => {
            Some(data.messages.iter().map(|m| m.message_id.clone()).collect())
        }
        _ => None,
    }
}

fn one_message_per_id<F: ClientProtocolFixture>(fx: &F) -> CaseResult {
    const X: &str = "0c1f7e6a-3b1d-4e2a-9c4b-5d6e7f809a1b";
    const Y: &str = "1d2e8f7b-4c2e-4f3b-8d5c-6e7f8091ab2c";
    let (a, b) = fx.agents();
    let mut from = agent_client(fx, &a)?;
    let mut to = agent_client(fx, &b)?;
    let send_as = |c: &mut ProbeClient, n: &str, id: &str, body: &str| {
        let command = AgentCommand::Send {
            to: b.clone(),
            message: body.into(),
            level: Some(MessageLevel::Queue),
            message_id: Some(id.into()),
        };
        ask(c, n, &command_request(n, command))
    };
    for (n, id, body) in [
        ("clp-17a", X, "one"),
        ("clp-17b", X, "one"),
        ("clp-17c", Y, "two"),
    ] {
        let reply = send_as(&mut from, n, id, body)?;
        ensure(result_of(&reply) == Some(&CommandResult::Accepted), || {
            format!("send {id} {body:?} ({n}) got {reply:?}, expected accepted")
        })?;
    }
    let huge = "h".repeat(MAX_MESSAGE_BYTES + 1);
    for (n, id, body) in [
        ("clp-17d", X, "not one"),
        ("clp-17e", "clp-not-a-uuid", "x"),
        (
            "clp-17j",
            "4a5b1c0e-7f5b-4c6d-9e8f-90a1b2c3d4e5",
            huge.as_str(),
        ),
    ] {
        let reply = send_as(&mut from, n, id, body)?;
        ensure(
            error_code_of(&reply) == Some(error_code::INVALID_REQUEST),
            || {
                format!(
                    "send {id} ({} bytes) got {reply:?}, expected invalid_request",
                    body.len()
                )
            },
        )?;
    }
    let all = inbox_of(&mut to, "clp-17f", None)?;
    ensure(message_ids(&all) == Some(vec![X.into(), Y.into()]), || {
        format!("{b}'s inbox is {all:?}, expected {X} once, then {Y}")
    })?;
    let after = inbox_of(&mut to, "clp-17g", Some(X))?;
    ensure(message_ids(&after) == Some(vec![Y.into()]), || {
        format!("{b}'s inbox after {X} is {after:?}, expected only {Y}")
    })?;
    for (c, n, cursor, whose) in [
        (
            &mut to,
            "clp-17h",
            "2e3f9a8c-5d3f-4a4c-9e6d-7f8091a2bc3d",
            "an unknown id",
        ),
        (&mut from, "clp-17i", X, "someone else's message"),
    ] {
        let reply = inbox_of(c, n, Some(cursor))?;
        ensure(
            error_code_of(&reply) == Some(error_code::UNKNOWN_MESSAGE),
            || format!("inbox after {whose} got {reply:?}, expected unknown_message"),
        )?;
    }
    Ok(())
}

// ---- gate 11 cases (terminal) ----

/// An instance no server has (CLP-10, CLP-19, CLP-20).
pub const NO_INSTANCE: &str = "clp-nobody";

pub fn terminal_input(instance: &str, bytes: &[u8]) -> ClientRequest {
    use base64::Engine;
    ClientRequest::TerminalInput {
        data: TerminalInputData {
            instance_id: instance.into(),
            bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        },
    }
}

fn subscribe_terminal(c: &mut ProbeClient, instance: &str) -> Result<(), String> {
    send(
        c,
        &ClientRequest::SubscribeTerminal {
            data: InstanceData {
                instance_id: instance.into(),
            },
        },
    )
}

/// The next screen or error of a terminal subscription (bytes and events
/// on the way are skipped).
fn terminal_reply(c: &mut ProbeClient) -> Result<ClientResponse, String> {
    let mut skipped = Vec::new();
    until(
        c,
        WITHIN,
        |r| {
            matches!(
                r,
                ClientResponse::TerminalSnapshot { .. } | ClientResponse::Error { .. }
            )
        },
        &mut skipped,
    )
}

fn screen_of(reply: ClientResponse, instance: &str) -> Result<String, String> {
    match reply {
        ClientResponse::TerminalSnapshot { data } if data.instance_id == instance => {
            Ok(data.screen)
        }
        other => Err(format!(
            "subscribe_terminal {instance} answered {other:?}, expected its screen"
        )),
    }
}

/// The text of the next `terminal_bytes` of `instance` (within [`WITHIN`]).
fn next_bytes(c: &mut ProbeClient, instance: &str) -> Result<String, String> {
    use base64::Engine;
    let mut skipped = Vec::new();
    let reply = until(
        c,
        WITHIN,
        |r| {
            matches!(
                r,
                ClientResponse::TerminalBytes { .. } | ClientResponse::Error { .. }
            )
        },
        &mut skipped,
    )?;
    let ClientResponse::TerminalBytes { data } = reply else {
        return Err(format!(
            "expected terminal_bytes of {instance}, got {reply:?}"
        ));
    };
    ensure(data.instance_id == instance, || {
        format!(
            "terminal_bytes of {}, expected {instance}",
            data.instance_id
        )
    })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data.bytes_base64)
        .map_err(|e| format!("terminal_bytes is not base64: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn subscribing_again_shows_new_output<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    let instance = fx.terminal_instance();
    let mut c = operator(fx)?;
    subscribe_terminal(&mut c, &instance)?;
    let first = screen_of(terminal_reply(&mut c)?, &instance)?;
    fx.make_output(&instance)?;
    let output = next_bytes(&mut c, &instance)?;
    let shown = output
        .lines()
        .map(|l| l.trim())
        .rfind(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned();
    subscribe_terminal(&mut c, &instance)?;
    let again = screen_of(terminal_reply(&mut c)?, &instance)?;
    ensure(again != first && again.contains(&shown), || {
        format!(
            "after terminal_bytes {output:?} the new screen does not show {shown:?}:\n  first {first:?}\n  again {again:?}"
        )
    })
}

fn no_terminal_ends_the_old_stream<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    let instance = fx.terminal_instance();
    let mut c = operator(fx)?;
    subscribe_terminal(&mut c, &instance)?;
    screen_of(terminal_reply(&mut c)?, &instance)?;
    subscribe_terminal(&mut c, NO_INSTANCE)?;
    let reply = terminal_reply(&mut c)?;
    let ClientResponse::Error { data } = &reply else {
        return Err(format!(
            "subscribe_terminal {NO_INSTANCE} answered {reply:?}, expected no_terminal"
        ));
    };
    ensure(
        data.code == error_code::NO_TERMINAL && data.request_id.is_none(),
        || format!("subscribe_terminal {NO_INSTANCE} answered {reply:?}, expected no_terminal"),
    )?;
    // The failed subscription replaced the old one: no more bytes of it.
    fx.make_output(&instance)?;
    let deadline = Instant::now() + Duration::from_millis(2500);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match c.recv_within(left) {
            Ok(Some(ClientResponse::TerminalBytes { data })) => {
                return Err(format!(
                    "terminal_bytes of {} still arrive after the failed subscription",
                    data.instance_id
                ));
            }
            Ok(Some(_)) => {}
            Ok(None) => return Err("the server closed the connection".into()),
            Err(e) if timed_out(&e) => break,
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    get_fleet(&mut c, "clp-19")
        .map(|_| ())
        .map_err(|e| format!("the connection did not stay usable: {e}"))
}

fn only_the_operator_types<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    const AGENT_TYPED: &str = "clp-agent-typed";
    const TYPED: &str = "clp-operator-typed";
    let instance = fx.terminal_instance();
    let mut agent = agent_client(fx, "clp-agent")?;
    for target in [instance.as_str(), NO_INSTANCE] {
        send(&mut agent, &terminal_input(target, AGENT_TYPED.as_bytes()))?;
        let (code, id) = next_error(&mut agent, WITHIN)?;
        ensure(code == error_code::FORBIDDEN && id.is_none(), || {
            format!("an agent typing into {target} got {code} ({id:?}), expected forbidden")
        })?;
    }
    let mut op = operator(fx)?;
    send(&mut op, &terminal_input(NO_INSTANCE, b"x"))?;
    let (code, _) = next_error(&mut op, WITHIN)?;
    ensure(code == error_code::NO_TERMINAL, || {
        format!("typing into {NO_INSTANCE} got {code}, expected no_terminal")
    })?;
    send(&mut op, &terminal_input(&instance, TYPED.as_bytes()))?;
    let quiet = collect(&mut op, QUIET)?;
    ensure(quiet.errors.is_empty() && !quiet.closed, || {
        format!(
            "typing into {instance} got errors {:?} (closed {})",
            quiet.errors, quiet.closed
        )
    })?;
    let typed = fx.typed(&instance, TYPED)?;
    ensure(
        typed.contains(TYPED) && !typed.contains(AGENT_TYPED),
        || format!("{instance} received {typed:?}, expected {TYPED:?} and nothing an agent typed"),
    )
}

fn typing_into_a_failed_instance<F: ClientProtocolFixture>(fx: &mut F) -> CaseResult {
    let stopped = fx.stopped_instance()?;
    let mut op = operator(fx)?;
    send(&mut op, &terminal_input(&stopped, b"x"))?;
    let (code, id) = next_error(&mut op, WITHIN)?;
    ensure(code == error_code::NO_TERMINAL && id.is_none(), || {
        format!("typing into failed {stopped} got {code} ({id:?}), expected no_terminal")
    })?;
    get_fleet(&mut op, "clp-21")
        .map(|_| ())
        .map_err(|e| format!("the connection did not stay usable: {e}"))
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
    names: u64,
    outputs: u64,
}

pub const FAKE_INSTANCE: &str = "clp-1";
/// The second instance of the fake (the other agent of CLP-13, CLP-17).
pub const FAKE_PEER: &str = "clp-2";
/// A `failed` instance of the fake (CLP-21).
pub const FAKE_STOPPED: &str = "clp-3";

impl FakeDaemonFixture {
    pub fn new() -> FakeDaemonFixture {
        let dir = TempDir::new("clp").expect("temp dir");
        let mut fx = FakeDaemonFixture {
            daemon: None,
            dir,
            base: Arc::new(AtomicU64::new(0)),
            items: 0,
            names: 0,
            outputs: 0,
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
        for id in [FAKE_INSTANCE, FAKE_PEER] {
            daemon.set_instance(InstanceView {
                instance_id: id.into(),
                team_id: "general".into(),
                backend: "claude".into(),
                state: AgentState::Unknown,
                working_directory: Some(format!("/fake/workspace/{id}")),
            });
        }
        daemon.set_instance(InstanceView {
            instance_id: FAKE_STOPPED.into(),
            team_id: "general".into(),
            backend: "claude".into(),
            state: AgentState::Failed,
            working_directory: Some(format!("/fake/workspace/{FAKE_STOPPED}")),
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

    fn agents(&self) -> (String, String) {
        (FAKE_INSTANCE.into(), FAKE_PEER.into())
    }

    fn fresh_name(&mut self) -> String {
        self.names += 1;
        format!("clp-n{}", self.names)
    }

    fn make_output(&mut self, instance: &str) -> Result<(), String> {
        self.outputs += 1;
        let line = format!("clp-output-{}\r\n", self.outputs);
        self.daemon().push_terminal_bytes(instance, line.as_bytes());
        Ok(())
    }

    fn typed(&mut self, instance: &str, _expect: &str) -> Result<String, String> {
        let typed: Vec<u8> = self
            .daemon()
            .terminal_inputs()
            .into_iter()
            .filter(|(id, _)| id == instance)
            .flat_map(|(_, bytes)| bytes)
            .collect();
        Ok(String::from_utf8_lossy(&typed).into_owned())
    }

    fn stopped_instance(&mut self) -> Result<String, String> {
        Ok(FAKE_STOPPED.into())
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
    fn agents(&self) -> (String, String) {
        self.inner.agents()
    }
    fn fresh_name(&mut self) -> String {
        self.inner.fresh_name()
    }
    fn make_output(&mut self, instance: &str) -> Result<(), String> {
        self.inner.make_output(instance)
    }
    fn typed(&mut self, instance: &str, expect: &str) -> Result<String, String> {
        self.inner.typed(instance, expect)
    }
    fn stopped_instance(&mut self) -> Result<String, String> {
        self.inner.stopped_instance()
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
