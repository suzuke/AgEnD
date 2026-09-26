//! `plan_boot` (gate 6 P2): one pure function decides, at daemon boot, what
//! happens to every instance in the DB and every running holder, so
//! reconnecting, starting and orphan cleanup are decided in one place.
//!
//! | DB | holder lock held | action |
//! |---|---|---|
//! | yes (not `failed`) | yes | reconnect |
//! | yes (not `failed`) | no | start (resume only when `running`: its session exists) |
//! | yes, `failed` | either | nothing: a human decides (gate 6 P6) |
//! | no | yes | orphan: send `Shutdown` |
//!
//! Must NOT: do I/O; the daemon gathers the inputs and runs the actions.

use crate::store::{Instance, InstanceStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootAction {
    /// The holder runs: reconnect to it.
    Reconnect { id: String },
    /// No holder runs: start one; `resume` when it is `running` (its session exists).
    Start { id: String, resume: bool },
    /// A holder runs for an instance the DB does not have: stop it.
    Orphan { id: String },
}

/// Actions for `instances` (from the DB) and `running` (ids whose holder
/// lock is held): instances in DB order first, then orphans in `running`
/// order.
pub fn plan_boot(instances: &[Instance], running: &[String]) -> Vec<BootAction> {
    let mut actions: Vec<BootAction> = instances
        .iter()
        .filter(|i| i.status != InstanceStatus::Failed)
        .map(|i| {
            let id = i.id.clone();
            if running.contains(&i.id) {
                BootAction::Reconnect { id }
            } else {
                let resume = i.status == InstanceStatus::Running;
                BootAction::Start { id, resume }
            }
        })
        .collect();
    actions.extend(
        running
            .iter()
            .filter(|id| !instances.iter().any(|i| &i.id == *id))
            .map(|id| BootAction::Orphan { id: id.clone() }),
    );
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use BootAction::*;
    use agend_core::model::Backend;

    fn instance(id: &str, status: InstanceStatus) -> Instance {
        Instance {
            id: id.into(),
            backend: Backend::Claude,
            program: "/bin/bash".into(),
            args: vec![],
            working_directory: "/tmp".into(),
            session_id: Some("s-1".into()),
            status,
        }
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| (*s).to_owned()).collect()
    }

    fn id(s: &str) -> String {
        s.to_owned()
    }

    #[test]
    fn each_row_of_the_boot_table() {
        use InstanceStatus::{Failed, New, Running};
        // (DB instances, running holders, expected actions)
        let table: Vec<(Vec<Instance>, Vec<String>, Vec<BootAction>)> = vec![
            // The page's example (P2).
            (
                vec![instance("g6-1", Running), instance("g6-2", Running)],
                ids(&["g6-1", "g6-9"]),
                vec![
                    Reconnect { id: id("g6-1") },
                    Start {
                        id: id("g6-2"),
                        resume: true,
                    },
                    Orphan { id: id("g6-9") },
                ],
            ),
            // Nothing at all.
            (vec![], vec![], vec![]),
            // Never started: the first start is fresh.
            (
                vec![instance("a", New)],
                vec![],
                vec![Start {
                    id: id("a"),
                    resume: false,
                }],
            ),
            // A new instance whose holder runs (the daemon died before its
            // first `Spawn` was acknowledged): reconnect; the re-sent
            // `Spawn` starts the session.
            (
                vec![instance("a", New)],
                ids(&["a"]),
                vec![Reconnect { id: id("a") }],
            ),
            // Failed: left alone, with or without a holder; never an orphan.
            (
                vec![instance("a", Failed), instance("b", Failed)],
                ids(&["a"]),
                vec![],
            ),
            // Only orphans.
            (
                vec![],
                ids(&["x", "y"]),
                vec![Orphan { id: id("x") }, Orphan { id: id("y") }],
            ),
        ];
        for (n, (instances, running, expected)) in table.into_iter().enumerate() {
            assert_eq!(plan_boot(&instances, &running), expected, "row {n}");
        }
    }
}
