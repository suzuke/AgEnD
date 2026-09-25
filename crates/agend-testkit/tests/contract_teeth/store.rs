//! Store mutants: a `FakeStore` with one method replaced. Mutants that keep
//! events themselves also replace the fixture's view of them. `reopen`
//! builds the next mutant over `FakeStore::reopen` (the same data) with the
//! same replaced methods; a mutant's own event log does not survive it.
//! `CounterStore` (verifier r4) is standalone.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agend_core::pipeline::task::Task;
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{CasResult, Store, StoredEvent, VersionedTask};
use agend_testkit::block_on;
use agend_testkit::contract::store::{self, StoreFixture};
use agend_testkit::fakes::{FakeError, FakeStore};

use super::Mutant;

type Load = fn(&M, &str) -> Result<Option<VersionedTask>, FakeError>;
type Create = fn(&M, &Task) -> Result<(), FakeError>;
type Cas = fn(&M, &Task, u64) -> Result<CasResult, FakeError>;
type LoadWorkflow = fn(&M, &str, u64) -> Result<Option<Workflow>, FakeError>;
type Append = fn(&M, &str, &StoredEvent) -> Result<(), FakeError>;
type Events = fn(&M, &str) -> Vec<StoredEvent>;
type Reopen = fn(&M) -> FakeStore;

/// A fake store with one operation replaced.
pub struct M {
    store: FakeStore,
    /// Events a mutant keeps itself: `(task id, event)`.
    log: Mutex<Vec<(String, StoredEvent)>>,
    load: Load,
    create: Create,
    cas: Cas,
    load_workflow: LoadWorkflow,
    append: Append,
    events: Events,
    reopen: Reopen,
}

impl M {
    fn new() -> Self {
        Self {
            store: FakeStore::new(),
            log: Mutex::new(Vec::new()),
            load: |m, id| m.real_load(id),
            create: |m, t| block_on(m.store.create_task(t)),
            cas: |m, t, v| m.real_cas(t, v),
            load_workflow: |m, id, v| m.real_load_workflow(id, v),
            append: |m, id, e| block_on(m.store.append_event(id, e)),
            events: |m, id| m.store.events(id),
            reopen: |m| m.store.reopen(),
        }
    }

    fn real_load(&self, id: &str) -> Result<Option<VersionedTask>, FakeError> {
        block_on(self.store.load_task(id))
    }

    fn real_cas(&self, task: &Task, version: u64) -> Result<CasResult, FakeError> {
        block_on(self.store.compare_and_swap_task(task, version))
    }

    fn real_load_workflow(&self, id: &str, version: u64) -> Result<Option<Workflow>, FakeError> {
        block_on(self.store.load_workflow(id, version))
    }

    /// Writes `task` whatever the version.
    fn force_write(&self, task: &Task) -> Result<CasResult, FakeError> {
        match self.real_load(&task.id)? {
            Some(current) => self.real_cas(task, current.version),
            None => self.real_cas(task, 0),
        }
    }

    /// Events this mutant keeps itself for `task_id`, in stored order.
    fn logged(&self, task_id: &str) -> Vec<StoredEvent> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _)| id == task_id)
            .map(|(_, e)| e.clone())
            .collect()
    }
}

impl Store for M {
    type Error = FakeError;
    async fn load_task(&self, id: &str) -> Result<Option<VersionedTask>, FakeError> {
        (self.load)(self, id)
    }
    async fn create_task(&self, task: &Task) -> Result<(), FakeError> {
        (self.create)(self, task)
    }
    async fn compare_and_swap_task(&self, task: &Task, v: u64) -> Result<CasResult, FakeError> {
        (self.cas)(self, task, v)
    }
    async fn load_workflow(&self, id: &str, v: u64) -> Result<Option<Workflow>, FakeError> {
        (self.load_workflow)(self, id, v)
    }
    async fn append_event(&self, id: &str, event: &StoredEvent) -> Result<(), FakeError> {
        (self.append)(self, id, event)
    }
}

impl StoreFixture for M {
    type Store = Self;
    type Error = FakeError;
    fn store(&self) -> &Self {
        self
    }
    fn insert_workflow(&self, workflow: &Workflow) {
        self.store.insert_workflow(workflow.clone());
    }
    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        (self.events)(self, task_id)
    }
    fn reopen(&self) -> Self {
        Self {
            store: (self.reopen)(self),
            log: Mutex::new(Vec::new()),
            ..*self
        }
    }
}

/// Versions toggle 1 -> 2 -> 1 (ABA): every single write looks newer.
fn toggled(version: u64) -> u64 {
    if version % 2 == 1 { 1 } else { 2 }
}

