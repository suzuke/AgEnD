//! Clock mutants: a `FakeClock` read through a broken `now_unix_ms`. Time
//! still passes and the UTC reference is the fake's own time.

use std::sync::atomic::{AtomicU64, Ordering};

use agend_core::traits::Clock;
use agend_testkit::contract::clock::{self, ClockFixture};
use agend_testkit::fakes::FakeClock;

use super::Mutant;

pub struct M {
    time: FakeClock,
    reads: AtomicU64,
    now: fn(&M) -> u64,
}

impl M {
    fn new(now: fn(&M) -> u64) -> Self {
        Self {
            time: FakeClock::default(),
            reads: AtomicU64::new(0),
            now,
        }
    }
}

impl Clock for M {
    fn now_unix_ms(&self) -> u64 {
        (self.now)(self)
    }
}

impl ClockFixture for M {
    type Clock = Self;
    fn clock(&self) -> &Self {
        self
    }
    fn let_time_pass(&self) {
        self.time.advance(1);
    }
    fn utc_now_unix_ms(&self) -> u64 {
        self.time.peek()
    }
}

pub fn mutants() -> Vec<Mutant> {
    vec![
        // CLK-1: seconds instead of milliseconds.
        Mutant {
            rule: "CLK-1",
            name: "Seconds",
            run: |name| clock::run(name, || M::new(|m| m.time.peek() / 1_000)),
        },
        // CLK-2: jitters 5 ms back and forth (e.g. an unsynchronised source).
        Mutant {
            rule: "CLK-2",
            name: "Jitters",
            run: |name| {
                clock::run(name, || {
                    M::new(|m| {
                        let even = m.reads.fetch_add(1, Ordering::SeqCst) % 2 == 0;
                        m.time.peek() + if even { 5 } else { 0 }
                    })
                })
            },
        },
        // CLK-3 (verifier r2 C2): local time (UTC+8) instead of UTC.
        Mutant {
            rule: "CLK-3",
            name: "LocalTimeOffset",
            run: |name| clock::run(name, || M::new(|m| m.time.peek() + 8 * 3_600_000)),
        },
        // CLK-4 (verifier r2 C1): frozen at the start time.
        Mutant {
            rule: "CLK-4",
            name: "Frozen",
            run: |name| clock::run(name, || M::new(|_| FakeClock::DEFAULT_START_UNIX_MS)),
        },
    ]
}
