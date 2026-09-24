//! `Runtime` contract: a started holder is reported for the instance that
//! was launched, recovery lists exactly the running holders, and a stopped
//! holder is gone and can be started again.
//!
//! Not pinned: starting an instance twice; stopping an unknown instance.

use std::fmt::Debug;

use agend_core::traits::{HolderLaunch, Runtime};

use super::{Case, CaseResult, Report, ensure, ok, run_suite};
use crate::block_on;

pub trait RuntimeFixture {
    type Runtime: Runtime<Error = Self::Error>;
    type Error: Send + Debug;

    fn runtime(&self) -> &Self::Runtime;

    /// A launch for `instance_id` that stays running until stopped.
    fn launch(&self, instance_id: &str) -> HolderLaunch;
}

pub fn cases<F: RuntimeFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            name: "start_reports_the_launched_instance",
            check: start_reports_the_launched_instance,
        },
        Case {
            name: "recovery_lists_running_holders",
            check: recovery_lists_running_holders,
        },
        Case {
            name: "stopped_holder_is_not_recovered",
            check: stopped_holder_is_not_recovered,
        },
        Case {
            name: "stopped_instance_can_start_again",
            check: stopped_instance_can_start_again,
        },
    ]
}

pub fn run<F: RuntimeFixture>(implementation: &str, make: impl FnMut() -> F) -> Report {
    run_suite("Runtime", implementation, &cases::<F>(), make)
}

fn start<F: RuntimeFixture>(fx: &F, id: &str) -> CaseResult {
    let handle = ok(
        "start_holder",
        block_on(fx.runtime().start_holder(&fx.launch(id))),
    )?;
    ensure(handle.instance_id == id, || {
        format!(
            "start_holder({id}) returned a handle for {}",
            handle.instance_id
        )
    })?;
    ensure(!handle.socket_path.is_empty(), || {
        "empty holder socket path".into()
    })
}

fn recovered<F: RuntimeFixture>(fx: &F) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = ok("recover_holders", block_on(fx.runtime().recover_holders()))?
        .into_iter()
        .map(|h| h.instance_id)
        .collect();
    ids.sort();
    Ok(ids)
}

fn stop<F: RuntimeFixture>(fx: &F, id: &str) -> CaseResult {
    ok("stop_holder", block_on(fx.runtime().stop_holder(id)))
}

fn start_reports_the_launched_instance<F: RuntimeFixture>(fx: &F) -> CaseResult {
    start(fx, "contract-a")?;
    stop(fx, "contract-a")
}

fn recovery_lists_running_holders<F: RuntimeFixture>(fx: &F) -> CaseResult {
    start(fx, "contract-a")?;
    start(fx, "contract-b")?;
    let ids = recovered(fx)?;
    stop(fx, "contract-a")?;
    stop(fx, "contract-b")?;
    ensure(ids == ["contract-a", "contract-b"], || {
        format!("expected [contract-a, contract-b], recovered {ids:?}")
    })
}

fn stopped_holder_is_not_recovered<F: RuntimeFixture>(fx: &F) -> CaseResult {
    start(fx, "contract-a")?;
    start(fx, "contract-b")?;
    stop(fx, "contract-a")?;
    let ids = recovered(fx)?;
    stop(fx, "contract-b")?;
    ensure(ids == ["contract-b"], || {
        format!("expected [contract-b] after stopping contract-a, recovered {ids:?}")
    })
}

fn stopped_instance_can_start_again<F: RuntimeFixture>(fx: &F) -> CaseResult {
    start(fx, "contract-a")?;
    stop(fx, "contract-a")?;
    start(fx, "contract-a")?;
    stop(fx, "contract-a")
}
