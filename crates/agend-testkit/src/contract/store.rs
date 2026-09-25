//! `Store` contract (rules STO-1..12 in CONTRACTS.md): tasks round-trip
//! exactly; writes are compare-and-swap on a version that must equal the
//! current one (older and newer both conflict) and strictly grows with
//! every write (checked over a sequence, so a version that returns to an old
//! value fails); a conflict changes nothing and reports the version actually
//! stored; events keep their order and stay with their task; everything,
//! versions included, survives every reopen ([`StoreFixture::boot`]: a
//! daemon restart, also one after a boot that only opened the store), and
//! versions keep growing across reopens. The reopen cases run a whole daemon
//! lifecycle ([`super::daemon_lifecycle`]).
//!
//! Not pinned: the first version number; what appending an event to an
//! unknown task does.

use std::fmt::Debug;

use agend_core::pipeline::task::{Task, TaskStatus};
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{CasResult, Store, StoredEvent};

use super::{Boot, Case, CaseResult, Report, daemon_lifecycle, ensure, ok, run_suite};
use crate::block_on;

pub trait StoreFixture: Sized {
    type Store: Store<Error = Self::Error>;
    type Error: Send + Debug;
    /// What survives a daemon restart: the database (a real fixture: the
    /// database file; the fake: [`crate::fakes::FakeStoreFile`]). It is not
    /// a store handle and keeps none open.
    type Persisted;

    fn store(&self) -> &Self::Store;

    /// Saves a workflow version through whatever path the implementation
    /// uses outside the trait.
    fn insert_workflow(&self, workflow: &Workflow);

    /// Events appended for `task_id`, in append order.
    fn events(&self, task_id: &str) -> Vec<StoredEvent>;

    /// The persisted state this fixture's store works on.
    fn persisted(&self) -> Self::Persisted;

    /// A daemon boot: a new fixture whose store is a new handle opened on
    /// `persisted` (the same database file). Restart cases drop the case's
    /// fixture before the first boot and each booted one before the next,
    /// so no handle is open in between. A real fixture goes through the real
    /// persistence (the database file), never a process-global static or a
    /// shared in-memory database (CONTRACTS.md).
    fn boot(persisted: &Self::Persisted) -> Self;
}

pub fn cases<F: StoreFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            rule: "STO-1",
            name: "created_task_round_trips",
            check: |fx| created_task_round_trips(&fx),
        },
        Case {
            rule: "STO-2",
            name: "unknown_task_loads_as_none",
            check: |fx| unknown_task_loads_as_none(&fx),
        },
        Case {
            rule: "STO-3",
            name: "duplicate_create_fails_and_keeps_the_original",
            check: |fx| duplicate_create_fails_and_keeps_the_original(&fx),
        },
        Case {
            rule: "STO-4",
            name: "cas_with_current_version_writes_a_newer_version",
            check: |fx| cas_with_current_version_writes_a_newer_version(&fx),
        },
        Case {
            rule: "STO-4",
            name: "versions_keep_growing_across_reopens",
            check: versions_keep_growing_across_reopens,
        },
        Case {
            rule: "STO-5",
            name: "cas_with_stale_version_conflicts_and_changes_nothing",
            check: |fx| cas_with_stale_version_conflicts_and_changes_nothing(&fx),
        },
        Case {
            rule: "STO-6",
            name: "cas_with_future_version_conflicts_and_changes_nothing",
            check: |fx| cas_with_future_version_conflicts_and_changes_nothing(&fx),
        },
        Case {
            rule: "STO-7",
            name: "conflict_reports_the_actual_current_version",
            check: |fx| conflict_reports_the_actual_current_version(&fx),
        },
        Case {
            rule: "STO-8",
            name: "cas_on_unknown_task_conflicts_without_a_version",
            check: |fx| cas_on_unknown_task_conflicts_without_a_version(&fx),
        },
        Case {
            rule: "STO-9",
            name: "workflow_versions_load_exactly",
            check: |fx| workflow_versions_load_exactly(&fx),
        },
        Case {
            rule: "STO-10",
            name: "appended_events_keep_their_order",
            check: |fx| appended_events_keep_their_order(&fx),
        },
        Case {
            rule: "STO-11",
            name: "events_are_kept_per_task",
            check: |fx| events_are_kept_per_task(&fx),
        },
        Case {
            rule: "STO-12",
            name: "data_survives_every_reopen",
            check: data_survives_every_reopen,
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
    let first = write(fx, &task, "first writer", stale)?;
    let mut second = task.clone();
    second.title = "second writer".into();
    let result = ok(
        "compare_and_swap_task",
        block_on(fx.store().compare_and_swap_task(&second, stale)),
    )?;
    ensure(
        matches!(
            result,
            CasResult::Conflict {
                current_version: Some(_)
            }
        ),
        || format!("a stale version {stale}: expected Conflict with a version, got {result:?}"),
    )?;
    unchanged(fx, &first)
}

