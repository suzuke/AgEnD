//! Fake implementations of every `agend_core::traits` trait: `Driver`,
//! `Forge`, `Store`, `Runtime`, `Notifier`, `Clock`, `Runner`.
//!
//! Every fake is:
//! - deterministic: ids, cursors, SHAs and pids come from counters, never
//!   from time or randomness;
//! - inspectable: `calls()` returns every trait call in order;
//! - scriptable: `fail_next(operation, message)` makes the next call of that
//!   trait method return `FakeError`, plus per-fake setup methods.
//!
//! Each fake passes the matching suite in `crate::contract`; behaviour the
//! contract does not pin (for example stopping an unknown holder) is
//! documented on the fake.
//!
//! Must NOT: be used outside tests, sleep, or read the system clock.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Mutex, MutexGuard};

mod clock;
mod driver;
mod forge;
mod notifier;
mod runner;
mod runtime;
mod store;

pub use clock::FakeClock;
pub use driver::{DriverCall, FakeBackend, FakeDriver};
pub use forge::{FakeForge, ForgeCall};
pub use notifier::FakeNotifier;
pub use runner::{FakeRunner, RunnerCall, ScriptedCommand};
pub use runtime::{FakeHolders, FakeRuntime, RuntimeCall};
pub use store::{FakeStore, FakeStoreFile, StoreCall};

/// Error returned by every fake: a scripted failure or a precondition the
/// fake enforces (unknown branch, unknown instance, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeError {
    pub operation: &'static str,
    pub message: String,
}

impl FakeError {
    pub(crate) fn new(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            operation,
            message: message.into(),
        }
    }
}

impl fmt::Display for FakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.operation, self.message)
    }
}

impl std::error::Error for FakeError {}

/// Queue of scripted failures, keyed by trait method name.
#[derive(Debug)]
pub(crate) struct Failures {
    operations: &'static [&'static str],
    queue: VecDeque<FakeError>,
}

impl Failures {
    pub(crate) const fn new(operations: &'static [&'static str]) -> Self {
        Self {
            operations,
            queue: VecDeque::new(),
        }
    }

    /// Panics on an operation name the fake does not have, so a typo in a
    /// test cannot silently script nothing.
    pub(crate) fn push(&mut self, operation: &str, message: &str) {
        let Some(known) = self.operations.iter().find(|op| **op == operation) else {
            panic!(
                "unknown fake operation `{operation}`; expected one of {:?}",
                self.operations
            );
        };
        self.queue.push_back(FakeError::new(known, message));
    }

    pub(crate) fn take(&mut self, operation: &str) -> Option<FakeError> {
        let index = self.queue.iter().position(|e| e.operation == operation)?;
        self.queue.remove(index)
    }
}

/// Locks a fake's state; a poisoned lock (a panicking test) still yields the
/// data so later assertions can inspect it.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_are_taken_per_operation_in_order() {
        let mut failures = Failures::new(&["a", "b"]);
        failures.push("a", "first");
        failures.push("b", "other");
        failures.push("a", "second");
        assert_eq!(failures.take("a").unwrap().message, "first");
        assert_eq!(failures.take("a").unwrap().message, "second");
        assert!(failures.take("a").is_none());
        assert_eq!(failures.take("b").unwrap().to_string(), "b: other");
    }

    #[test]
    #[should_panic(expected = "unknown fake operation `merge`")]
    fn unknown_operation_names_panic() {
        Failures::new(&["merge_if_head_is"]).push("merge", "typo");
    }
}
