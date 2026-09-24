//! Merge gate and approval retention (D14, D19, D20). The daemon supplies the
//! observed heads, check results, rebase result and patch IDs; core decides the
//! outcome. This is the single implementation of the merge rule: the pipeline
//! state machine builds one [`GateFact`] per command and approval stage before
//! the merge stage and asks [`evaluate`].
//!
//! Must NOT: compute patch-ids or run git.

use alloc::string::String;
use alloc::vec::Vec;

/// What one stage before the merge stage contributes to the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateFact<'a> {
    /// A `command` stage and every head it has passed on.
    Check {
        stage_id: &'a str,
        passed_heads: Vec<&'a str>,
    },
    /// An `approval` stage and every recorded approval for it. A recorded
    /// approval carries `Some(head)` when it was given for a head, `None` when
    /// the stage is not head-bound.
    Approval {
        stage_id: &'a str,
        bind_head: bool,
        approvals: Vec<Option<&'a str>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeBlocker {
    /// There is no head to merge.
    NoHead,
    /// The workflow has no command check before merge (D19 save rule).
    NoChecks,
    /// The workflow has no head-bound approval and does not set
    /// `allow_unreviewed` (D19 save rule).
    NoBoundApproval,
    ChecksNotPassed {
        stage_id: String,
    },
    ApprovalMissing {
        stage_id: String,
    },
    /// The stage was approved, but not for the current head.
    HeadChanged {
        stage_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeGateResult {
    pub allowed: bool,
    pub blocker: Option<MergeBlocker>,
}

/// Whether recorded approvals satisfy one approval stage for `current_head`.
/// Head-bound: some approval was given for exactly this head. Not head-bound:
/// an approval exists.
pub fn approval_satisfied(
    bind_head: bool,
    approvals: &[Option<&str>],
    current_head: Option<&str>,
) -> bool {
    if bind_head {
        current_head.is_some() && approvals.contains(&current_head)
    } else {
        !approvals.is_empty()
    }
}

/// The merge gate: every command stage before merge passed for the current
/// head, and every approval stage before merge is satisfied for it.
pub fn evaluate(
    current_head: Option<&str>,
    facts: &[GateFact<'_>],
    allow_unreviewed: bool,
) -> MergeGateResult {
    let blocker = blocker(current_head, facts, allow_unreviewed);
    MergeGateResult {
        allowed: blocker.is_none(),
        blocker,
    }
}

fn blocker(
    current_head: Option<&str>,
    facts: &[GateFact<'_>],
    allow_unreviewed: bool,
) -> Option<MergeBlocker> {
    let Some(head) = current_head else {
        return Some(MergeBlocker::NoHead);
    };
    if !facts
        .iter()
        .any(|fact| matches!(fact, GateFact::Check { .. }))
    {
        return Some(MergeBlocker::NoChecks);
    }
    if !allow_unreviewed
        && !facts.iter().any(|fact| {
            matches!(
                fact,
                GateFact::Approval {
                    bind_head: true,
                    ..
                }
            )
        })
    {
        return Some(MergeBlocker::NoBoundApproval);
    }
    for fact in facts {
        match fact {
            GateFact::Check {
                stage_id,
                passed_heads,
            } => {
                if !passed_heads.contains(&head) {
                    return Some(MergeBlocker::ChecksNotPassed {
                        stage_id: (*stage_id).into(),
                    });
                }
            }
            GateFact::Approval {
                stage_id,
                bind_head,
                approvals,
            } => {
                if approval_satisfied(*bind_head, approvals, Some(head)) {
                    continue;
                }
                let stage_id = (*stage_id).into();
                return Some(if approvals.is_empty() {
                    MergeBlocker::ApprovalMissing { stage_id }
                } else {
                    MergeBlocker::HeadChanged { stage_id }
                });
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseOutcome {
    pub conflict: bool,
    pub rebased_head: Option<String>,
    pub previous_patch_id: String,
    pub rebased_patch_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalAfterRebase {
    Keep { new_approved_head: String },
    ReturnToWork,
}

/// Preserve review only for a clean rebase that leaves the branch diff intact.
pub fn approval_after_rebase(outcome: &RebaseOutcome) -> ApprovalAfterRebase {
    if outcome.conflict
        || outcome.rebased_patch_id.as_deref() != Some(outcome.previous_patch_id.as_str())
    {
        return ApprovalAfterRebase::ReturnToWork;
    }
    outcome
        .rebased_head
        .as_ref()
        .map(|head| ApprovalAfterRebase::Keep {
            new_approved_head: head.clone(),
        })
        .unwrap_or(ApprovalAfterRebase::ReturnToWork)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check<'a>(stage_id: &'a str, passed: &[&'a str]) -> GateFact<'a> {
        GateFact::Check {
            stage_id,
            passed_heads: passed.to_vec(),
        }
    }

    fn approval<'a>(stage_id: &'a str, bind_head: bool, heads: &[Option<&'a str>]) -> GateFact<'a> {
        GateFact::Approval {
            stage_id,
            bind_head,
            approvals: heads.to_vec(),
        }
    }

    #[test]
    fn merge_requires_checks_and_the_exact_approved_head() {
        let blocker = |facts: &[GateFact<'_>]| evaluate(Some("H1"), facts, false).blocker;
        assert_eq!(
            blocker(&[
                check("checks", &["H0"]),
                approval("review", true, &[Some("H1")])
            ]),
            Some(MergeBlocker::ChecksNotPassed {
                stage_id: "checks".into()
            })
        );
        assert_eq!(
            blocker(&[check("checks", &["H1"]), approval("review", true, &[])]),
            Some(MergeBlocker::ApprovalMissing {
                stage_id: "review".into()
            })
        );
        assert_eq!(
            blocker(&[
                check("checks", &["H1"]),
                approval("review", true, &[Some("H0")])
            ]),
            Some(MergeBlocker::HeadChanged {
                stage_id: "review".into()
            })
        );
        assert!(
            evaluate(
                Some("H1"),
                &[
                    check("checks", &["H1"]),
                    approval("review", true, &[Some("H1")])
                ],
                false
            )
            .allowed
        );
        assert_eq!(
            evaluate(None, &[check("checks", &["H1"])], true).blocker,
            Some(MergeBlocker::NoHead)
        );
    }

    #[test]
    fn every_approval_stage_must_cover_the_current_head() {
        let facts = [
            check("checks", &["H1"]),
            approval("a", true, &[Some("H1")]),
            approval("b", true, &[]),
            approval("c", false, &[]),
        ];
        assert_eq!(
            evaluate(Some("H1"), &facts, false).blocker,
            Some(MergeBlocker::ApprovalMissing {
                stage_id: "b".into()
            })
        );
        let facts = [
            check("checks", &["H1"]),
            approval("a", true, &[Some("H1")]),
            approval("b", true, &[Some("H1")]),
            approval("c", false, &[]),
        ];
        assert_eq!(
            evaluate(Some("H1"), &facts, false).blocker,
            Some(MergeBlocker::ApprovalMissing {
                stage_id: "c".into()
            })
        );
        let facts = [
            check("checks", &["H1"]),
            approval("a", true, &[Some("H1")]),
            approval("b", true, &[Some("H1")]),
            approval("c", false, &[None]),
        ];
        assert!(evaluate(Some("H1"), &facts, false).allowed);
    }

    #[test]
    fn every_command_stage_must_pass_on_the_current_head() {
        let facts = [
            check("checks", &["H1"]),
            check("lint", &["H0"]),
            approval("review", true, &[Some("H1")]),
        ];
        assert_eq!(
            evaluate(Some("H1"), &facts, false).blocker,
            Some(MergeBlocker::ChecksNotPassed {
                stage_id: "lint".into()
            })
        );
    }

    #[test]
    fn allow_unreviewed_only_waives_the_bound_approval_requirement() {
        assert_eq!(
            evaluate(Some("H1"), &[check("checks", &["H1"])], false).blocker,
            Some(MergeBlocker::NoBoundApproval)
        );
        assert!(evaluate(Some("H1"), &[check("checks", &["H1"])], true).allowed);
        assert_eq!(
            evaluate(
                Some("H1"),
                &[check("checks", &["H1"]), approval("human", false, &[])],
                true
            )
            .blocker,
            Some(MergeBlocker::ApprovalMissing {
                stage_id: "human".into()
            })
        );
        assert_eq!(
            evaluate(Some("H1"), &[], true).blocker,
            Some(MergeBlocker::NoChecks)
        );
    }

    #[test]
    fn approval_satisfaction_is_per_head_for_bound_stages() {
        assert!(approval_satisfied(true, &[Some("H1")], Some("H1")));
        assert!(!approval_satisfied(true, &[Some("H1")], Some("H2")));
        assert!(!approval_satisfied(true, &[None], None));
        assert!(approval_satisfied(false, &[None], None));
        assert!(!approval_satisfied(false, &[], Some("H1")));
    }

    #[test]
    fn clean_rebase_with_the_same_patch_keeps_approval_at_new_head() {
        assert_eq!(
            approval_after_rebase(&RebaseOutcome {
                conflict: false,
                rebased_head: Some("H2".into()),
                previous_patch_id: "P1".into(),
                rebased_patch_id: Some("P1".into()),
            }),
            ApprovalAfterRebase::Keep {
                new_approved_head: "H2".into(),
            }
        );
    }

    #[test]
    fn conflict_or_changed_patch_returns_to_work() {
        let changed = RebaseOutcome {
            conflict: false,
            rebased_head: Some("H2".into()),
            previous_patch_id: "P1".into(),
            rebased_patch_id: Some("P2".into()),
        };
        assert_eq!(
            approval_after_rebase(&changed),
            ApprovalAfterRebase::ReturnToWork
        );
        assert_eq!(
            approval_after_rebase(&RebaseOutcome {
                conflict: true,
                ..changed
            }),
            ApprovalAfterRebase::ReturnToWork
        );
    }
}
