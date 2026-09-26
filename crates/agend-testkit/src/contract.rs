//! Contract test suites: one suite per `agend_core::traits` trait, plus the
//! client protocol (`client`, CLP, gate 8: a server, not a trait). A suite is
//! a list of named cases written against a fixture trait (for example
//! [`forge::ForgeFixture`]), so the same cases run against the fake now and
//! against the real implementation at its gate. That is what keeps fakes from
//! drifting away from production (v1 #1483).
//!
//! Each case gets a fresh fixture, by value: a daemon-restart case hands its
//! fixture's persisted state to [`daemon_lifecycle`] and drops the fixture.
//! [`run_all_fakes`] runs every suite against the fakes in `crate::fakes`.
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

#[cfg(unix)]
pub mod client;
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

/// One named check of one numbered contract rule (see CONTRACTS.md). The
/// check owns the fixture, so it can drop it.
pub struct Case<F> {
    pub rule: &'static str,
    pub name: &'static str,
    pub check: fn(F) -> CaseResult,
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
            let result = catch_unwind(AssertUnwindSafe(|| (case.check)(fixture)))
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

/// What one boot of a [`daemon_lifecycle`] does after its boot recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boot {
    /// Checks what earlier boots left, then changes state (writes, starts
    /// holders, delivers).
    Work,
    /// Only the boot recovery (recover holders, backfill, open the store),
    /// then the daemon goes away: state destroyed when read and written
    /// back only on a change is lost here.
    Idle,
    /// Checks everything the earlier boots left; writes only what a check
    /// needs (a CAS to see the next version).
    Check,
}

/// The boots of every daemon lifecycle, in order: a restarted daemon works
/// again, and an idle boot sits between two working ones.
pub const LIFECYCLE: [Boot; 4] = [Boot::Work, Boot::Idle, Boot::Work, Boot::Check];

/// A daemon lifecycle (CONTRACTS.md "daemon 重啟"). The case's fixture is
/// turned into its persisted state (`persist`) and dropped before the first
/// boot, so from then on no instance of the implementation is alive except
/// the booted daemon, and between boots none at all. Boot `n` (1-based, of
/// kind `LIFECYCLE[n - 1]`) gets a new daemon from `boot_daemon` over the
/// persisted state; `boot` recovers the way a real daemon does at every boot
/// and then does what its [`Boot`] says. Then the daemon is dropped, and
/// before the next boot `while_down(n)` lets the backend act and checks it
/// through the persisted state, with no daemon running. Errors name the
/// boot.
pub(crate) fn daemon_lifecycle<F, P>(
    fx: F,
    persist: impl FnOnce(&F) -> P,
    boot_daemon: impl Fn(&P) -> F,
    mut while_down: impl FnMut(&P, usize) -> CaseResult,
    mut boot: impl FnMut(usize, Boot, &F) -> CaseResult,
) -> CaseResult {
    let persisted = persist(&fx);
    drop(fx);
    let boots = LIFECYCLE.len();
    for (index, kind) in LIFECYCLE.into_iter().enumerate() {
        let n = index + 1;
        let daemon = boot_daemon(&persisted);
        let result = boot(n, kind, &daemon);
        drop(daemon);
        result.map_err(|e| format!("boot {n} of {boots} ({kind:?}): {e}"))?;
        if n < boots {
            while_down(&persisted, n)
                .map_err(|e| format!("while down after boot {n} of {boots}: {e}"))?;
        }
    }
    Ok(())
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
                check: |n| ensure(n == 0, || format!("expected 0, got {n}")),
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
