//! Contract test suites: one suite per `agend_core::traits` trait. A suite is
//! a list of named cases written against a fixture trait (for example
//! [`forge::ForgeFixture`]), so the same cases run against the fake now and
//! against the real implementation at its gate. That is what keeps fakes from
//! drifting away from production (v1 #1483).
//!
//! Each case gets a fresh fixture. [`run_all_fakes`] runs every suite
//! against the fakes in `crate::fakes`.
//!
//! Every case names the rule it checks (`DRV-1`, `STO-6`, ...). The rules
//! are numbered in `crates/agend-testkit/CONTRACTS.md`; a coverage test in
//! `tests/contract_teeth/main.rs` fails when a rule has no case or no deliberately
//! broken implementation that the suite rejects.
//!
//! Must NOT: hand-write wire shapes; build inputs with the real producer (#1493).

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

pub mod clock;
pub mod driver;
pub mod fakes;
pub mod forge;
pub mod notifier;
pub mod runner;
pub mod runtime;
pub mod store;

pub use fakes::run_all_fakes;

/// Outcome of one case: `Err` carries an English explanation.
pub type CaseResult = Result<(), String>;

/// One named check of one numbered contract rule (see CONTRACTS.md).
pub struct Case<F> {
    pub rule: &'static str,
    pub name: &'static str,
    pub check: fn(&F) -> CaseResult,
}

/// The result of one case.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub rule: &'static str,
    pub name: &'static str,
    pub result: CaseResult,
}

/// Results of running one suite against one implementation.
#[derive(Debug, Clone)]
pub struct Report {
    pub contract: &'static str,
    pub implementation: String,
    pub results: Vec<Outcome>,
}

impl Report {
    pub fn passed(&self) -> usize {
        self.results.iter().filter(|o| o.result.is_ok()).count()
    }

    /// Rule ids of the failing cases, in case order.
    pub fn failing_rules(&self) -> Vec<&'static str> {
        self.results
            .iter()
            .filter(|o| o.result.is_err())
            .map(|o| o.rule)
            .collect()
    }

    pub fn total(&self) -> usize {
        self.results.len()
    }

    pub fn all_passed(&self) -> bool {
        self.passed() == self.total()
    }

    /// `contract Forge: fake 10/10 pass` (or `..., 1 FAIL`).
    pub fn summary(&self) -> String {
        let failed = self.total() - self.passed();
        let mut line = format!(
            "contract {}: {} {}/{} pass",
            self.contract,
            self.implementation,
            self.passed(),
            self.total()
        );
        if failed > 0 {
            line.push_str(&format!(", {failed} FAIL"));
        }
        line
    }

    /// Panics with the full report unless every case passed.
    pub fn assert_passed(&self) {
        assert!(self.all_passed(), "{self}");
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())?;
        for outcome in &self.results {
            if let Err(reason) = &outcome.result {
                write!(
                    f,
                    "\n  FAIL {}.{} ({}): {reason}",
                    self.contract.to_lowercase(),
                    outcome.name,
                    outcome.rule
                )?;
            }
        }
        Ok(())
    }
}

/// Runs every case on a fresh fixture from `make`. A panicking case counts
/// as a failure with the panic message.
pub fn run_suite<F>(
    contract: &'static str,
    implementation: &str,
    cases: &[Case<F>],
    mut make: impl FnMut() -> F,
) -> Report {
    let results = cases
        .iter()
        .map(|case| {
            let fixture = make();
            let result = catch_unwind(AssertUnwindSafe(|| (case.check)(&fixture)))
                .unwrap_or_else(|panic| Err(format!("panicked: {}", panic_message(&*panic))));
            Outcome {
                rule: case.rule,
                name: case.name,
                result,
            }
        })
        .collect();
    Report {
        contract,
        implementation: implementation.to_owned(),
        results,
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic".to_owned()
    }
}

/// `Err` with `what` unless `condition`.
pub(crate) fn ensure(condition: bool, what: impl FnOnce() -> String) -> CaseResult {
    if condition { Ok(()) } else { Err(what()) }
}

/// Turns a trait error into a case failure naming the call.
pub(crate) fn ok<T, E: fmt::Debug>(call: &str, result: Result<T, E>) -> Result<T, String> {
    result.map_err(|e| format!("{call} failed: {e:?}"))
}

/// Polls `probe` until it returns `Some` or `timeout` passes. Real
/// implementations answer asynchronously (a fake agent finishing a turn);
/// fakes answer on the first poll.
pub(crate) fn eventually<T>(
    timeout: Duration,
    mut probe: impl FnMut() -> Result<Option<T>, String>,
) -> Result<Option<T>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = probe()? {
            return Ok(Some(value));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_names_each_failing_case_and_counts_panics() {
        let cases: [Case<u32>; 3] = [
            Case {
                rule: "DEM-1",
                name: "passes",
                check: |_| Ok(()),
            },
            Case {
                rule: "DEM-2",
                name: "fails",
                check: |n| ensure(*n == 0, || format!("expected 0, got {n}")),
            },
            Case {
                rule: "DEM-3",
                name: "panics",
                check: |_| panic!("boom"),
            },
        ];
        let report = run_suite("Demo", "fake", &cases, || 7);
        assert_eq!(report.summary(), "contract Demo: fake 1/3 pass, 2 FAIL");
        let text = report.to_string();
        assert!(
            text.contains("FAIL demo.fails (DEM-2): expected 0, got 7"),
            "{text}"
        );
        assert!(
            text.contains("FAIL demo.panics (DEM-3): panicked: boom"),
            "{text}"
        );
        assert_eq!(report.failing_rules(), ["DEM-2", "DEM-3"]);
    }
}
