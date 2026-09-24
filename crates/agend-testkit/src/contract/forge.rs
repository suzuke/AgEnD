//! `Forge` contract: heads are reported as they are, submit reports the head
//! it submitted, and `merge_if_head_is` merges only the expected head and
//! otherwise echoes the actual head without changing anything (the
//! event-identity rule for merges).
//!
//! Not pinned (differs between a local forge and GitHub): what submitting the
//! same branch twice returns; whether a merged branch still exists.

use std::fmt::Debug;

use agend_core::model::work_branch;
use agend_core::traits::{Forge, MergeRequest, MergeResult, Submission};

use super::{Case, CaseResult, Report, ensure, ok, run_suite};
use crate::block_on;

/// What a forge contract needs besides the trait.
pub trait ForgeFixture {
    type Forge: Forge<Error = Self::Error>;
    type Error: Send + Debug;

    fn forge(&self) -> &Self::Forge;

    /// Adds a new commit on `branch` (creating the branch from the base if
    /// needed), makes it visible to the forge and returns the new head.
    fn commit_to(&self, branch: &str) -> String;
}

pub fn cases<F: ForgeFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            name: "head_reports_the_latest_commit",
            check: head_reports_the_latest_commit,
        },
        Case {
            name: "head_of_unknown_branch_is_an_error",
            check: head_of_unknown_branch_is_an_error,
        },
        Case {
            name: "submit_reports_the_submitted_head",
            check: submit_reports_the_submitted_head,
        },
        Case {
            name: "submit_of_unknown_branch_is_an_error",
            check: submit_of_unknown_branch_is_an_error,
        },
        Case {
            name: "merge_with_expected_head_merges",
            check: merge_with_expected_head_merges,
        },
        Case {
            name: "merge_with_stale_head_echoes_actual_head",
            check: merge_with_stale_head_echoes_actual_head,
        },
        Case {
            name: "stale_merge_changes_nothing",
            check: stale_merge_changes_nothing,
        },
        Case {
            name: "merge_of_unknown_branch_is_an_error",
            check: merge_of_unknown_branch_is_an_error,
        },
    ]
}

pub fn run<F: ForgeFixture>(implementation: &str, make: impl FnMut() -> F) -> Report {
    run_suite("Forge", implementation, &cases::<F>(), make)
}

fn branch(slug: &str) -> String {
    work_branch("T-contract", slug)
}

fn submission(branch: &str) -> Submission {
    Submission {
        task_id: "T-contract".into(),
        branch: branch.into(),
        title: "contract change".into(),
        body: "opened by the Forge contract suite".into(),
    }
}

fn head_reports_the_latest_commit<F: ForgeFixture>(fx: &F) -> CaseResult {
    let b = branch("head");
    let first = fx.commit_to(&b);
    let reported = ok("head", block_on(fx.forge().head(&b)))?;
    ensure(reported == first, || {
        format!("head after first commit: expected {first}, got {reported}")
    })?;
    let second = fx.commit_to(&b);
    ensure(second != first, || "two commits got the same id".into())?;
    let reported = ok("head", block_on(fx.forge().head(&b)))?;
    ensure(reported == second, || {
        format!("head after second commit: expected {second}, got {reported}")
    })
}

fn head_of_unknown_branch_is_an_error<F: ForgeFixture>(fx: &F) -> CaseResult {
    let result = block_on(fx.forge().head(&branch("missing")));
    ensure(result.is_err(), || {
        format!("expected an error for an unknown branch, got {result:?}")
    })
}

fn submit_reports_the_submitted_head<F: ForgeFixture>(fx: &F) -> CaseResult {
    let b = branch("submit");
    let head = fx.commit_to(&b);
    let change = ok("submit", block_on(fx.forge().submit(&submission(&b))))?;
    ensure(!change.id.is_empty(), || {
        "submitted change has an empty id".into()
    })?;
    ensure(change.head == head, || {
        format!(
            "submit must echo the branch head {head}, got {}",
            change.head
        )
    })
}

fn submit_of_unknown_branch_is_an_error<F: ForgeFixture>(fx: &F) -> CaseResult {
    let result = block_on(fx.forge().submit(&submission(&branch("missing"))));
    ensure(result.is_err(), || {
        format!("expected an error for an unknown branch, got {result:?}")
    })
}

fn merge_with_expected_head_merges<F: ForgeFixture>(fx: &F) -> CaseResult {
    let b = branch("merge");
    let head = fx.commit_to(&b);
    ok("submit", block_on(fx.forge().submit(&submission(&b))))?;
    let request = MergeRequest {
        branch: b,
        expected_head: head,
    };
    match ok(
        "merge_if_head_is",
        block_on(fx.forge().merge_if_head_is(&request)),
    )? {
        MergeResult::Merged { merge_commit } if !merge_commit.is_empty() => Ok(()),
        other => Err(format!(
            "expected Merged with a merge commit, got {other:?}"
        )),
    }
}

/// Commits twice, then asks to merge the first head.
fn stale_merge<F: ForgeFixture>(fx: &F, b: &str) -> Result<(String, MergeResult), String> {
    let stale = fx.commit_to(b);
    ok("submit", block_on(fx.forge().submit(&submission(b))))?;
    let actual = fx.commit_to(b);
    let request = MergeRequest {
        branch: b.into(),
        expected_head: stale,
    };
    let result = ok(
        "merge_if_head_is",
        block_on(fx.forge().merge_if_head_is(&request)),
    )?;
    Ok((actual, result))
}

fn merge_with_stale_head_echoes_actual_head<F: ForgeFixture>(fx: &F) -> CaseResult {
    let (actual, result) = stale_merge(fx, &branch("stale"))?;
    let expected = MergeResult::HeadChanged {
        actual_head: actual,
    };
    ensure(result == expected, || {
        format!("expected {expected:?}, got {result:?}")
    })
}

fn stale_merge_changes_nothing<F: ForgeFixture>(fx: &F) -> CaseResult {
    let b = branch("stale-then-merge");
    let (actual, _) = stale_merge(fx, &b)?;
    let head = ok("head", block_on(fx.forge().head(&b)))?;
    ensure(head == actual, || {
        format!("a refused merge moved the head: expected {actual}, got {head}")
    })?;
    let request = MergeRequest {
        branch: b,
        expected_head: actual,
    };
    match ok(
        "merge_if_head_is",
        block_on(fx.forge().merge_if_head_is(&request)),
    )? {
        MergeResult::Merged { .. } => Ok(()),
        other => Err(format!(
            "merge with the current head after a refusal: expected Merged, got {other:?}"
        )),
    }
}

fn merge_of_unknown_branch_is_an_error<F: ForgeFixture>(fx: &F) -> CaseResult {
    let request = MergeRequest {
        branch: branch("missing"),
        expected_head: "0".repeat(40),
    };
    let result = block_on(fx.forge().merge_if_head_is(&request));
    ensure(result.is_err(), || {
        format!("expected an error for an unknown branch, got {result:?}")
    })
}
