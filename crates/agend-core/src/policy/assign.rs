//! Assignment rules for role templates (D18, D25, D33). The daemon supplies
//! team candidates, what each holds and allowed backends; core makes the
//! deterministic choice.
//!
//! D33: an agent holds at most one task, from assignment until the task is
//! done or cancelled, including while it waits for submit, checks or review;
//! a review assignment is the reviewer's one task. Rework therefore always
//! returns to the task holder, who cannot be busy with something else. The
//! task holder is replaced only when it hits a usage limit or is removed.
//! ("Holder" alone means the per-instance holder process, not this.)
//!
//! Must NOT: spawn instances or talk to drivers.

use alloc::string::String;
use alloc::vec::Vec;

use crate::model::Backend;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub instance_id: String,
    pub team_id: String,
    pub role: String,
    pub backend: Backend,
    /// The one task this instance holds (D33); `None` when it is free.
    pub held_task: Option<String>,
    /// Spawned on demand within the role's maximum headcount.
    pub ephemeral: bool,
    pub usage_available: bool,
}

impl Candidate {
    pub fn is_free(&self) -> bool {
        self.held_task.is_none()
    }
}

/// An ephemeral instance may be reclaimed only once the task it holds has
/// ended, so a task holder doing rework is never reclaimed mid-task (D33).
pub fn may_reclaim(candidate: &Candidate) -> bool {
    candidate.ephemeral && candidate.is_free()
}

