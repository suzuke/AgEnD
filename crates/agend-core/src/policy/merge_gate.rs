//! Merge gate and approval retention (D14). The daemon supplies the observed
//! heads, check result, rebase result and patch IDs; core decides the outcome.
//!
//! Must NOT: compute patch-ids or run git.

use alloc::string::String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeBlocker {
    ChecksNotPassed,
    ApprovalMissing,
    HeadChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeGateResult {
    pub allowed: bool,
    pub blocker: Option<MergeBlocker>,
}

pub fn evaluate(
    checks_passed: bool,
    approved_head: Option<&str>,
    current_head: &str,
    allow_unreviewed: bool,
) -> MergeGateResult {
    let blocker = if !checks_passed {
        Some(MergeBlocker::ChecksNotPassed)
    } else if !allow_unreviewed && approved_head.is_none() {
        Some(MergeBlocker::ApprovalMissing)
    } else if !allow_unreviewed && approved_head != Some(current_head) {
        Some(MergeBlocker::HeadChanged)
    } else {
        None
    };
    MergeGateResult {
        allowed: blocker.is_none(),
        blocker,
    }
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

    #[test]
    fn merge_requires_checks_and_the_exact_approved_head() {
        assert_eq!(
            evaluate(false, Some("H1"), "H1", false).blocker,
            Some(MergeBlocker::ChecksNotPassed)
        );
        assert_eq!(
            evaluate(true, None, "H1", false).blocker,
            Some(MergeBlocker::ApprovalMissing)
        );
        assert_eq!(
            evaluate(true, Some("H1"), "H2", false).blocker,
            Some(MergeBlocker::HeadChanged)
        );
        assert!(evaluate(true, Some("H1"), "H1", false).allowed);
        assert!(evaluate(true, None, "H1", true).allowed);
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
