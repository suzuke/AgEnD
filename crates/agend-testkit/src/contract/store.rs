//! `Store` contract: tasks round-trip exactly, writes are compare-and-swap on
//! a version that strictly grows with every write (checked over a sequence
//! of writes, so a version that returns to an old value fails), and a conflict changes nothing and
//! reports the current version.
//!
//! Not pinned: the first version number; what appending an event to an
//! unknown task does.

use std::fmt::Debug;

use agend_core::pipeline::task::{Task, TaskStatus};
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{CasResult, Store, StoredEvent};

use super::{Case, CaseResult, Report, ensure, ok, run_suite};
use crate::block_on;

pub trait StoreFixture {
    type Store: Store<Error = Self::Error>;
    type Error: Send + Debug;

    fn store(&self) -> &Self::Store;

    /// Saves a workflow version through whatever path the implementation
    /// uses outside the trait.
    fn insert_workflow(&self, workflow: &Workflow);

    /// Events appended for `task_id`, in append order.
    fn events(&self, task_id: &str) -> Vec<StoredEvent>;
}

pub fn cases<F: StoreFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            name: "created_task_round_trips",
            check: created_task_round_trips,
        },
        Case {
            name: "unknown_task_loads_as_none",
            check: unknown_task_loads_as_none,
        },
        Case {
            name: "duplicate_create_fails_and_keeps_the_original",
            check: duplicate_create_fails_and_keeps_the_original,
        },
        Case {
            name: "cas_with_current_version_writes_a_newer_version",
            check: cas_with_current_version_writes_a_newer_version,
        },
        Case {
            name: "cas_with_stale_version_conflicts_and_changes_nothing",
            check: cas_with_stale_version_conflicts_and_changes_nothing,
        },
        Case {
            name: "cas_on_unknown_task_conflicts_without_a_version",
            check: cas_on_unknown_task_conflicts_without_a_version,
        },
        Case {
            name: "workflow_versions_load_exactly",
            check: workflow_versions_load_exactly,
        },
        Case {
            name: "appended_events_keep_their_order",
            check: appended_events_keep_their_order,
        },
    ]
}

pub fn run<F: StoreFixture>(implementation: &str, make: impl FnMut() -> F) -> Report {
    run_suite("Store", implementation, &cases::<F>(), make)
}

/// A task with every optional field set, so a store that drops one fails.
fn full_task(id: &str) -> Task {
    let mut task = Task::new(id, "contract task", "team-a", "code", 3).set_requires_repo(true);
    task.parent = Some("T-parent".into());
    task.depends_on = vec!["T-dep-1".into(), "T-dep-2".into()];
    task.assignee = Some("dev-1".into());
    task.status = TaskStatus::Running;
    task
}

fn created_task_round_trips<F: StoreFixture>(fx: &F) -> CaseResult {
    let task = full_task("T-roundtrip");
    ok("create_task", block_on(fx.store().create_task(&task)))?;
    let loaded = ok("load_task", block_on(fx.store().load_task(&task.id)))?;
    match loaded {
        Some(versioned) if versioned.task == task => Ok(()),
        other => Err(format!("expected the created task back, got {other:?}")),
    }
}

fn unknown_task_loads_as_none<F: StoreFixture>(fx: &F) -> CaseResult {
    let loaded = ok("load_task", block_on(fx.store().load_task("T-missing")))?;
    ensure(loaded.is_none(), || {
        format!("expected None, got {loaded:?}")
    })
}

fn duplicate_create_fails_and_keeps_the_original<F: StoreFixture>(fx: &F) -> CaseResult {
    let task = full_task("T-dup");
    ok("create_task", block_on(fx.store().create_task(&task)))?;
    let mut other = task.clone();
    other.title = "overwritten".into();
    let second = block_on(fx.store().create_task(&other));
    ensure(second.is_err(), || {
        "creating an existing task id succeeded".into()
    })?;
    let loaded = ok("load_task", block_on(fx.store().load_task(&task.id)))?;
    ensure(loaded.as_ref().map(|v| &v.task) == Some(&task), || {
        format!("the original task changed: {loaded:?}")
    })
}

