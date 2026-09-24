//! Assignment rules for role templates (D18, D25). The daemon supplies team
//! candidates, loads and allowed backends; core makes the deterministic choice.
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
    pub active_tasks: usize,
    pub waiting_fanout_parents: usize,
    /// Per-instance concurrency supplied by the daemon's session policy.
    pub max_concurrent_tasks: usize,
    pub usage_available: bool,
}

impl Candidate {
    pub fn occupied_slots(&self) -> usize {
        self.active_tasks
            .saturating_sub(self.waiting_fanout_parents)
    }

    pub fn has_capacity(&self) -> bool {
        self.occupied_slots() < self.max_concurrent_tasks
    }
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
        author_instance: String,
        author_backend: Backend,
    },
    Rework {
        original_author: String,
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
    ReworkAuthorUnavailable,
    InvalidRoleCapacity,
    NoAllowedBackend,
    NoEligibleReviewer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignmentDecision {
    Assigned { instance_id: String },
    SpawnEphemeral { backend: Backend },
    Queue { reason: QueueReason },
    AskForRole { role: String },
}

pub fn choose(request: &AssignmentRequest, candidates: &[Candidate]) -> AssignmentDecision {
    if let Purpose::Rework { original_author } = &request.purpose {
        let author = candidates.iter().find(|candidate| {
            candidate.team_id == request.team_id && candidate.instance_id == *original_author
        });
        let Some(author) = author else {
            return AssignmentDecision::Queue {
                reason: QueueReason::ReworkAuthorUnavailable,
            };
        };
        if author.usage_available && author.has_capacity() {
            return AssignmentDecision::Assigned {
                instance_id: author.instance_id.clone(),
            };
        }
        if !author.usage_available {
            if let Some(candidate) = eligible_candidates(request, candidates, Some(original_author))
                .into_iter()
                .find(|candidate| candidate.backend != author.backend)
            {
                return AssignmentDecision::Assigned {
                    instance_id: candidate.instance_id.clone(),
                };
            }
            if let Some(backend) = spawn_backend(request, Some(author.backend)) {
                return AssignmentDecision::SpawnEphemeral { backend };
            }
            return AssignmentDecision::Queue {
                reason: QueueReason::UsageLimit,
            };
        }
        return AssignmentDecision::Queue {
            reason: QueueReason::AtCapacity,
        };
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

    let mut eligible = eligible_candidates(request, candidates, None);

    if let Purpose::Review {
        author_instance,
        author_backend,
    } = &request.purpose
    {
        eligible.retain(|candidate| candidate.instance_id != *author_instance);
        eligible.sort_by_key(|candidate| {
            (
                candidate.backend == *author_backend,
                candidate.occupied_slots(),
                candidate.instance_id.as_str(),
            )
        });
    } else {
        eligible
            .sort_by_key(|candidate| (candidate.occupied_slots(), candidate.instance_id.as_str()));
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
    // Review prefers a backend other than the author's, but falls back to
    // the same backend rather than queueing when it is the only one allowed.
    let preferred_spawn = match &request.purpose {
        Purpose::Review { author_backend, .. } => spawn_backend(request, Some(*author_backend)),
        _ => None,
    };
    if let Some(backend) = preferred_spawn.or_else(|| spawn_backend(request, None)) {
        return AssignmentDecision::SpawnEphemeral { backend };
    }

    if let Purpose::Review {
        author_instance, ..
    } = &request.purpose
        && !role_candidates.is_empty()
        && role_candidates
            .iter()
            .all(|candidate| candidate.instance_id == *author_instance)
    {
        return AssignmentDecision::Queue {
            reason: QueueReason::NoEligibleReviewer,
        };
    }

    if request
        .role_capacity
        .is_some_and(|capacity| capacity.current_instances >= capacity.maximum_instances)
    {
        return AssignmentDecision::Queue {
            reason: QueueReason::AtCapacity,
        };
    }
    AssignmentDecision::Queue {
        reason: QueueReason::UsageLimit,
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

fn eligible_candidates<'a>(
    request: &AssignmentRequest,
    candidates: &'a [Candidate],
    exclude_instance: Option<&str>,
) -> Vec<&'a Candidate> {
    let mut eligible: Vec<&Candidate> = role_candidates(request, candidates)
        .into_iter()
        .filter(|candidate| {
            is_allowed(candidate, &request.allowed_backends)
                && candidate.usage_available
                && candidate.has_capacity()
                && exclude_instance != Some(candidate.instance_id.as_str())
        })
        .collect();
    eligible.sort_by_key(|candidate| (candidate.occupied_slots(), candidate.instance_id.as_str()));
    eligible
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

    fn candidate(
        id: &str,
        backend: Backend,
        active_tasks: usize,
        waiting_parents: usize,
        max_concurrent_tasks: usize,
        usage_available: bool,
    ) -> Candidate {
        Candidate {
            instance_id: id.into(),
            team_id: "team-a".into(),
            role: "dev".into(),
            backend,
            active_tasks,
            waiting_fanout_parents: waiting_parents,
            max_concurrent_tasks,
            usage_available,
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

    #[test]
    fn review_excludes_author_and_prefers_another_backend() {
        let candidates = [
            candidate("author", Backend::Codex, 0, 0, 2, true),
            candidate("same", Backend::Codex, 0, 0, 2, true),
            candidate("other", Backend::Claude, 1, 0, 2, true),
        ];
        assert_eq!(
            choose(
                &request(Purpose::Review {
                    author_instance: "author".into(),
                    author_backend: Backend::Codex,
                }),
                &candidates,
            ),
            AssignmentDecision::Assigned {
                instance_id: "other".into()
            }
        );
    }

    #[test]
    fn rework_goes_back_to_the_original_author() {
        let mut author = candidate("author", Backend::Codex, 0, 0, 1, true);
        author.role = "reviewer".into();
        let candidates = [author, candidate("other", Backend::Claude, 0, 0, 1, true)];
        assert_eq!(
            choose(
                &request(Purpose::Rework {
                    original_author: "author".into(),
                }),
                &candidates,
            ),
            AssignmentDecision::Assigned {
                instance_id: "author".into()
            }
        );
    }

    #[test]
    fn rework_finds_the_original_author_even_when_their_role_changed() {
        let mut author = candidate("author", Backend::Codex, 0, 0, 1, true);
        author.role = "reviewer".into();
        let request = AssignmentRequest {
            role: "dev".into(),
            purpose: Purpose::Rework {
                original_author: "author".into(),
            },
            ..request(Purpose::NewTask)
        };
        assert_eq!(
            choose(&request, &[author]),
            AssignmentDecision::Assigned {
                instance_id: "author".into()
            }
        );
    }

    #[test]
    fn fanout_waiting_parent_does_not_consume_a_slot() {
        let candidate = candidate("dev", Backend::Codex, 2, 1, 2, true);
        assert_eq!(candidate.occupied_slots(), 1);
        assert!(candidate.has_capacity());
    }

    #[test]
    fn exhausted_backend_is_replaced_by_an_allowed_backend() {
        let candidates = [
            candidate("codex", Backend::Codex, 0, 0, 1, false),
            candidate("claude", Backend::Claude, 0, 0, 1, true),
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
            choose(&request, &[candidate("dev", Backend::Codex, 0, 0, 1, true)]),
            AssignmentDecision::AskForRole {
                role: "analyst".into()
            }
        );
    }

    #[test]
    fn role_with_no_live_instance_spawns_ephemeral_below_its_headcount_maximum() {
        let request = AssignmentRequest {
            role_capacity: Some(RoleCapacity {
                minimum_instances: 0,
                maximum_instances: 2,
                current_instances: 0,
            }),
            ..request(Purpose::NewTask)
        };
        assert_eq!(
            choose(&request, &[]),
            AssignmentDecision::SpawnEphemeral {
                backend: Backend::Claude
            }
        );
    }

    #[test]
    fn role_headcount_maximum_queues_when_no_candidate_has_capacity() {
        let request = AssignmentRequest {
            role_capacity: Some(RoleCapacity {
                minimum_instances: 1,
                maximum_instances: 1,
                current_instances: 1,
            }),
            ..request(Purpose::NewTask)
        };
        let full = candidate("dev", Backend::Codex, 1, 0, 1, true);
        assert_eq!(
            choose(&request, &[full]),
            AssignmentDecision::Queue {
                reason: QueueReason::AtCapacity
            }
        );
    }

    #[test]
    fn rework_returns_to_author_or_uses_an_allowed_backend_after_quota_exhaustion() {
        let author = candidate("author", Backend::Codex, 0, 0, 1, true);
        let other = candidate("other", Backend::Claude, 0, 0, 1, true);
        assert_eq!(
            choose(
                &request(Purpose::Rework {
                    original_author: "author".into(),
                }),
                &[author.clone(), other],
            ),
            AssignmentDecision::Assigned {
                instance_id: "author".into()
            }
        );
        let mut request = request(Purpose::Rework {
            original_author: "author".into(),
        });
        request.available_backends = alloc::vec![Backend::Claude];
        let exhausted_author = candidate("author", Backend::Codex, 0, 0, 1, false);
        assert_eq!(
            choose(&request, &[exhausted_author]),
            AssignmentDecision::SpawnEphemeral {
                backend: Backend::Claude
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
    fn review_n6_review_spawns_on_the_author_backend_when_it_is_the_only_one() {
        let request = AssignmentRequest {
            team_id: "team-a".into(),
            role: "reviewer".into(),
            allowed_backends: alloc::vec![Backend::Claude],
            available_backends: alloc::vec![Backend::Claude],
            role_capacity: Some(RoleCapacity {
                minimum_instances: 0,
                maximum_instances: 2,
                current_instances: 0,
            }),
            purpose: Purpose::Review {
                author_instance: "dev-1".into(),
                author_backend: Backend::Claude,
            },
        };
        assert_eq!(
            choose(&request, &[]),
            AssignmentDecision::SpawnEphemeral {
                backend: Backend::Claude
            }
        );
        let request = AssignmentRequest {
            allowed_backends: alloc::vec![Backend::Claude, Backend::Codex],
            available_backends: alloc::vec![Backend::Claude, Backend::Codex],
            ..request
        };
        assert_eq!(
            choose(&request, &[]),
            AssignmentDecision::SpawnEphemeral {
                backend: Backend::Codex
            }
        );
    }
}
