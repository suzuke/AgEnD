//! `Runtime` contract (rules RTM-1..7 in CONTRACTS.md): a started holder is
//! reported for the instance that was launched and really runs; recovery
//! lists exactly the running holders with the same handles `start_holder`
//! returned (the daemon reconnects through them); a stopped holder really
//! stops (observed outside the trait, through [`RuntimeFixture::is_running`]),
//! is no longer recovered, and can be started again.
//!
//! Not pinned: starting an instance twice; stopping an unknown instance.

use std::fmt::Debug;

use agend_core::traits::{HolderHandle, HolderLaunch, Runtime};

use super::{Case, CaseResult, Report, ensure, ok, run_suite};
use crate::block_on;

pub trait RuntimeFixture {
    type Runtime: Runtime<Error = Self::Error>;
    type Error: Send + Debug;

    fn runtime(&self) -> &Self::Runtime;

    /// A launch for `instance_id` that stays running until stopped.
    fn launch(&self, instance_id: &str) -> HolderLaunch;

    /// Whether the holder behind `handle` is alive, observed outside the
    /// trait (a real runtime: the process exists and the socket accepts a
    /// connection; the fake: its table of running holders).
    fn is_running(&self, handle: &HolderHandle) -> bool;
}

pub fn cases<F: RuntimeFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            rule: "RTM-1",
            name: "start_reports_the_launched_instance",
            check: start_reports_the_launched_instance,
        },
        Case {
            rule: "RTM-2",
            name: "started_holder_is_running",
            check: started_holder_is_running,
        },
        Case {
            rule: "RTM-3",
            name: "recovery_lists_running_holders",
            check: recovery_lists_running_holders,
        },
        Case {
            rule: "RTM-4",
            name: "recovered_handles_match_the_started_ones",
            check: recovered_handles_match_the_started_ones,
        },
        Case {
            rule: "RTM-5",
            name: "stopped_holder_stops_running",
            check: stopped_holder_stops_running,
        },
        Case {
            rule: "RTM-6",
            name: "stopped_holder_is_not_recovered",
            check: stopped_holder_is_not_recovered,
        },
        Case {
            rule: "RTM-7",
            name: "stopped_instance_can_start_again",
            check: stopped_instance_can_start_again,
        },
    ]
}

pub fn run<F: RuntimeFixture>(implementation: &str, make: impl FnMut() -> F) -> Report {
    run_suite("Runtime", implementation, &cases::<F>(), make)
}

fn start<F: RuntimeFixture>(fx: &F, id: &str) -> Result<HolderHandle, String> {
    ok(
        "start_holder",
        block_on(fx.runtime().start_holder(&fx.launch(id))),
    )
}

fn recover<F: RuntimeFixture>(fx: &F) -> Result<Vec<HolderHandle>, String> {
    let mut handles = ok("recover_holders", block_on(fx.runtime().recover_holders()))?;
    handles.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
    Ok(handles)
}

fn recovered_ids<F: RuntimeFixture>(fx: &F) -> Result<Vec<String>, String> {
    Ok(recover(fx)?.into_iter().map(|h| h.instance_id).collect())
}

fn stop<F: RuntimeFixture>(fx: &F, id: &str) -> CaseResult {
    ok("stop_holder", block_on(fx.runtime().stop_holder(id)))
}

fn start_reports_the_launched_instance<F: RuntimeFixture>(fx: &F) -> CaseResult {
    let handle = start(fx, "contract-a")?;
    stop(fx, "contract-a")?;
    ensure(handle.instance_id == "contract-a", || {
        format!(
            "start_holder(contract-a) returned a handle for {}",
            handle.instance_id
        )
    })?;
    ensure(!handle.socket_path.is_empty(), || {
        "empty holder socket path".into()
    })
}

fn started_holder_is_running<F: RuntimeFixture>(fx: &F) -> CaseResult {
    let handle = start(fx, "contract-a")?;
    let running = fx.is_running(&handle);
    stop(fx, "contract-a")?;
    ensure(running, || {
        format!("start_holder returned {handle:?} but no such holder is running")
    })
}

fn recovery_lists_running_holders<F: RuntimeFixture>(fx: &F) -> CaseResult {
    start(fx, "contract-a")?;
    start(fx, "contract-b")?;
    let ids = recovered_ids(fx)?;
    stop(fx, "contract-a")?;
    stop(fx, "contract-b")?;
    ensure(ids == ["contract-a", "contract-b"], || {
        format!("expected [contract-a, contract-b], recovered {ids:?}")
    })
}

fn recovered_handles_match_the_started_ones<F: RuntimeFixture>(fx: &F) -> CaseResult {
    let started = vec![start(fx, "contract-a")?, start(fx, "contract-b")?];
    let recovered = recover(fx)?;
    stop(fx, "contract-a")?;
    stop(fx, "contract-b")?;
    ensure(recovered == started, || {
        format!(
            "recovery must return the handles start_holder returned: started {started:?}, recovered {recovered:?}"
        )
    })
}

fn stopped_holder_stops_running<F: RuntimeFixture>(fx: &F) -> CaseResult {
    let a = start(fx, "contract-a")?;
    let b = start(fx, "contract-b")?;
    stop(fx, "contract-a")?;
    let (a_running, b_running) = (fx.is_running(&a), fx.is_running(&b));
    stop(fx, "contract-b")?;
    ensure(!a_running, || {
        format!("stop_holder(contract-a) returned Ok but {a:?} is still running")
    })?;
    ensure(b_running, || {
        format!("stopping contract-a also stopped {b:?}")
    })
}

fn stopped_holder_is_not_recovered<F: RuntimeFixture>(fx: &F) -> CaseResult {
    start(fx, "contract-a")?;
    start(fx, "contract-b")?;
    stop(fx, "contract-a")?;
    let ids = recovered_ids(fx)?;
    stop(fx, "contract-b")?;
    ensure(ids == ["contract-b"], || {
        format!("expected [contract-b] after stopping contract-a, recovered {ids:?}")
    })
}

fn stopped_instance_can_start_again<F: RuntimeFixture>(fx: &F) -> CaseResult {
    start(fx, "contract-a")?;
    stop(fx, "contract-a")?;
    let again = start(fx, "contract-a")?;
    let running = fx.is_running(&again);
    stop(fx, "contract-a")?;
    ensure(running, || {
        format!("restarted contract-a as {again:?} but it is not running")
    })
}
