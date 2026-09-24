use std::sync::atomic::{AtomicU64, Ordering};

use agend_core::traits::Clock;

/// Manually advanced clock. Time never moves backwards: `set` to an earlier
/// instant panics, which is the `Clock` contract's monotonicity rule.
#[derive(Debug)]
pub struct FakeClock {
    now_unix_ms: AtomicU64,
    reads: AtomicU64,
}

impl FakeClock {
    /// 2026-09-21T13:46:40Z: a plausible "now" in unix milliseconds.
    pub const DEFAULT_START_UNIX_MS: u64 = 1_790_000_000_000;

    pub fn new(start_unix_ms: u64) -> Self {
        Self {
            now_unix_ms: AtomicU64::new(start_unix_ms),
            reads: AtomicU64::new(0),
        }
    }

    pub fn advance(&self, ms: u64) {
        self.now_unix_ms.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn set(&self, unix_ms: u64) {
        let previous = self.now_unix_ms.swap(unix_ms, Ordering::SeqCst);
        assert!(
            unix_ms >= previous,
            "FakeClock::set would move time backwards ({previous} -> {unix_ms})"
        );
    }

    /// How many times `now_unix_ms` was called.
    pub fn reads(&self) -> u64 {
        self.reads.load(Ordering::SeqCst)
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new(Self::DEFAULT_START_UNIX_MS)
    }
}

impl Clock for FakeClock {
    fn now_unix_ms(&self) -> u64 {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.now_unix_ms.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advances_only_when_told_and_counts_reads() {
        let clock = FakeClock::new(1_000);
        assert_eq!(clock.now_unix_ms(), 1_000);
        assert_eq!(clock.now_unix_ms(), 1_000);
        clock.advance(250);
        assert_eq!(clock.now_unix_ms(), 1_250);
        clock.set(2_000);
        assert_eq!(clock.now_unix_ms(), 2_000);
        assert_eq!(clock.reads(), 4);
    }

    #[test]
    #[should_panic(expected = "backwards")]
    fn setting_an_earlier_time_panics() {
        FakeClock::new(5_000).set(4_999);
    }
}
