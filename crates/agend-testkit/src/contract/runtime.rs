//! `Runtime` contract (rules RTM-1..9 in CONTRACTS.md): a started holder is
//! reported for the instance that was launched and really runs; recovery
//! lists exactly the running holders with the same handles `start_holder`
//! returned (the daemon reconnects through them); a stopped holder really
//! stops (observed outside the trait, through [`RuntimeFixture::is_running`]),
//! is no longer recovered, and can be started again. Across daemon restarts
//! (the old runtime is dropped, [`RuntimeFixture::boot`] builds a new one
//! over the persisted state) holders keep running, also while no daemon
//! runs, and every new runtime recovers them with the same handles and can
//! stop them (D3). The restart cases run a whole daemon lifecycle
//! ([`super::daemon_lifecycle`]: every boot recovers first, one boot does
//! nothing else).
//!
//! Not pinned: starting an instance twice; stopping an unknown instance.

use std::cell::RefCell;
use std::fmt::Debug;

use agend_core::traits::{HolderHandle, HolderLaunch, Runtime};

use super::{Boot, Case, CaseResult, Report, daemon_lifecycle, ensure, ok, run_suite};
use crate::block_on;

pub trait RuntimeFixture: Sized {
    type Runtime: Runtime<Error = Self::Error>;
    type Error: Send + Debug;
    /// What survives a daemon restart: the run directory and the holder
    /// processes (the fake: [`crate::fakes::FakeHolders`]). It is not a
    /// runtime and keeps none alive.
    type Persisted;

    fn runtime(&self) -> &Self::Runtime;

    /// A launch for `instance_id` that stays running until stopped.
    fn launch(&self, instance_id: &str) -> HolderLaunch;

    /// Whether the holder behind `handle` is alive, observed outside the
    /// trait and without any runtime (a real runtime: the process exists and
    /// the socket accepts a connection; the fake: its table of running
    /// holders). Also asked while no daemon runs.
    fn is_running(persisted: &Self::Persisted, handle: &HolderHandle) -> bool;

    /// The persisted state this fixture's runtime works on.
    fn persisted(&self) -> Self::Persisted;

    /// A daemon boot: a new fixture whose runtime is a new instance built
    /// over `persisted`, the way a restarted daemon builds it. Restart cases
    /// drop the case's fixture before the first boot and each booted one
    /// before the next. A real fixture goes through the real persisted state
    /// (the run directory, the holder processes), never a process-global
    /// static (CONTRACTS.md).
    fn boot(persisted: &Self::Persisted) -> Self;
}

pub fn cases<F: RuntimeFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            rule: "RTM-1",
            name: "start_reports_the_launched_instance",
            check: |fx| start_reports_the_launched_instance(&fx),
        },
        Case {
            rule: "RTM-2",
            name: "started_holder_is_running",
            check: |fx| started_holder_is_running(&fx),
        },
        Case {
            rule: "RTM-3",
            name: "recovery_lists_running_holders",
            check: |fx| recovery_lists_running_holders(&fx),
        },
        Case {
            rule: "RTM-4",
            name: "recovered_handles_match_the_started_ones",
            check: |fx| recovered_handles_match_the_started_ones(&fx),
        },
        Case {
            rule: "RTM-5",
            name: "stopped_holder_stops_running",
            check: |fx| stopped_holder_stops_running(&fx),
        },
        Case {
            rule: "RTM-6",
            name: "stopped_holder_is_not_recovered",
            check: |fx| stopped_holder_is_not_recovered(&fx),
        },
        Case {
            rule: "RTM-7",
            name: "stopped_instance_can_start_again",
            check: |fx| stopped_instance_can_start_again(&fx),
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

fn running<F: RuntimeFixture>(fx: &F, handle: &HolderHandle) -> bool {
    F::is_running(&fx.persisted(), handle)
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
    let running = running(fx, &handle);
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
    let (a_running, b_running) = (running(fx, &a), running(fx, &b));
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
    let running = running(fx, &again);
    stop(fx, "contract-a")?;
    ensure(running, || {
        format!("restarted contract-a as {again:?} but it is not running")
    })
}

/// A daemon lifecycle ([`daemon_lifecycle`]) for the runtime: every boot
/// starts with `recover_holders`, the way a restarted daemon reconnects its
/// holders, and `boot` gets what it recovered; it returns the holders that
/// must keep running. While no daemon is up they are checked through the
/// persisted state ([`RuntimeFixture::is_running`]).
fn lifecycle<F: RuntimeFixture>(
    fx: F,
    mut boot: impl FnMut(usize, Boot, &F, Vec<HolderHandle>) -> Result<Vec<HolderHandle>, String>,
) -> CaseResult {
    let up: RefCell<Vec<HolderHandle>> = RefCell::default();
    daemon_lifecycle(
        fx,
        F::persisted,
        F::boot,
        |persisted, _| {
            let dead: Vec<HolderHandle> = up
                .borrow()
                .iter()
                .filter(|h| !F::is_running(persisted, h))
                .cloned()
                .collect();
            ensure(dead.is_empty(), || {
                format!("holders stopped running with no daemon up: {dead:?}")
            })
        },
        |n, kind, daemon| {
            let recovered = recover(daemon)?;
            *up.borrow_mut() = boot(n, kind, daemon, recovered)?;
            Ok(())
        },
    )
}

/// D3: holders outlive every daemon, not only the one that started them. At
/// every boot each holder started so far still runs and is recovered with
/// the handle `start_holder` returned; a working boot then starts one more,
/// and the last boot stops them all.
fn holders_survive_every_daemon_restart<F: RuntimeFixture>(fx: F) -> CaseResult {
    let mut started: Vec<HolderHandle> = Vec::new();
    lifecycle(fx, |n, kind, daemon, recovered| {
        let stopped: Vec<&HolderHandle> = started.iter().filter(|h| !running(daemon, h)).collect();
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
        match kind {
            Boot::Work if verdict.is_ok() => {
                started.push(start(daemon, &format!("contract-boot{n}"))?);
            }
            Boot::Idle if verdict.is_ok() => {}
            // The last boot (or a failed one): clean up whatever runs.
            _ => {
                for handle in recovered.iter().chain(&started) {
                    let _ = stop(daemon, &handle.instance_id);
                }
                started.clear();
            }
        }
        verdict.map(|()| started.clone())
    })
}

/// The restarted daemon manages the recovered holders: the first boot
/// starts them, and every later boot that is not idle stops one that an
/// earlier daemon started; it really stops while the others keep running.
fn restarted_daemon_stops_recovered_holders<F: RuntimeFixture>(fx: F) -> CaseResult {
    let mut started: Vec<HolderHandle> = Vec::new();
    lifecycle(fx, |n, kind, daemon, _| {
        if n == 1 {
            for id in ["contract-a", "contract-b"] {
                started.push(start(daemon, id)?);
            }
            return Ok(started.clone());
        }
        if kind == Boot::Idle {
            return Ok(started.clone());
        }
        if started.is_empty() {
            return Err("no holder left to stop".into());
        }
        let handle = started.remove(0);
        stop(daemon, &handle.instance_id)?;
        ensure(!running(daemon, &handle), || {
            format!(
                "the restarted runtime's stop_holder({}) returned Ok but {handle:?} is still running",
                handle.instance_id
            )
        })?;
        let stopped: Vec<&HolderHandle> = started.iter().filter(|h| !running(daemon, h)).collect();
        ensure(stopped.is_empty(), || {
            format!("stopping {} also stopped {stopped:?}", handle.instance_id)
        })?;
        Ok(started.clone())
    })
}
