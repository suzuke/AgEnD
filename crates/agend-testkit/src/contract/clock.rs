//! `Clock` contract (rules CLK-1..4 in CONTRACTS.md): the value is unix time
//! in milliseconds (not seconds or microseconds), in UTC (it matches a
//! reference read outside the implementation), never goes backwards between
//! reads, and moves forward as time passes (it is not frozen).
//!
//! Not pinned: resolution below one millisecond.

use agend_core::traits::Clock;

use super::{Case, CaseResult, Report, ensure, run_suite};

/// 2020-01-01T00:00:00Z and 2100-01-01T00:00:00Z in unix milliseconds.
pub const PLAUSIBLE_UNIX_MS: std::ops::Range<u64> = 1_577_836_800_000..4_102_444_800_000;
/// How far a reading may be from [`ClockFixture::utc_now_unix_ms`]. A time
/// zone offset is at least 15 minutes, so a second catches every one.
pub const UTC_TOLERANCE_MS: u64 = 1_000;
/// [`ClockFixture::let_time_pass`] calls within which the clock must move.
pub const ADVANCE_WITHIN_CALLS: usize = 10;

pub trait ClockFixture {
    type Clock: Clock;

    fn clock(&self) -> &Self::Clock;

    /// Lets at least one millisecond pass (a real clock sleeps, a fake
    /// advances).
    fn let_time_pass(&self);

    /// Unix milliseconds now, UTC, read outside the implementation (a real
    /// clock: `SystemTime::now()`; a fake: the time the test set).
    fn utc_now_unix_ms(&self) -> u64;
}

pub fn cases<F: ClockFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            rule: "CLK-1",
            name: "reads_unix_milliseconds",
            check: |fx| reads_unix_milliseconds(&fx),
        },
        Case {
            rule: "CLK-2",
            name: "never_goes_backwards",
            check: |fx| never_goes_backwards(&fx),
        },
        Case {
            rule: "CLK-3",
            name: "reads_utc",
            check: |fx| reads_utc(&fx),
        },
        Case {
            rule: "CLK-4",
            name: "moves_forward_as_time_passes",
            check: |fx| moves_forward_as_time_passes(&fx),
        },
    ]
}

pub fn run<F: ClockFixture>(implementation: &str, make: impl FnMut() -> F) -> Report {
    run_suite("Clock", implementation, &cases::<F>(), make)
}

fn reads_unix_milliseconds<F: ClockFixture>(fx: &F) -> CaseResult {
    let now = fx.clock().now_unix_ms();
    ensure(PLAUSIBLE_UNIX_MS.contains(&now), || {
        format!("{now} is not unix milliseconds between 2020 and 2100")
    })
}

fn never_goes_backwards<F: ClockFixture>(fx: &F) -> CaseResult {
    let mut previous = fx.clock().now_unix_ms();
    for _ in 0..100 {
        fx.let_time_pass();
        let now = fx.clock().now_unix_ms();
        ensure(now >= previous, || {
            format!("clock went backwards: {previous} then {now}")
        })?;
        previous = now;
    }
    Ok(())
}

fn reads_utc<F: ClockFixture>(fx: &F) -> CaseResult {
    for _ in 0..3 {
        let before = fx.utc_now_unix_ms();
        let now = fx.clock().now_unix_ms();
        let after = fx.utc_now_unix_ms();
        let low = before.saturating_sub(UTC_TOLERANCE_MS);
        let high = after.saturating_add(UTC_TOLERANCE_MS);
        ensure((low..=high).contains(&now), || {
            format!(
                "read {now}, but UTC was {before}..{after} (tolerance {UTC_TOLERANCE_MS} ms, off by {} ms)",
                now as i128 - before as i128
            )
        })?;
        fx.let_time_pass();
    }
    Ok(())
}

fn moves_forward_as_time_passes<F: ClockFixture>(fx: &F) -> CaseResult {
    let start = fx.clock().now_unix_ms();
    for _ in 0..ADVANCE_WITHIN_CALLS {
        fx.let_time_pass();
        if fx.clock().now_unix_ms() > start {
            return Ok(());
        }
    }
    Err(format!(
        "clock stayed at {start} after letting time pass {ADVANCE_WITHIN_CALLS} times"
    ))
}
