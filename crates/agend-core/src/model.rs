//! Domain types shared across crates: backends, instances, teams, tasks,
//! messages, bindings.
//!
//! Only types whose shape is already decided live here; the rest arrive with
//! the stage that needs them (see `docs/ROADMAP.md`).
//!
//! Must NOT: perform I/O or know about storage layout.

use alloc::format;
use alloc::string::String;

/// Agent backends supported by v2.0 (plan §1; other v1 backends are out of scope).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Claude,
    Codex,
    Opencode,
}

impl Backend {
    pub const ALL: [Backend; 3] = [Backend::Claude, Backend::Codex, Backend::Opencode];

    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Claude => "claude",
            Backend::Codex => "codex",
            Backend::Opencode => "opencode",
        }
    }

    pub fn parse(s: &str) -> Option<Backend> {
        Backend::ALL.into_iter().find(|b| b.as_str() == s)
    }
}

/// Name of the built-in team that cannot be deleted. Instances and tasks
/// created without a team belong to it; there is no "no team" case (D12).
/// It has no repo (D15).
pub const DEFAULT_TEAM: &str = "general";

/// Whether an instance is started with the daemon or cleaned up when its
/// task/team ends (D8, plan §4.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    Persistent,
    Ephemeral,
}

/// Delivery state of one message: `queued -> sent -> confirmed | failed`
/// (plan §4.4). Messages are idempotent by id; this is the only dedup layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    Queued,
    Sent,
    Confirmed,
    Failed,
}

impl DeliveryState {
    pub fn is_terminal(self) -> bool {
        matches!(self, DeliveryState::Confirmed | DeliveryState::Failed)
    }

    /// Allowed forward transitions. A queued message may fail before it is
    /// ever sent; nothing leaves a terminal state.
    pub fn can_transition_to(self, next: DeliveryState) -> bool {
        use DeliveryState::*;
        matches!(
            (self, next),
            (Queued, Sent) | (Queued, Failed) | (Sent, Confirmed) | (Sent, Failed)
        )
    }
}

/// Prefix of every daemon-owned branch. Branches outside this namespace are
/// never touched by the daemon (plan §4.5.1 rule 3).
pub const BRANCH_NAMESPACE: &str = "agend/";

/// Work branch for a task: `agend/<task-id>/<slug>` (plan §4.5.1 rule 3).
pub fn work_branch(task_id: &str, slug: &str) -> String {
    format!("{BRANCH_NAMESPACE}{task_id}/{slug}")
}

/// Worktree directory for a task, relative to the AgEnD home:
/// `worktrees/<task-id>/` (plan §4.5.1 rule 3).
pub fn worktree_dir(task_id: &str) -> String {
    format!("worktrees/{task_id}/")
}

/// Task id encoded in a daemon-namespace branch name, if the name is shaped
/// `agend/<task-id>/<slug>` with non-empty parts. A branch that matches but has
/// no DB record is an orphan; one that does not match is not ours.
pub fn task_id_of_branch(branch: &str) -> Option<&str> {
    let rest = branch.strip_prefix(BRANCH_NAMESPACE)?;
    let (task_id, slug) = rest.split_once('/')?;
    (!task_id.is_empty() && !slug.is_empty()).then_some(task_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_names_round_trip() {
        for b in Backend::ALL {
            assert_eq!(Backend::parse(b.as_str()), Some(b));
        }
        assert_eq!(Backend::parse("grok"), None);
    }

    #[test]
    fn delivery_state_transitions() {
        use DeliveryState::*;
        assert!(Queued.can_transition_to(Sent));
        assert!(Queued.can_transition_to(Failed));
        assert!(Sent.can_transition_to(Confirmed));
        assert!(Sent.can_transition_to(Failed));
        assert!(!Queued.can_transition_to(Confirmed));
        for terminal in [Confirmed, Failed] {
            assert!(terminal.is_terminal());
            for next in [Queued, Sent, Confirmed, Failed] {
                assert!(!terminal.can_transition_to(next));
            }
        }
    }

    #[test]
    fn work_branch_is_recognised_by_its_producer() {
        let branch = work_branch("t-42", "fix-lock-order");
        assert_eq!(branch, "agend/t-42/fix-lock-order");
        assert_eq!(task_id_of_branch(&branch), Some("t-42"));
        assert_eq!(worktree_dir("t-42"), "worktrees/t-42/");
    }

    #[test]
    fn branches_outside_namespace_are_not_ours() {
        assert_eq!(task_id_of_branch("main"), None);
        assert_eq!(task_id_of_branch("feat/x"), None);
        assert_eq!(task_id_of_branch("agend/t-1"), None);
        assert_eq!(task_id_of_branch("agend//slug"), None);
        assert_eq!(task_id_of_branch("agend/t-1/"), None);
    }
}
