//! `Driver` contract (rules DRV-1..9 in CONTRACTS.md): delivery to a
//! running, idle instance is accepted and eventually completes a turn;
//! delivering the same message id again starts no second turn, also after
//! daemon restarts; events are resumable from any cursor they returned, also
//! by every new driver after a daemon restart ([`DriverFixture::restart`];
//! reconnect backfill: exactly the newer events, including those the agent
//! emitted while no daemon ran, [`DriverFixture::emit_while_down`]; reading
//! does not consume them); unknown instances are errors. The restart cases
//! run a whole daemon lifecycle ([`super::daemon_lifecycle`]).
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

use super::{Case, CaseResult, Report, daemon_lifecycle, ensure, eventually, ok, run_suite};
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

    /// A daemon restart: a new fixture whose driver is a new instance over
    /// the same backend and instance (the agent keeps running in its
    /// holder). Cases drop the old fixture before creating the next one, and
    /// call it several times on the fixture they got. A real fixture goes
    /// through the real persisted state (the holder and backend sockets, the
    /// DB), never a process-global static (CONTRACTS.md).
    fn restart(&self) -> Self;

    /// The backend acts while no daemon runs: the agent in its holder
    /// finishes a turn on its own, so the backend emits at least one event
    /// for [`instance_id`](Self::instance_id), one of them a
    /// `TurnCompleted`. Called on the fixture the case got, between
    /// dropping one restarted fixture and creating the next; it must not go
    /// through a driver (a real fixture talks to the fake agent directly).
    fn emit_while_down(&self);
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
            rule: "DRV-6",
            name: "every_boot_backfills_what_happened_while_down",
            check: every_boot_backfills_what_happened_while_down,
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
        Case {
            rule: "DRV-9",
            name: "same_message_id_makes_one_turn_across_restarts",
            check: same_message_id_makes_one_turn_across_restarts,
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

/// A daemon lifecycle ([`daemon_lifecycle`]) for the driver. Every boot
/// starts by backfilling from the cursor the previous daemon saw last (the
/// way a restarted daemon reconnects to the backend) and `boot` gets that
/// backfill; the next cursor is the newest event the boot saw. While no
/// daemon runs, the agent finishes a turn on its own
/// ([`DriverFixture::emit_while_down`]).
fn lifecycle<F: DriverFixture>(
    fx: &F,
    mut boot: impl FnMut(usize, &F, &[DriverEvent]) -> CaseResult,
) -> CaseResult {
    let mut cursor: Option<String> = None;
    daemon_lifecycle(
        || fx.restart(),
        |_| fx.emit_while_down(),
        |n, daemon| {
            let backfill = events(daemon, cursor.as_deref())?;
            boot(n, daemon, &backfill)?;
            let seen = events(daemon, cursor.as_deref())?;
            if let Some(last) = seen.last() {
                cursor = Some(last.cursor.clone());
            }
            Ok(())
        },
    )
}

fn turns(all: &[DriverEvent]) -> usize {
    all.iter()
        .filter(|e| matches!(e.kind, DriverEventKind::TurnCompleted { .. }))
        .count()
}

/// ARCHITECTURE process model rule 2: a restarted daemon backfills the
/// events of the downtime. At every boot the backfill from the previous
/// daemon's last cursor is exactly what followed that cursor, and after a
/// downtime it holds the turn the agent finished meanwhile; then the boot
/// completes a turn of its own.
fn every_boot_backfills_what_happened_while_down<F: DriverFixture>(fx: &F) -> CaseResult {
    let mut last_cursor: Option<String> = None;
    lifecycle(fx, |n, daemon, backfill| {
        let all = events(daemon, None)?;
        let expected = match &last_cursor {
            None => &all[..],
            Some(cursor) => {
                let index = all
                    .iter()
                    .position(|e| &e.cursor == cursor)
                    .ok_or_else(|| {
                        format!("the cursor {cursor} an earlier daemon saw is gone: {all:?}")
                    })?;
                &all[index + 1..]
            }
        };
        ensure(backfill == expected, || {
            format!(
                "the backfill after {last_cursor:?} must be the {} events that followed it: expected {expected:?}, got {backfill:?}",
                expected.len()
            )
        })?;
        if n > 1 {
            ensure(turns(backfill) > 0, || {
                format!(
                    "the turn the agent finished while the daemon was down is missing from the backfill after {last_cursor:?}: {backfill:?}"
                )
            })?;
        }
        let seen = deliver_and_finish(daemon, &format!("m-contract-boot{n}"))?;
        last_cursor = seen.last().map(|e| e.cursor.clone());
        Ok(())
    })
}

/// delivery.md: messages are idempotent by id, also across restarts (a
/// message resent after a crash). Boot 1 delivers the id and waits for its
/// turn; every boot then delivers it again: not an error, and no new turn
/// completes within the turn timeout.
fn same_message_id_makes_one_turn_across_restarts<F: DriverFixture>(fx: &F) -> CaseResult {
    const ID: &str = "m-contract-dup";
    lifecycle(fx, |n, daemon, _| {
        if n == 1 {
            deliver_and_finish(daemon, ID)?;
        }
        let before = turns(&events(daemon, None)?);
        ok(
            "deliver (same id again)",
            block_on(
                daemon
                    .driver()
                    .deliver(daemon.instance_id(), &message(ID), BusyLevel::Queue),
            ),
        )?;
        let again = eventually(daemon.turn_timeout(), || {
            let all = events(daemon, None)?;
            Ok((turns(&all) > before).then_some(all))
        })?;
        ensure(again.is_none(), || {
            format!("delivering {ID} again completed another turn: {again:?}")
        })
    })
}