/// Role template headcount. `current_instances` counts persistent and live
/// ephemeral instances for this team and role; the daemon maintains the
/// configured minimum, while assignment uses the maximum before spawning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleCapacity {
    pub minimum_instances: usize,
    pub maximum_instances: usize,
    pub current_instances: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    NewTask,
    Review {
        task_holder: String,
        task_holder_backend: Backend,
    },
    /// The task's work continues — rework after requested changes, or its
    /// task holder has to be replaced. `branch` and `review_comments` travel
    /// with the task if another instance takes it over.
    Rework {
        task_id: String,
        task_holder: String,
        branch: Option<String>,
        review_comments: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignmentRequest {
    pub team_id: String,
    pub role: String,
    pub allowed_backends: Vec<Backend>,
    /// Backends whose usage quota can currently support a new instance.
    pub available_backends: Vec<Backend>,
    /// `None` means the role template does not exist in this team.
    pub role_capacity: Option<RoleCapacity>,
    pub purpose: Purpose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueReason {
    AtCapacity,
    UsageLimit,
    InvalidRoleCapacity,
    NoAllowedBackend,
    NoEligibleReviewer,
}

/// What a new task holder receives when a task changes hands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub from_instance: String,
    pub branch: Option<String>,
    pub review_comments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignmentDecision {
    Assigned {
        instance_id: String,
    },
    /// The task holder cannot continue; another instance takes the task over.
    Reassigned {
        instance_id: String,
        handoff: Handoff,
    },
    SpawnEphemeral {
        backend: Backend,
        handoff: Option<Handoff>,
    },
    Queue {
        reason: QueueReason,
    },
    AskForRole {
        role: String,
    },
}

pub fn choose(request: &AssignmentRequest, candidates: &[Candidate]) -> AssignmentDecision {
    if let Purpose::Rework {
        task_id,
        task_holder,
        branch,
        review_comments,
    } = &request.purpose
    {
        // Instance ids are team-scoped. The task holder still counts only if
        // it holds this task, its backend is still allowed and the role still
        // exists; otherwise it is treated as removed.
        let current = candidates.iter().find(|candidate| {
            candidate.team_id == request.team_id
                && candidate.instance_id == *task_holder
                && candidate.held_task.as_deref() == Some(task_id.as_str())
                && is_allowed(candidate, &request.allowed_backends)
                && request.role_capacity.is_some()
        });
        // The task holder keeps the task: it cannot be busy with another one.
        if let Some(current) = current
            && current.usage_available
        {
            return AssignmentDecision::Assigned {
                instance_id: task_holder.clone(),
            };
        }
        let handoff = Handoff {
            from_instance: task_holder.clone(),
            branch: branch.clone(),
            review_comments: review_comments.clone(),
        };
        return take_over(request, candidates, current, handoff);
    }

    let Some(capacity) = request.role_capacity else {
        return AssignmentDecision::AskForRole {
            role: request.role.clone(),
        };
    };
    if !valid_role_capacity(capacity) {
        return AssignmentDecision::Queue {
            reason: QueueReason::InvalidRoleCapacity,
        };
    }

    let role_candidates = role_candidates(request, candidates);
    let mut eligible = free_candidates(request, candidates, None);

    if let Purpose::Review {
        task_holder,
        task_holder_backend,
    } = &request.purpose
    {
        eligible.retain(|candidate| candidate.instance_id != *task_holder);
        eligible.sort_by_key(|candidate| {
            (
                candidate.backend == *task_holder_backend,
                candidate.instance_id.as_str(),
            )
        });
    }

    if let Some(candidate) = eligible.first() {
        return AssignmentDecision::Assigned {
            instance_id: candidate.instance_id.clone(),
        };
    }

    if role_candidates.is_empty() && request.allowed_backends.is_empty() {
        return AssignmentDecision::Queue {
            reason: QueueReason::NoAllowedBackend,
        };
    }
    // Review prefers a backend other than the task holder's, but falls back to
    // the same backend rather than queueing when it is the only one allowed.
    let preferred_spawn = match &request.purpose {
        Purpose::Review {
            task_holder_backend,
            ..
        } => spawn_backend(request, Some(*task_holder_backend)),
        _ => None,
    };
    if let Some(backend) = preferred_spawn.or_else(|| spawn_backend(request, None)) {
        return AssignmentDecision::SpawnEphemeral {
            backend,
            handoff: None,
        };
    }

    if let Purpose::Review { task_holder, .. } = &request.purpose
        && !role_candidates.is_empty()
        && role_candidates
            .iter()
            .all(|candidate| candidate.instance_id == *task_holder)
    {
        return AssignmentDecision::Queue {
            reason: QueueReason::NoEligibleReviewer,
        };
    }
    AssignmentDecision::Queue {
        reason: queue_reason(capacity),
    }
}

/// The task holder cannot continue. Usage limit (`current` still present): a
/// free same-role member on another allowed backend, else an ephemeral
/// instance on another backend, else queue for the usage limit. Removed task
/// holder (`current` is `None`): any free same-role member, else an ephemeral
/// instance within headcount, else queue.
fn take_over(
    request: &AssignmentRequest,
    candidates: &[Candidate],
    current: Option<&Candidate>,
    handoff: Handoff,
) -> AssignmentDecision {
    let Some(capacity) = request.role_capacity else {
        return AssignmentDecision::AskForRole {
            role: request.role.clone(),
        };
    };
    if !valid_role_capacity(capacity) {
        return AssignmentDecision::Queue {
            reason: QueueReason::InvalidRoleCapacity,
        };
    }
    let excluded_backend = current.map(|task_holder| task_holder.backend);
    if let Some(candidate) = free_candidates(request, candidates, Some(&handoff.from_instance))
        .into_iter()
        .find(|candidate| Some(candidate.backend) != excluded_backend)
    {
        return AssignmentDecision::Reassigned {
            instance_id: candidate.instance_id.clone(),
            handoff,
        };
    }
    if let Some(backend) = spawn_backend(request, excluded_backend) {
        return AssignmentDecision::SpawnEphemeral {
            backend,
            handoff: Some(handoff),
        };
    }
    AssignmentDecision::Queue {
        reason: if current.is_some() {
            QueueReason::UsageLimit
        } else {
            queue_reason(capacity)
        },
    }
}

fn queue_reason(capacity: RoleCapacity) -> QueueReason {
    if capacity.current_instances >= capacity.maximum_instances {
        QueueReason::AtCapacity
    } else {
        QueueReason::UsageLimit
    }
}

fn is_allowed(candidate: &Candidate, allowed_backends: &[Backend]) -> bool {
    allowed_backends.contains(&candidate.backend)
}

fn role_candidates<'a>(
    request: &AssignmentRequest,
    candidates: &'a [Candidate],
) -> Vec<&'a Candidate> {
    candidates
        .iter()
        .filter(|candidate| candidate.team_id == request.team_id && candidate.role == request.role)
        .collect()
}

