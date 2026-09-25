//! `Driver` contract (rules DRV-1..8 in CONTRACTS.md): delivery to a
//! running, idle instance is accepted and eventually completes a turn;
//! events are resumable from any cursor they returned (reconnect backfill:
//! exactly the newer events, reading does not consume them); unknown
//! instances are errors.
//!
//! Not pinned: whether a delivery is confirmed (some paths cannot confirm,
//! docs/architecture/delivery.md); busy behaviour; unknown cursors; whether
//! events of different instances are kept apart (one instance per fixture).

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::time::Duration;

use agend_core::model::DeliveryState;
use agend_core::policy::busy::BusyLevel;
use agend_core::traits::{AgentMessage, Driver, DriverEvent, DriverEventKind};

use super::{Case, CaseResult, Report, ensure, eventually, ok, run_suite};
use crate::block_on;

/// Default for [`DriverFixture::turn_timeout`]: how long a real driver may
/// take to finish a turn on a fake agent.
pub const TURN_TIMEOUT: Duration = Duration::from_secs(10);

pub trait DriverFixture {
    type Driver: Driver<Error = Self::Error>;
    type Error: Send + Debug;

    fn driver(&self) -> &Self::Driver;

    /// An instance that is running and idle.
    fn instance_id(&self) -> &str;

    /// How long a delivered message may take to complete its turn.
    fn turn_timeout(&self) -> Duration {
        TURN_TIMEOUT
    }
}

pub fn cases<F: DriverFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            rule: "DRV-1",
            name: "delivery_to_idle_instance_is_sent",
            check: delivery_to_idle_instance_is_sent,
        },
        Case {
            rule: "DRV-2",
            name: "delivery_to_unknown_instance_is_an_error",
            check: delivery_to_unknown_instance_is_an_error,
        },
        Case {
            rule: "DRV-3",
            name: "delivered_message_completes_a_turn",
            check: delivered_message_completes_a_turn,
        },
        Case {
            rule: "DRV-4",
            name: "cursors_are_unique",
            check: cursors_are_unique,
        },
        Case {
            rule: "DRV-5",
            name: "events_after_a_cursor_are_only_newer_ones",
            check: events_after_a_cursor_are_only_newer_ones,
        },
        Case {
            rule: "DRV-6",
            name: "events_after_a_cursor_are_all_newer_ones",
            check: events_after_a_cursor_are_all_newer_ones,
        },
        Case {
            rule: "DRV-7",
            name: "replay_from_a_cursor_only_grows",
            check: replay_from_a_cursor_only_grows,
        },
        Case {
            rule: "DRV-8",
            name: "events_of_unknown_instance_are_an_error",
            check: events_of_unknown_instance_are_an_error,
        },
    ]
}

pub fn run<F: DriverFixture>(implementation: &str, make: impl FnMut() -> F) -> Report {
    run_suite("Driver", implementation, &cases::<F>(), make)
}

const UNKNOWN_INSTANCE: &str = "contract-no-such-instance";

fn message(id: &str) -> AgentMessage {
    AgentMessage {
        id: id.into(),
        from: "operator".into(),
        task_id: Some("T-contract".into()),
        body: "Reply with one word: ok".into(),
    }
}

fn deliver<F: DriverFixture>(fx: &F, id: &str) -> CaseResult {
    let receipt = ok(
        "deliver",
        block_on(
            fx.driver()
                .deliver(fx.instance_id(), &message(id), BusyLevel::Queue),
        ),
    )?;
    ensure(
        matches!(
            receipt.state,
            DeliveryState::Sent | DeliveryState::Confirmed
        ),
        || format!("idle delivery must be Sent or Confirmed, got {receipt:?}"),
    )
}

fn events<F: DriverFixture>(fx: &F, after: Option<&str>) -> Result<Vec<DriverEvent>, String> {
    ok(
        "events",
        block_on(fx.driver().events(fx.instance_id(), after)),
    )
}