/// A writer that claims a version the store never issued (a newer one) must
/// not win either: CAS is equality, not "at least".
fn cas_with_future_version_conflicts_and_changes_nothing<F: StoreFixture>(fx: &F) -> CaseResult {
    let task = full_task("T-future");
    ok("create_task", block_on(fx.store().create_task(&task)))?;
    let first = write(fx, &task, "first writer", current_version(fx, &task.id)?)?;
    let current = current_version(fx, &task.id)?;
    for future in [current + 1, current + 100] {
        let mut other = task.clone();
        other.title = format!("writer claiming version {future}");
        let result = ok(
            "compare_and_swap_task",
            block_on(fx.store().compare_and_swap_task(&other, future)),
        )?;
        ensure(matches!(result, CasResult::Conflict { .. }), || {
            format!("version {future} (current {current}): expected Conflict, got {result:?}")
        })?;
        unchanged(fx, &first)?;
    }
    Ok(())
}

/// The conflict names the version actually stored, not a guess derived from
/// the caller's version (three writes apart, so `expected + 1` is wrong).
fn conflict_reports_the_actual_current_version<F: StoreFixture>(fx: &F) -> CaseResult {
    let task = full_task("T-current");
    ok("create_task", block_on(fx.store().create_task(&task)))?;
    let original = current_version(fx, &task.id)?;
    let mut version = original;
    for n in 0..3 {
        write(fx, &task, &format!("write {n}"), version)?;
        version = current_version(fx, &task.id)?;
    }
    for claimed in [original, version + 7] {
        let result = ok(
            "compare_and_swap_task",
            block_on(fx.store().compare_and_swap_task(&task, claimed)),
        )?;
        let expected = CasResult::Conflict {
            current_version: Some(version),
        };
        ensure(result == expected, || {
            format!("version {claimed}: expected {expected:?}, got {result:?}")
        })?;
    }
    Ok(())
}

/// Writes `task` with `title` at `version`; returns what was written.
fn write<F: StoreFixture>(fx: &F, task: &Task, title: &str, version: u64) -> Result<Task, String> {
    let mut next = task.clone();
    next.title = title.into();
    let result = ok(
        "compare_and_swap_task",
        block_on(fx.store().compare_and_swap_task(&next, version)),
    )?;
    ensure(matches!(result, CasResult::Written { .. }), || {
        format!("write {title:?} at the current version {version}: got {result:?}")
    })?;
    Ok(next)
}