/// What `CounterStore` persists: everything but its version counter.
#[derive(Default)]
pub struct Disk {
    tasks: BTreeMap<String, VersionedTask>,
    workflows: BTreeMap<(String, u64), Workflow>,
    events: BTreeMap<String, Vec<StoredEvent>>,
}

/// Verifier r4 `CounterStore`: persists tasks, versions, workflows and
/// events, but issues versions from a counter in the object, which starts
/// at 0 again after a reopen (versions go backwards: ABA across restarts).
pub struct CounterStore {
    disk: Arc<Mutex<Disk>>,
    counter: AtomicU64,
}

impl CounterStore {
    fn open(disk: Arc<Mutex<Disk>>) -> Self {
        Self {
            disk,
            counter: AtomicU64::new(0),
        }
    }

    fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn disk(&self) -> std::sync::MutexGuard<'_, Disk> {
        self.disk.lock().unwrap()
    }
}

impl Store for CounterStore {
    type Error = String;
    async fn load_task(&self, id: &str) -> Result<Option<VersionedTask>, String> {
        Ok(self.disk().tasks.get(id).cloned())
    }
    async fn create_task(&self, task: &Task) -> Result<(), String> {
        let version = self.next();
        let mut disk = self.disk();
        if disk.tasks.contains_key(&task.id) {
            return Err(format!("{} exists", task.id));
        }
        disk.tasks.insert(
            task.id.clone(),
            VersionedTask {
                version,
                task: task.clone(),
            },
        );
        Ok(())
    }
    async fn compare_and_swap_task(&self, task: &Task, expected: u64) -> Result<CasResult, String> {
        let mut disk = self.disk();
        let Some(current) = disk.tasks.get_mut(&task.id) else {
            return Ok(CasResult::Conflict {
                current_version: None,
            });
        };
        if current.version != expected {
            return Ok(CasResult::Conflict {
                current_version: Some(current.version),
            });
        }
        let version = self.next();
        current.version = version;
        current.task = task.clone();
        Ok(CasResult::Written {
            new_version: version,
        })
    }
    async fn load_workflow(&self, id: &str, version: u64) -> Result<Option<Workflow>, String> {
        Ok(self
            .disk()
            .workflows
            .get(&(id.to_owned(), version))
            .cloned())
    }
    async fn append_event(&self, id: &str, event: &StoredEvent) -> Result<(), String> {
        let mut disk = self.disk();
        if !disk.tasks.contains_key(id) {
            return Err(format!("no task {id}"));
        }
        disk.events
            .entry(id.to_owned())
            .or_default()
            .push(event.clone());
        Ok(())
    }
}

impl StoreFixture for CounterStore {
    type Store = Self;
    type Error = String;
    fn store(&self) -> &Self {
        self
    }
    fn insert_workflow(&self, workflow: &Workflow) {
        self.disk()
            .workflows
            .insert((workflow.id.clone(), workflow.version), workflow.clone());
    }
    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        self.disk().events.get(task_id).cloned().unwrap_or_default()
    }
    fn reopen(&self) -> Self {
        Self::open(Arc::clone(&self.disk))
    }
}