/// Delivers `id` and waits for a `TurnCompleted`; returns all events so far.
fn deliver_and_finish<F: DriverFixture>(fx: &F, id: &str) -> Result<Vec<DriverEvent>, String> {
    let before = events(fx, None)?.len();
    deliver(fx, id)?;
    let timeout = fx.turn_timeout();
    eventually(timeout, || {
        let all = events(fx, None)?;
        let done = all[before.min(all.len())..]
            .iter()
            .any(|e| matches!(e.kind, DriverEventKind::TurnCompleted { .. }));
        Ok(done.then_some(all))
    })?
    .ok_or_else(|| format!("no TurnCompleted within {timeout:?} after delivering {id}"))
}

fn delivery_to_idle_instance_is_sent<F: DriverFixture>(fx: &F) -> CaseResult {
    deliver(fx, "m-contract-sent")
}

fn delivery_to_unknown_instance_is_an_error<F: DriverFixture>(fx: &F) -> CaseResult {
    let result = block_on(fx.driver().deliver(
        UNKNOWN_INSTANCE,
        &message("m-contract-unknown"),
        BusyLevel::Queue,
    ));
    ensure(result.is_err(), || {
        format!("expected an error for an unknown instance, got {result:?}")
    })
}

fn delivered_message_completes_a_turn<F: DriverFixture>(fx: &F) -> CaseResult {
    deliver_and_finish(fx, "m-contract-turn").map(|_| ())
}

fn cursors_are_unique<F: DriverFixture>(fx: &F) -> CaseResult {
    deliver_and_finish(fx, "m-contract-c1")?;
    let all = deliver_and_finish(fx, "m-contract-c2")?;
    let unique: BTreeSet<&str> = all.iter().map(|e| e.cursor.as_str()).collect();
    ensure(unique.len() == all.len(), || {
        format!("duplicate cursors in {all:?}")
    })
}

fn events_after_a_cursor_are_only_newer_ones<F: DriverFixture>(fx: &F) -> CaseResult {
    let first = deliver_and_finish(fx, "m-contract-a1")?;
    let last = first
        .last()
        .map(|e| e.cursor.clone())
        .ok_or("a completed turn produced no events")?;
    deliver_and_finish(fx, "m-contract-a2")?;
    let newer = events(fx, Some(&last))?;
    let old: BTreeSet<&str> = first.iter().map(|e| e.cursor.as_str()).collect();
    ensure(
        newer.iter().all(|e| !old.contains(e.cursor.as_str())),
        || format!("events after {last} repeat the cursor or older ones: {newer:?}"),
    )
}

/// Backfill after a reconnect: from every cursor, exactly the events that
/// followed it (none dropped, none cut off, same order).
fn events_after_a_cursor_are_all_newer_ones<F: DriverFixture>(fx: &F) -> CaseResult {
    deliver_and_finish(fx, "m-contract-b1")?;
    deliver_and_finish(fx, "m-contract-b2")?;
    let all = events(fx, None)?;
    for (index, event) in all.iter().enumerate() {
        let newer = events(fx, Some(&event.cursor))?;
        let expected = &all[index + 1..];
        ensure(newer == expected, || {
            format!(
                "events after {} must be the {} events that followed it: expected {expected:?}, got {newer:?}",
                event.cursor,
                expected.len()
            )
        })?;
    }
    Ok(())
}

fn replay_from_a_cursor_only_grows<F: DriverFixture>(fx: &F) -> CaseResult {
    let all = deliver_and_finish(fx, "m-contract-r1")?;
    let first = all.first().map(|e| e.cursor.clone()).ok_or("no events")?;
    let once = events(fx, Some(&first))?;
    let twice = events(fx, Some(&first))?;
    ensure(twice.starts_with(&once), || {
        format!("replaying after {first} changed history: {once:?} then {twice:?}")
    })
}

fn events_of_unknown_instance_are_an_error<F: DriverFixture>(fx: &F) -> CaseResult {
    let result = block_on(fx.driver().events(UNKNOWN_INSTANCE, None));
    ensure(result.is_err(), || {
        format!("expected an error for an unknown instance, got {result:?}")
    })
}
