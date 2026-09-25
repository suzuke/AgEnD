//! `Runtime` contract (rules RTM-1..9 in CONTRACTS.md): a started holder is
//! reported for the instance that was launched and really runs; recovery
//! lists exactly the running holders with the same handles `start_holder`
//! returned (the daemon reconnects through them); a stopped holder really
//! stops (observed outside the trait, through [`RuntimeFixture::is_running`]),
//! is no longer recovered, and can be started again. Across a daemon restart
//! ([`RuntimeFixture::restart`]: the old runtime is dropped, a new one is
//! built over the same persisted state) holders keep running, and every new
//! runtime recovers them with the same handles and can stop them (D3). The
//! restart cases run a whole daemon lifecycle (three boots, each recovering
//! first, see [`super::daemon_lifecycle`]).
//!
//! Not pinned: starting an instance twice; stopping an unknown instance.

use std::fmt::Debug;

use agend_core::traits::{HolderHandle, HolderLaunch, Runtime};

use super::{BOOTS, Case, CaseResult, Report, daemon_lifecycle, ensure, ok, run_suite};
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

    /// A daemon restart: a new fixture whose runtime is a new instance built
    /// over the same persisted state as `self` (holder processes, run
    /// directory), the way a restarted daemon builds it. Cases drop the old
    /// fixture before creating the next one, so whatever the old runtime
    /// does when it goes away has happened. Cases call it several times on
    /// the fixture they got. A real fixture goes through the real persisted
    /// state (the run directory, the holder processes), never a
    /// process-global static (CONTRACTS.md).
    fn restart(&self) -> Self;
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
        Case {
            rule: "RTM-8",
            name: "holders_survive_every_daemon_restart",
            check: holders_survive_every_daemon_restart,
        },
        Case {
            rule: "RTM-9",
            name: "restarted_daemon_stops_recovered_holders",
            check: restarted_daemon_stops_recovered_holders,
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

/// A daemon lifecycle ([`daemon_lifecycle`]) for the runtime: every boot
/// starts with `recover_holders`, the way a restarted daemon reconnects its
/// holders, and `boot` gets what it recovered. Holders keep running while no
/// daemon is up.
fn lifecycle<F: RuntimeFixture>(
    fx: &F,
    mut boot: impl FnMut(usize, &F, Vec<HolderHandle>) -> CaseResult,
) -> CaseResult {
    daemon_lifecycle(
        || fx.restart(),
        |_| {},
        |n, daemon| boot(n, daemon, recover(daemon)?),
    )
}

/// Instances the lifecycle cases start, one per boot but the last.
const LIFECYCLE_INSTANCES: [&str; BOOTS - 1] = ["contract-a", "contract-b"];

/// D3: holders outlive every daemon, not only the one that started them. At
/// every boot each holder started so far still runs and is recovered with
/// the handle `start_holder` returned; then the boot starts one more.
fn holders_survive_every_daemon_restart<F: RuntimeFixture>(fx: &F) -> CaseResult {
    let mut started: Vec<HolderHandle> = Vec::new();
    lifecycle(fx, |n, daemon, recovered| {
        let stopped: Vec<&HolderHandle> =
            started.iter().filter(|h| !daemon.is_running(h)).collect();
        let verdict = ensure(stopped.is_empty(), || {
            format!("holders died with an earlier daemon: {stopped:?}")
        })
        .and_then(|()| {
            ensure(recovered == started, || {
                format!(
                    "the daemon must recover every holder started so far: started {started:?}, recovered {recovered:?}"
                )
            })
        });
        match LIFECYCLE_INSTANCES.get(n - 1) {
            Some(id) if verdict.is_ok() => started.push(start(daemon, id)?),
            // Last boot (or a failed one): clean up whatever runs.
            _ => {
                for handle in &recovered {
                    let _ = stop(daemon, &handle.instance_id);
                }
            }
        }
        verdict
    })
}

/// The restarted daemon manages the recovered holders: at every boot after
/// the first it stops one that an earlier daemon started, and it really
/// stops while the other keeps running.
fn restarted_daemon_stops_recovered_holders<F: RuntimeFixture>(fx: &F) -> CaseResult {
    let mut started: Vec<HolderHandle> = Vec::new();
    lifecycle(fx, |n, daemon, _| {
        if n == 1 {
            for id in LIFECYCLE_INSTANCES {
                started.push(start(daemon, id)?);
            }
            return Ok(());
        }
        let handle = started.remove(0);
        stop(daemon, &handle.instance_id)?;
        ensure(!daemon.is_running(&handle), || {
            format!(
                "the restarted runtime's stop_holder({}) returned Ok but {handle:?} is still running",
                handle.instance_id
            )
        })?;
        let stopped: Vec<&HolderHandle> =
            started.iter().filter(|h| !daemon.is_running(h)).collect();
        ensure(stopped.is_empty(), || {
            format!("stopping {} also stopped {stopped:?}", handle.instance_id)
        })
    })
}