/// The stored task is still `expected`.
fn unchanged<F: StoreFixture>(fx: &F, expected: &Task) -> CaseResult {
    let loaded = ok("load_task", block_on(fx.store().load_task(&expected.id)))?;
    ensure(loaded.as_ref().map(|v| &v.task) == Some(expected), || {
        format!("a conflicting write changed the task: {loaded:?}")
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

/// Two tasks' events never mix, even when appended interleaved.
fn events_are_kept_per_task<F: StoreFixture>(fx: &F) -> CaseResult {
    let tasks = [full_task("T-events-a"), full_task("T-events-b")];
    for task in &tasks {
        ok("create_task", block_on(fx.store().create_task(task)))?;
    }
    let event = |task: &str, n: u64| StoredEvent {
        id: format!("{task}-e{n}"),
        occurred_at_unix_ms: 1_790_000_000_000 + n,
        kind: "stage_completed".into(),
        detail: format!("{task} step {n}"),
    };
    for n in 1..=2 {
        for task in &tasks {
            ok(
                "append_event",
                block_on(fx.store().append_event(&task.id, &event(&task.id, n))),
            )?;
        }
    }
    for task in &tasks {
        let expected = vec![event(&task.id, 1), event(&task.id, 2)];
        let stored = fx.events(&task.id);
        ensure(stored == expected, || {
            format!(
                "events of {}: expected {expected:?}, got {stored:?}",
                task.id
            )
        })?;
    }
    Ok(())
}

fn current_version<F: StoreFixture>(fx: &F, task_id: &str) -> Result<u64, String> {
    ok("load_task", block_on(fx.store().load_task(task_id)))?
        .map(|v| v.version)
        .ok_or_else(|| format!("task {task_id} vanished"))
}

/// A daemon lifecycle ([`daemon_lifecycle`]) for the store: every boot
/// opens the same data ([`StoreFixture::boot`]); the idle boot does nothing
/// else, the others run `boot`.
fn lifecycle<F: StoreFixture>(
    fx: F,
    mut boot: impl FnMut(usize, Boot, &F) -> CaseResult,
) -> CaseResult {
    daemon_lifecycle(
        fx,
        F::persisted,
        F::boot,
        |_, _| Ok(()),
        |n, kind, daemon| match kind {
            Boot::Idle => Ok(()),
            Boot::Work | Boot::Check => boot(n, kind, daemon),
        },
    )
}

/// STO-4 across reopens: every version the store ever issued, before any
/// reopen, stays behind. At every boot that is not idle the stored version
/// is the last one issued, a write gets a version greater than all of them,
/// and a writer holding any earlier one (from this boot or an earlier one)
/// conflicts.
fn versions_keep_growing_across_reopens<F: StoreFixture>(fx: F) -> CaseResult {
    let task = full_task("T-versions");
    let mut issued: Vec<u64> = Vec::new();
    let mut stored = task.clone();
    lifecycle(fx, |n, _, daemon| {
        if n == 1 {
            ok("create_task", block_on(daemon.store().create_task(&task)))?;
            issued.push(current_version(daemon, &task.id)?);
        }
        let mut held = current_version(daemon, &task.id)?;
        let last = issued.last().copied();
        ensure(Some(held) == last, || {
            format!("the stored version is {held}, but the last one issued was {last:?}")
        })?;
        for (k, status) in WRITE_SEQUENCE.into_iter().take(2).enumerate() {
            let mut next = task.clone();
            next.status = status;
            next.title = format!("boot {n} write {k}");
            let result = ok(
                "compare_and_swap_task",
                block_on(daemon.store().compare_and_swap_task(&next, held)),
            )?;
            let CasResult::Written { new_version } = result else {
                return Err(format!("write {k}: expected Written, got {result:?}"));
            };
            let highest = issued.iter().max().copied().unwrap_or(0);
            ensure(new_version > highest, || {
                format!(
                    "write {k}: new version {new_version} is not greater than every version issued before ({issued:?})"
                )
            })?;
            issued.push(new_version);
            held = new_version;
            stored = next;
        }
        for &old in &issued[..issued.len() - 1] {
            let result = ok(
                "compare_and_swap_task",
                block_on(daemon.store().compare_and_swap_task(&task, old)),
            )?;
            ensure(matches!(result, CasResult::Conflict { .. }), || {
                format!(
                    "a writer holding the earlier version {old} (issued {issued:?}) must conflict, got {result:?}"
                )
            })?;
        }
        unchanged(daemon, &stored)
    })
}

/// GLOSSARY store / D8: the store is the only source of truth, so tasks,
/// versions, workflows and events survive every restart, and CAS continues
/// from the stored version. Every boot that is not idle checks everything
/// the earlier boots wrote; a working boot then writes the task and appends
/// an event.
fn data_survives_every_reopen<F: StoreFixture>(fx: F) -> CaseResult {
    let task = full_task("T-reopen");
    let workflow = Workflow::builtin_code();
    let mut written = task.clone();
    let mut version = 0;
    let mut appended: Vec<StoredEvent> = Vec::new();
    lifecycle(fx, |n, kind, daemon| {
        if n == 1 {
            ok("create_task", block_on(daemon.store().create_task(&task)))?;
            version = current_version(daemon, &task.id)?;
            daemon.insert_workflow(&workflow);
        }
        let loaded = ok("load_task", block_on(daemon.store().load_task(&task.id)))?;
        ensure(
            loaded.as_ref().map(|v| (v.version, &v.task)) == Some((version, &written)),
            || format!("expected version {version} with {written:?}, got {loaded:?}"),
        )?;
        let loaded = ok(
            "load_workflow",
            block_on(daemon.store().load_workflow(&workflow.id, workflow.version)),
        )?;
        ensure(loaded.as_ref() == Some(&workflow), || {
            format!("the workflow loaded as {loaded:?}")
        })?;
        let events = daemon.events(&task.id);
        ensure(events == appended, || {
            format!("expected events {appended:?}, got {events:?}")
        })?;
        if kind == Boot::Check {
            return Ok(());
        }
        written = write(daemon, &written, &format!("written at boot {n}"), version)?;
        version = current_version(daemon, &task.id)?;
        let event = StoredEvent {
            id: format!("e-boot{n}"),
            occurred_at_unix_ms: 1_790_000_000_000 + n as u64,
            kind: "stage_completed".into(),
            detail: format!("boot {n}"),
        };
        ok(
            "append_event",
            block_on(daemon.store().append_event(&task.id, &event)),
        )?;
        appended.push(event);
        Ok(())
    })
}
