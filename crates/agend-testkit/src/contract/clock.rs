//! `Clock` contract: the value is unix time in milliseconds (not seconds or
//! microseconds) and never goes backwards between reads.

use agend_core::traits::Clock;

use super::{Case, CaseResult, Report, ensure, run_suite};

/// 2020-01-01T00:00:00Z and 2100-01-01T00:00:00Z in unix milliseconds.
pub const PLAUSIBLE_UNIX_MS: std::ops::Range<u64> = 1_577_836_800_000..4_102_444_800_000;

pub trait ClockFixture {
    type Clock: Clock;

    fn clock(&self) -> &Self::Clock;

    /// Lets time pass (a real clock sleeps, a fake advances).
    fn let_time_pass(&self);
}

pub fn cases<F: ClockFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            name: "reads_unix_milliseconds",
            check: reads_unix_milliseconds,
        },
        Case {
            name: "never_goes_backwards",
            check: never_goes_backwards,
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