fn cas_with_current_version_writes_a_newer_version<F: StoreFixture>(fx: &F) -> CaseResult {
    let task = full_task("T-cas");
    ok("create_task", block_on(fx.store().create_task(&task)))?;
    let mut version = current_version(fx, &task.id)?;
    // Several writes in a row: a version that only has to differ from the
    // previous one (1 -> 2 -> 1) would let a stale writer succeed (ABA).
    for (n, status) in WRITE_SEQUENCE.into_iter().enumerate() {
        let mut next = task.clone();
        next.status = status;
        next.title = format!("write {n}");
        let result = ok(
            "compare_and_swap_task",
            block_on(fx.store().compare_and_swap_task(&next, version)),
        )?;
        let CasResult::Written { new_version } = result else {
            return Err(format!("write {n}: expected Written, got {result:?}"));
        };
        ensure(new_version > version, || {
            format!("write {n}: new version {new_version} is not greater than {version}")
        })?;
        let loaded = ok("load_task", block_on(fx.store().load_task(&task.id)))?;
        ensure(
            loaded.as_ref().map(|v| (v.version, &v.task)) == Some((new_version, &next)),
            || {
                format!(
                    "write {n}: expected version {new_version} with the new task, got {loaded:?}"
                )
            },
        )?;
        version = new_version;
    }
    Ok(())
}

/// Statuses written one after another by the CAS case.
const WRITE_SEQUENCE: [TaskStatus; 4] = [
    TaskStatus::Blocked,
    TaskStatus::Running,
    TaskStatus::Blocked,
    TaskStatus::Running,
];

fn cas_with_stale_version_conflicts_and_changes_nothing<F: StoreFixture>(fx: &F) -> CaseResult {
    let task = full_task("T-stale");
    ok("create_task", block_on(fx.store().create_task(&task)))?;
    let stale = current_version(fx, &task.id)?;
    let mut first = task.clone();
    first.title = "first writer".into();
    ok(
        "compare_and_swap_task",
        block_on(fx.store().compare_and_swap_task(&first, stale)),
    )?;
    let current = current_version(fx, &task.id)?;
    let mut second = task.clone();
    second.title = "second writer".into();
    let result = ok(
        "compare_and_swap_task",
        block_on(fx.store().compare_and_swap_task(&second, stale)),
    )?;
    let expected = CasResult::Conflict {
        current_version: Some(current),
    };
    ensure(result == expected, || {
        format!("expected {expected:?}, got {result:?}")
    })?;
    let loaded = ok("load_task", block_on(fx.store().load_task(&task.id)))?;
    ensure(loaded.map(|v| v.task) == Some(first), || {
        "a conflicting write changed the task".into()
    })
}

fn cas_on_unknown_task_conflicts_without_a_version<F: StoreFixture>(fx: &F) -> CaseResult {
    let result = ok(
        "compare_and_swap_task",
        block_on(fx.store().compare_and_swap_task(&full_task("T-none"), 1)),
    )?;
    let expected = CasResult::Conflict {
        current_version: None,
    };
    ensure(result == expected, || {
        format!("expected {expected:?}, got {result:?}")
    })
}

fn workflow_versions_load_exactly<F: StoreFixture>(fx: &F) -> CaseResult {
    let v1 = Workflow::builtin_code();
    let mut v2 = v1.clone();
    v2.version = v1.version + 1;
    v2.allow_unreviewed = !v1.allow_unreviewed;
    fx.insert_workflow(&v1);
    fx.insert_workflow(&v2);
    for wanted in [&v1, &v2] {
        let loaded = ok(
            "load_workflow",
            block_on(fx.store().load_workflow(&wanted.id, wanted.version)),
        )?;
        ensure(loaded.as_ref() == Some(wanted), || {
            format!("version {} loaded as {loaded:?}", wanted.version)
        })?;
    }
    let missing = ok(
        "load_workflow",
        block_on(fx.store().load_workflow(&v1.id, v2.version + 1)),
    )?;
    ensure(missing.is_none(), || {
        format!("an unsaved version loaded as {missing:?}")
    })
}

fn appended_events_keep_their_order<F: StoreFixture>(fx: &F) -> CaseResult {
    let task = full_task("T-events");
    ok("create_task", block_on(fx.store().create_task(&task)))?;
    let events: Vec<StoredEvent> = (1..=3)
        .map(|n| StoredEvent {
            id: format!("e-{n}"),
            occurred_at_unix_ms: 1_790_000_000_000 + n,
            kind: "stage_completed".into(),
            detail: format!("step {n}"),
        })
        .collect();
    for event in &events {
        ok(
            "append_event",
            block_on(fx.store().append_event(&task.id, event)),
        )?;
    }
    let stored = fx.events(&task.id);
    ensure(stored == events, || {
        format!("expected {events:?}, got {stored:?}")
    })
}

fn current_version<F: StoreFixture>(fx: &F, task_id: &str) -> Result<u64, String> {
    ok("load_task", block_on(fx.store().load_task(task_id)))?
        .map(|v| v.version)
        .ok_or_else(|| format!("task {task_id} vanished"))
}
