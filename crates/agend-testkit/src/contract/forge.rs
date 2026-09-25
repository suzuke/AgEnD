//! `Forge` contract (rules FRG-1..9 in CONTRACTS.md): heads are reported as
//! they are, submit reports the head it submitted, and `merge_if_head_is`
//! merges only when the whole expected head equals the head at merge time
//! and otherwise echoes the actual head without changing anything (the
//! event-identity rule for merges). "Changing anything" is observed on the
//! base branch through [`ForgeFixture::base_head`]: a merge moves it to the
//! reported merge commit, a refused merge leaves it where it was.
//!
//! Not pinned (differs between a local forge and GitHub): what submitting the
//! same branch twice returns; whether a merged branch still exists; what
//! merging an already merged head again does.

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

    /// Current head of the base branch that merges land on (read outside
    /// the trait, e.g. `git rev-parse` or the GitHub API).
    fn base_head(&self) -> String;
}

pub fn cases<F: ForgeFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            rule: "FRG-1",
            name: "head_reports_the_latest_commit",
            check: head_reports_the_latest_commit,
        },
        Case {
            rule: "FRG-2",
            name: "head_of_unknown_branch_is_an_error",
            check: head_of_unknown_branch_is_an_error,
        },
        Case {
            rule: "FRG-3",
            name: "submit_reports_the_submitted_head",
            check: submit_reports_the_submitted_head,
        },
        Case {
            rule: "FRG-4",
            name: "submit_of_unknown_branch_is_an_error",
            check: submit_of_unknown_branch_is_an_error,
        },
        Case {
            rule: "FRG-5",
            name: "merge_with_expected_head_merges",
            check: merge_with_expected_head_merges,
        },
        Case {
            rule: "FRG-5",
            name: "merge_compares_the_head_at_merge_time",
            check: merge_compares_the_head_at_merge_time,
        },
        Case {
            rule: "FRG-6",
            name: "merge_with_stale_head_echoes_actual_head",
            check: merge_with_stale_head_echoes_actual_head,
        },
        Case {
            rule: "FRG-7",
            name: "stale_merge_changes_nothing",
            check: stale_merge_changes_nothing,
        },
        Case {
            rule: "FRG-8",
            name: "merge_needs_the_whole_head",
            check: merge_needs_the_whole_head,
        },
        Case {
            rule: "FRG-9",
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
    let base_before = fx.base_head();
    let request = MergeRequest {
        branch: b,
        expected_head: head,
    };
    let result = ok(
        "merge_if_head_is",
        block_on(fx.forge().merge_if_head_is(&request)),
    )?;
    base_moved_to_merge_commit(fx, &base_before, &result)
}

/// The head compared is the one at merge time, not the one recorded when
/// the change was submitted (commits after submit are normal: rework).
fn merge_compares_the_head_at_merge_time<F: ForgeFixture>(fx: &F) -> CaseResult {
    let b = branch("merge-after-rework");
    fx.commit_to(&b);
    ok("submit", block_on(fx.forge().submit(&submission(&b))))?;
    let head = fx.commit_to(&b);
    let base_before = fx.base_head();
    let request = MergeRequest {
        branch: b,
        expected_head: head,
    };
    let result = ok(
        "merge_if_head_is",
        block_on(fx.forge().merge_if_head_is(&request)),
    )?;
    base_moved_to_merge_commit(fx, &base_before, &result)
        .map_err(|e| format!("merge with the current head after a commit following submit: {e}"))
}

/// A merge reported as done moved the base from `before` to its merge commit.
fn base_moved_to_merge_commit<F: ForgeFixture>(
    fx: &F,
    before: &str,
    result: &MergeResult,
) -> CaseResult {
    let MergeResult::Merged { merge_commit } = result else {
        return Err(format!(
            "expected Merged with a merge commit, got {result:?}"
        ));
    };
    ensure(!merge_commit.is_empty(), || {
        "Merged has an empty merge commit".into()
    })?;
    let after = fx.base_head();
    ensure(after != before && after == *merge_commit, || {
        format!(
            "after Merged {{ merge_commit: {merge_commit} }} the base head must be that commit (was {before}), got {after}"
        )
    })
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
    let base_before = fx.base_head();
    let (actual, _) = stale_merge(fx, &b)?;
    let base_after = fx.base_head();
    ensure(base_after == base_before, || {
        format!(
            "a merge with a stale head moved the base: expected {base_before}, got {base_after}"
        )
    })?;
    let head = ok("head", block_on(fx.forge().head(&b)))?;
    ensure(head == actual, || {
        format!("a refused merge moved the head: expected {actual}, got {head}")
    })?;
    let request = MergeRequest {
        branch: b,
        expected_head: actual,
    };
    let result = ok(
        "merge_if_head_is",
        block_on(fx.forge().merge_if_head_is(&request)),
    )?;
    base_moved_to_merge_commit(fx, &base_before, &result)
        .map_err(|e| format!("merge with the current head after a refusal: {e}"))
}

/// An empty or abbreviated `expected_head` is not the head: only the whole
/// SHA approves a merge.
fn merge_needs_the_whole_head<F: ForgeFixture>(fx: &F) -> CaseResult {
    let b = branch("partial-head");
    let head = fx.commit_to(&b);
    ok("submit", block_on(fx.forge().submit(&submission(&b))))?;
    let base_before = fx.base_head();
    let short = head.chars().take(7).collect::<String>();
    let all_but_last = head[..head.len().saturating_sub(1)].to_owned();
    for partial in [String::new(), short, all_but_last] {
        let request = MergeRequest {
            branch: b.clone(),
            expected_head: partial.clone(),
        };
        let result = ok(
            "merge_if_head_is",
            block_on(fx.forge().merge_if_head_is(&request)),
        )?;
        let expected = MergeResult::HeadChanged {
            actual_head: head.clone(),
        };
        ensure(result == expected, || {
            format!(
                "expected_head {partial:?} (head {head}): expected {expected:?}, got {result:?}"
            )
        })?;
        let base_after = fx.base_head();
        ensure(base_after == base_before, || {
            format!("expected_head {partial:?} moved the base from {base_before} to {base_after}")
        })?;
    }
    Ok(())
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