/// Same-team, same-role instances on an allowed backend with usage left that
/// hold no task, ordered by instance id.
fn free_candidates<'a>(
    request: &AssignmentRequest,
    candidates: &'a [Candidate],
    exclude_instance: Option<&str>,
) -> Vec<&'a Candidate> {
    let mut free: Vec<&Candidate> = role_candidates(request, candidates)
        .into_iter()
        .filter(|candidate| {
            is_allowed(candidate, &request.allowed_backends)
                && candidate.usage_available
                && candidate.is_free()
                && exclude_instance != Some(candidate.instance_id.as_str())
        })
        .collect();
    free.sort_by_key(|candidate| candidate.instance_id.as_str());
    free
}

fn valid_role_capacity(capacity: RoleCapacity) -> bool {
    capacity.minimum_instances <= capacity.maximum_instances
        && capacity.current_instances <= capacity.maximum_instances
}

fn spawn_backend(
    request: &AssignmentRequest,
    excluded_backend: Option<Backend>,
) -> Option<Backend> {
    let capacity = request.role_capacity?;
    if !valid_role_capacity(capacity) || capacity.current_instances >= capacity.maximum_instances {
        return None;
    }
    request.allowed_backends.iter().copied().find(|backend| {
        Some(*backend) != excluded_backend && request.available_backends.contains(backend)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitEdge {
    pub team_id: String,
    pub waiter: String,
    pub waits_for: String,
}

/// Detect whether adding an edge would close a wait cycle in the same team.
pub fn creates_wait_cycle(existing: &[WaitEdge], new_edge: &WaitEdge) -> bool {
    if new_edge.waiter == new_edge.waits_for {
        return true;
    }
    let mut pending = alloc::vec![new_edge.waits_for.as_str()];
    let mut visited: Vec<&str> = Vec::new();
    while let Some(current) = pending.pop() {
        if current == new_edge.waiter {
            return true;
        }
        if visited.contains(&current) {
            continue;
        }
        visited.push(current);
        pending.extend(
            existing
                .iter()
                .filter(|edge| edge.team_id == new_edge.team_id && edge.waiter == current)
                .map(|edge| edge.waits_for.as_str()),
        );
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn candidate(id: &str, backend: Backend, held_task: Option<&str>, usage: bool) -> Candidate {
        Candidate {
            instance_id: id.into(),
            team_id: "team-a".into(),
            role: "dev".into(),
            backend,
            held_task: held_task.map(Into::into),
            ephemeral: false,
            usage_available: usage,
        }
    }

    fn request(purpose: Purpose) -> AssignmentRequest {
        AssignmentRequest {
            team_id: "team-a".into(),
            role: "dev".into(),
            allowed_backends: Backend::ALL.to_vec(),
            available_backends: Backend::ALL.to_vec(),
            role_capacity: Some(RoleCapacity {
                minimum_instances: 1,
                maximum_instances: 3,
                current_instances: 1,
            }),
            purpose,
        }
    }

    fn rework(task_holder: &str) -> Purpose {
        Purpose::Rework {
            task_id: "T-1".into(),
            task_holder: task_holder.into(),
            branch: Some("agend/T-1/demo".into()),
            review_comments: vec!["rename the flag".into()],
        }
    }

    fn handoff(from: &str) -> Handoff {
        Handoff {
            from_instance: from.into(),
            branch: Some("agend/T-1/demo".into()),
            review_comments: vec!["rename the flag".into()],
        }
    }

    fn capacity(current: usize, maximum: usize) -> Option<RoleCapacity> {
        Some(RoleCapacity {
            minimum_instances: 0,
            maximum_instances: maximum,
            current_instances: current,
        })
    }

    #[test]
    fn review_excludes_the_task_holder_and_prefers_another_backend() {
        let candidates = [
            candidate("task-holder", Backend::Codex, Some("T-1"), true),
            candidate("same", Backend::Codex, None, true),
            candidate("other", Backend::Claude, None, true),
        ];
        assert_eq!(
            choose(
                &request(Purpose::Review {
                    task_holder: "task-holder".into(),
                    task_holder_backend: Backend::Codex,
                }),
                &candidates,
            ),
            AssignmentDecision::Assigned {
                instance_id: "other".into()
            }
        );
    }

    /// D33 rule 1: one task per agent; a held task (including a review)
    /// makes the agent unavailable for anything else.
    #[test]
    fn an_agent_holding_a_task_gets_no_other_task_or_review() {
        let candidates = [
            candidate("busy", Backend::Codex, Some("T-1"), true),
            candidate("reviewing", Backend::Claude, Some("review:T-2"), true),
        ];
        let full = AssignmentRequest {
            role_capacity: capacity(2, 2),
            ..request(Purpose::NewTask)
        };
        assert_eq!(
            choose(&full, &candidates),
            AssignmentDecision::Queue {
                reason: QueueReason::AtCapacity
            }
        );
        let review = AssignmentRequest {
            role_capacity: capacity(2, 2),
            ..request(Purpose::Review {
                task_holder: "task-holder".into(),
                task_holder_backend: Backend::Opencode,
            })
        };
        assert_eq!(
            choose(&review, &candidates),
            AssignmentDecision::Queue {
                reason: QueueReason::AtCapacity
            }
        );
    }

    /// D33: a new task when every member holds one spawns within the role's
    /// maximum headcount, otherwise queues.
    #[test]
    fn new_task_when_all_members_hold_tasks_spawns_within_headcount_else_queues() {
        let members = [
            candidate("dev-1", Backend::Codex, Some("T-1"), true),
            candidate("dev-2", Backend::Claude, Some("T-2"), true),
        ];
        let room = AssignmentRequest {
            role_capacity: capacity(2, 3),
            ..request(Purpose::NewTask)
        };
        assert_eq!(
            choose(&room, &members),
            AssignmentDecision::SpawnEphemeral {
                backend: Backend::Claude,
                handoff: None,
            }
        );
        let full = AssignmentRequest {
            role_capacity: capacity(3, 3),
            ..room
        };
        assert_eq!(
            choose(&full, &members),
            AssignmentDecision::Queue {
                reason: QueueReason::AtCapacity
            }
        );
    }

    /// D33 rule 2: rework returns to the task holder, who still holds it.
    #[test]
    fn rework_returns_to_the_task_holder_that_still_holds_the_task() {
        let task_holder = candidate("task-holder", Backend::Codex, Some("T-1"), true);
        let other = candidate("other", Backend::Claude, None, true);
        let full = AssignmentRequest {
            role_capacity: capacity(2, 2),
            ..request(rework("task-holder"))
        };
        assert_eq!(
            choose(&full, &[task_holder, other]),
            AssignmentDecision::Assigned {
                instance_id: "task-holder".into()
            }
        );
    }

    #[test]
    fn rework_finds_the_task_holder_even_when_their_role_changed() {
        let mut task_holder = candidate("task-holder", Backend::Codex, Some("T-1"), true);
        task_holder.role = "reviewer".into();
        assert_eq!(
            choose(&request(rework("task-holder")), &[task_holder]),
            AssignmentDecision::Assigned {
                instance_id: "task-holder".into()
            }
        );
    }

    /// D33 rule 3: a task holder at its usage limit hands the task, with branch and
    /// review comments, to a free same-role member on another backend; else
    /// an ephemeral instance on another backend; else it queues.
    #[test]
    fn usage_limited_rework_moves_to_another_backend_with_the_branch() {
        let task_holder = candidate("task-holder", Backend::Codex, Some("T-1"), false);
        let same_backend = candidate("codex-2", Backend::Codex, None, true);
        let other_backend = candidate("claude-1", Backend::Claude, None, true);
        assert_eq!(
            choose(
                &request(rework("task-holder")),
                &[task_holder.clone(), same_backend.clone(), other_backend]
            ),
            AssignmentDecision::Reassigned {
                instance_id: "claude-1".into(),
                handoff: handoff("task-holder"),
            }
        );
        let spawn = AssignmentRequest {
            available_backends: vec![Backend::Codex, Backend::Opencode],
            ..request(rework("task-holder"))
        };
        assert_eq!(
            choose(&spawn, &[task_holder.clone(), same_backend.clone()]),
            AssignmentDecision::SpawnEphemeral {
                backend: Backend::Opencode,
                handoff: Some(handoff("task-holder")),
            }
        );
        let full = AssignmentRequest {
            role_capacity: capacity(3, 3),
            ..spawn
        };
        assert_eq!(
            choose(&full, &[task_holder, same_backend]),
            AssignmentDecision::Queue {
                reason: QueueReason::UsageLimit
            }
        );
    }

    /// D33 rule 3: a task holder removed by the operator is replaced immediately
    /// by any free same-role member, or an ephemeral instance within
    /// headcount.
    #[test]
    fn removed_task_holder_is_replaced_immediately() {
        let same_backend = candidate("codex-2", Backend::Codex, None, true);
        assert_eq!(
            choose(&request(rework("task-holder")), &[same_backend]),
            AssignmentDecision::Reassigned {
                instance_id: "codex-2".into(),
                handoff: handoff("task-holder"),
            }
        );
        let busy = candidate("codex-2", Backend::Codex, Some("T-9"), true);
        assert_eq!(
            choose(
                &request(rework("task-holder")),
                core::slice::from_ref(&busy)
            ),
            AssignmentDecision::SpawnEphemeral {
                backend: Backend::Claude,
                handoff: Some(handoff("task-holder")),
            }
        );
        let full = AssignmentRequest {
            role_capacity: capacity(1, 1),
            ..request(rework("task-holder"))
        };
        assert_eq!(
            choose(&full, &[busy]),
            AssignmentDecision::Queue {
                reason: QueueReason::AtCapacity
            }
        );
    }

    /// D33 rule 3: an ephemeral instance is reclaimed only after its task.
    #[test]
    fn ephemeral_task_holder_is_not_reclaimed_before_its_task_ends() {
        let mut task_holder = candidate("eph-1", Backend::Claude, Some("T-1"), true);
        task_holder.ephemeral = true;
        assert!(!may_reclaim(&task_holder));
        task_holder.held_task = None;
        assert!(may_reclaim(&task_holder));
        let persistent = candidate("dev-1", Backend::Claude, None, true);
        assert!(!may_reclaim(&persistent));
    }

    #[test]
    fn exhausted_backend_is_replaced_by_an_allowed_backend() {
        let candidates = [
            candidate("codex", Backend::Codex, None, false),
            candidate("claude", Backend::Claude, None, true),
        ];
        assert_eq!(
            choose(&request(Purpose::NewTask), &candidates),
            AssignmentDecision::Assigned {
                instance_id: "claude".into()
            }
        );
    }

    #[test]
    fn missing_role_becomes_an_ask() {
        let request = AssignmentRequest {
            role: "analyst".into(),
            role_capacity: None,
            ..request(Purpose::NewTask)
        };
        assert_eq!(
            choose(&request, &[candidate("dev", Backend::Codex, None, true)]),
            AssignmentDecision::AskForRole {
                role: "analyst".into()
            }
        );
    }

    #[test]
    fn role_with_no_live_instance_spawns_ephemeral_below_its_headcount_maximum() {
        let request = AssignmentRequest {
            role_capacity: capacity(0, 2),
            ..request(Purpose::NewTask)
        };
        assert_eq!(
            choose(&request, &[]),
            AssignmentDecision::SpawnEphemeral {
                backend: Backend::Claude,
                handoff: None,
            }
        );
    }

    #[test]
    fn a_team_wait_cycle_is_detected() {
        let existing = [WaitEdge {
            team_id: "team-a".into(),
            waiter: "A".into(),
            waits_for: "B".into(),
        }];
        assert!(creates_wait_cycle(
            &existing,
            &WaitEdge {
                team_id: "team-a".into(),
                waiter: "B".into(),
                waits_for: "A".into(),
            }
        ));
        assert!(!creates_wait_cycle(
            &existing,
            &WaitEdge {
                team_id: "team-b".into(),
                waiter: "B".into(),
                waits_for: "A".into(),
            }
        ));
    }

    /// Round-2 review N6: "prefer a different backend" must not become "only a
    /// different backend": with one allowed backend a reviewer is spawned on
    /// it instead of queueing for a usage limit that is not reached.
    #[test]
    fn review_n6_review_spawns_on_the_task_holder_backend_when_it_is_the_only_one() {
        let request = AssignmentRequest {
            team_id: "team-a".into(),
            role: "reviewer".into(),
            allowed_backends: vec![Backend::Claude],
            available_backends: vec![Backend::Claude],
            role_capacity: capacity(0, 2),
            purpose: Purpose::Review {
                task_holder: "dev-1".into(),
                task_holder_backend: Backend::Claude,
            },
        };
        assert_eq!(
            choose(&request, &[]),
            AssignmentDecision::SpawnEphemeral {
                backend: Backend::Claude,
                handoff: None,
            }
        );
        let request = AssignmentRequest {
            allowed_backends: vec![Backend::Claude, Backend::Codex],
            available_backends: vec![Backend::Claude, Backend::Codex],
            ..request
        };
        assert_eq!(
            choose(&request, &[]),
            AssignmentDecision::SpawnEphemeral {
                backend: Backend::Codex,
                handoff: None,
            }
        );
    }

    /// Verifier (D33 edge cases): the task holder counts only while it is in
    /// the same team, holds this task and runs on an allowed backend, and the
    /// role still exists; otherwise the task is reassigned with its handoff.
    #[test]
    fn verifier_task_holder_that_no_longer_qualifies_is_treated_as_removed() {
        let free = candidate("dev-2", Backend::Claude, None, true);
        let reassigned = AssignmentDecision::Reassigned {
            instance_id: "dev-2".into(),
            handoff: handoff("task-holder"),
        };

        let mut other_team = candidate("task-holder", Backend::Codex, Some("T-1"), true);
        other_team.team_id = "team-b".into();
        assert_eq!(
            choose(&request(rework("task-holder")), &[other_team, free.clone()]),
            reassigned
        );

        let other_task = candidate("task-holder", Backend::Codex, Some("T-9"), true);
        assert_eq!(
            choose(&request(rework("task-holder")), &[other_task, free.clone()]),
            reassigned
        );

        let disallowed = candidate("task-holder", Backend::Codex, Some("T-1"), true);
        let only_claude = AssignmentRequest {
            allowed_backends: vec![Backend::Claude],
            ..request(rework("task-holder"))
        };
        assert_eq!(
            choose(&only_claude, &[disallowed, free.clone()]),
            reassigned
        );

        let no_role = AssignmentRequest {
            role_capacity: None,
            ..request(rework("task-holder"))
        };
        let task_holder = candidate("task-holder", Backend::Codex, Some("T-1"), true);
        assert_eq!(
            choose(&no_role, &[task_holder, free]),
            AssignmentDecision::AskForRole { role: "dev".into() }
        );
    }
}