pub fn mutants() -> Vec<Mutant> {
    vec![
        // STO-1: a field is lost on the way back (a column never read).
        Mutant {
            rule: "STO-1",
            name: "DropsDependsOn",
            run: |name| {
                store::run(name, || M {
                    load: |m, id| {
                        Ok(m.real_load(id)?.map(|mut v| {
                            v.task.depends_on.clear();
                            v
                        }))
                    },
                    ..M::new()
                })
            },
        },
        // STO-2: a missing task is an error instead of None.
        Mutant {
            rule: "STO-2",
            name: "MissingTaskIsAnError",
            run: |name| {
                store::run(name, || M {
                    load: |m, id| {
                        m.real_load(id)?.map(Some).ok_or(FakeError {
                            operation: "load_task",
                            message: format!("no row for {id}"),
                        })
                    },
                    ..M::new()
                })
            },
        },
        // STO-3: creating an existing id overwrites it (an upsert).
        Mutant {
            rule: "STO-3",
            name: "CreateOverwrites",
            run: |name| {
                store::run(name, || M {
                    create: |m, t| match m.real_load(&t.id)? {
                        Some(_) => m.force_write(t).map(|_| ()),
                        None => block_on(m.store.create_task(t)),
                    },
                    ..M::new()
                })
            },
        },
        // STO-4: versions toggle 1 -> 2 -> 1 (ABA).
        Mutant {
            rule: "STO-4",
            name: "TogglingVersions",
            run: |name| {
                store::run(name, || M {
                    load: |m, id| {
                        Ok(m.real_load(id)?.map(|v| VersionedTask {
                            version: toggled(v.version),
                            task: v.task,
                        }))
                    },
                    cas: |m, t, expected| {
                        let Some(current) = m.real_load(&t.id)? else {
                            return m.real_cas(t, expected);
                        };
                        if toggled(current.version) != expected {
                            return Ok(CasResult::Conflict {
                                current_version: Some(toggled(current.version)),
                            });
                        }
                        Ok(match m.real_cas(t, current.version)? {
                            CasResult::Written { new_version } => CasResult::Written {
                                new_version: toggled(new_version),
                            },
                            CasResult::Conflict { current_version } => CasResult::Conflict {
                                current_version: current_version.map(toggled),
                            },
                        })
                    },
                    ..M::new()
                })
            },
        },
        // STO-5: the last writer wins, whatever version it read.
        Mutant {
            rule: "STO-5",
            name: "LastWriterWins",
            run: |name| {
                store::run(name, || M {
                    cas: |m, t, _| m.force_write(t),
                    ..M::new()
                })
            },
        },
        // STO-6 (verifier r2 S1): any version >= current is accepted.
        Mutant {
            rule: "STO-6",
            name: "AcceptsFutureVersions",
            run: |name| {
                store::run(name, || M {
                    cas: |m, t, expected| match m.real_load(&t.id)? {
                        Some(current) if expected >= current.version => {
                            m.real_cas(t, current.version)
                        }
                        _ => m.real_cas(t, expected),
                    },
                    ..M::new()
                })
            },
        },
        // STO-7 (verifier r2 S2): a conflict guesses `expected + 1`.
        Mutant {
            rule: "STO-7",
            name: "GuessesCurrentVersion",
            run: |name| {
                store::run(name, || M {
                    cas: |m, t, expected| match m.real_cas(t, expected)? {
                        CasResult::Conflict {
                            current_version: Some(_),
                        } => Ok(CasResult::Conflict {
                            current_version: Some(expected + 1),
                        }),
                        other => Ok(other),
                    },
                    ..M::new()
                })
            },
        },
        // STO-8: CAS on a missing task creates it.
        Mutant {
            rule: "STO-8",
            name: "CasCreatesMissingTask",
            run: |name| {
                store::run(name, || M {
                    cas: |m, t, expected| {
                        if m.real_load(&t.id)?.is_none() {
                            block_on(m.store.create_task(t))?;
                            let v = m.real_load(&t.id)?.map_or(1, |v| v.version);
                            return Ok(CasResult::Written { new_version: v });
                        }
                        m.real_cas(t, expected)
                    },
                    ..M::new()
                })
            },
        },
        // STO-9: a missing workflow version falls back to the one before.
        Mutant {
            rule: "STO-9",
            name: "FallsBackToOlderWorkflow",
            run: |name| {
                store::run(name, || M {
                    load_workflow: |m, id, v| match m.real_load_workflow(id, v)? {
                        Some(w) => Ok(Some(w)),
                        None if v > 0 => m.real_load_workflow(id, v - 1),
                        None => Ok(None),
                    },
                    ..M::new()
                })
            },
        },
        // STO-10: newest event first.
        Mutant {
            rule: "STO-10",
            name: "PrependsEvents",
            run: |name| {
                store::run(name, || M {
                    append: |m, id, e| {
                        m.log.lock().unwrap().insert(0, (id.to_owned(), e.clone()));
                        Ok(())
                    },
                    events: |m, id| m.logged(id),
                    ..M::new()
                })
            },
        },
        // STO-11 (verifier r2 S3): one event log shared by every task.
        Mutant {
            rule: "STO-11",
            name: "SharedEventLog",
            run: |name| {
                store::run(name, || M {
                    append: |m, _, e| {
                        m.log.lock().unwrap().push((String::new(), e.clone()));
                        Ok(())
                    },
                    events: |m, _| m.logged(""),
                    ..M::new()
                })
            },
        },
        // STO-12 (verifier r3 M3): everything lives in memory; a reopened
        // store starts empty.
        Mutant {
            rule: "STO-12",
            name: "InMemoryOnly",
            run: |name| {
                store::run(name, || M {
                    reopen: |_| FakeStore::new(),
                    ..M::new()
                })
            },
        },
        // STO-4 (verifier r4 R4-2): the version counter is not persisted;
        // after a reopen versions start over and a stale writer wins.
        Mutant {
            rule: "STO-4",
            name: "CounterStore",
            run: |name| store::run(name, || CounterStore::open(Arc::default())),
        },
    ]
}
