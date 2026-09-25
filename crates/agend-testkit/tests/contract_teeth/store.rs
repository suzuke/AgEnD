//! Store mutants: a `FakeStore` with one method replaced. Mutants that keep
//! events themselves also replace the fixture's view of them. A mutant's
//! persisted state is the fake's data ([`FakeStoreFile`]) and its replaced
//! methods; `boot` opens the next mutant on it (`open`, replaced by
//! `InMemoryOnly`); a mutant's own event log does not survive it.
//! `CounterStore` (verifier r4), `TruncOnOpen` and `SharedMem` (verifier
//! r5) are standalone.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use agend_core::pipeline::task::Task;
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{CasResult, Store, StoredEvent, VersionedTask};
use agend_testkit::block_on;
use agend_testkit::contract::store::{self, StoreFixture};
use agend_testkit::fakes::{FakeError, FakeStore, FakeStoreFile};

use super::Mutant;

type Load = fn(&M, &str) -> Result<Option<VersionedTask>, FakeError>;
type Create = fn(&M, &Task) -> Result<(), FakeError>;
type Cas = fn(&M, &Task, u64) -> Result<CasResult, FakeError>;
type LoadWorkflow = fn(&M, &str, u64) -> Result<Option<Workflow>, FakeError>;
type Append = fn(&M, &str, &StoredEvent) -> Result<(), FakeError>;
type Events = fn(&M, &str) -> Vec<StoredEvent>;
type Open = fn(&FakeStoreFile) -> FakeStore;

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
    open: Open,
}

/// What an `M` keeps across a restart: the data and its replaced methods.
#[derive(Clone, Copy)]
pub struct Methods {
    load: Load,
    create: Create,
    cas: Cas,
    load_workflow: LoadWorkflow,
    append: Append,
    events: Events,
    open: Open,
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
            open: FakeStore::open,
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
    type Persisted = (FakeStoreFile, Methods);
    fn store(&self) -> &Self {
        self
    }
    fn insert_workflow(&self, workflow: &Workflow) {
        self.store.insert_workflow(workflow.clone());
    }
    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        (self.events)(self, task_id)
    }
    fn persisted(&self) -> Self::Persisted {
        let methods = Methods {
            load: self.load,
            create: self.create,
            cas: self.cas,
            load_workflow: self.load_workflow,
            append: self.append,
            events: self.events,
            open: self.open,
        };
        (self.store.file(), methods)
    }
    fn boot((file, m): &Self::Persisted) -> Self {
        Self {
            store: (m.open)(file),
            log: Mutex::new(Vec::new()),
            load: m.load,
            create: m.create,
            cas: m.cas,
            load_workflow: m.load_workflow,
            append: m.append,
            events: m.events,
            open: m.open,
        }
    }
}

/// Versions toggle 1 -> 2 -> 1 (ABA): every single write looks newer.
fn toggled(version: u64) -> u64 {
    if version % 2 == 1 { 1 } else { 2 }
}

/// What `CounterStore` and `TruncOnOpen` persist: tasks, versions,
/// workflows, events.
#[derive(Default, Clone)]
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
    type Persisted = Arc<Mutex<Disk>>;
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
    fn persisted(&self) -> Arc<Mutex<Disk>> {
        Arc::clone(&self.disk)
    }
    fn boot(disk: &Arc<Mutex<Disk>>) -> Self {
        Self::open(Arc::clone(disk))
    }
}

/// A store that reads the whole file into memory when it opens and writes
/// the whole snapshot back on every mutation. Two ways to get it wrong:
///
/// - verifier r5 `TruncOnOpen` (`truncates`): opening leaves the file
///   empty. A daemon that boots, reads and idles loses everything at its
///   next restart.
/// - `FrozenDatabase` (round 6): only the daemon that created the file
///   writes it; what a restarted daemon writes is gone at the next restart.
pub struct TruncOnOpen {
    file: Arc<Mutex<Option<Disk>>>,
    mem: Mutex<Disk>,
    truncates: bool,
    writes: bool,
}

impl TruncOnOpen {
    fn open(file: Arc<Mutex<Option<Disk>>>, truncates: bool) -> Self {
        let (mem, writes) = {
            let mut stored = file.lock().unwrap();
            let writes = truncates || stored.is_none();
            let mem = if truncates {
                stored.take()
            } else {
                stored.clone()
            };
            (mem.unwrap_or_default(), writes)
        };
        Self {
            file,
            mem: Mutex::new(mem),
            truncates,
            writes,
        }
    }

    fn mem(&self) -> std::sync::MutexGuard<'_, Disk> {
        self.mem.lock().unwrap()
    }

    fn flush(&self) {
        if self.writes {
            *self.file.lock().unwrap() = Some(self.mem().clone());
        }
    }
}

impl Store for TruncOnOpen {
    type Error = String;
    async fn load_task(&self, id: &str) -> Result<Option<VersionedTask>, String> {
        Ok(self.mem().tasks.get(id).cloned())
    }
    async fn create_task(&self, task: &Task) -> Result<(), String> {
        {
            let mut mem = self.mem();
            if mem.tasks.contains_key(&task.id) {
                return Err(format!("{} exists", task.id));
            }
            mem.tasks.insert(
                task.id.clone(),
                VersionedTask {
                    version: 1,
                    task: task.clone(),
                },
            );
        }
        self.flush();
        Ok(())
    }
    async fn compare_and_swap_task(&self, task: &Task, expected: u64) -> Result<CasResult, String> {
        let result = {
            let mut mem = self.mem();
            let Some(current) = mem.tasks.get_mut(&task.id) else {
                return Ok(CasResult::Conflict {
                    current_version: None,
                });
            };
            if current.version != expected {
                return Ok(CasResult::Conflict {
                    current_version: Some(current.version),
                });
            }
            current.version += 1;
            current.task = task.clone();
            CasResult::Written {
                new_version: current.version,
            }
        };
        self.flush();
        Ok(result)
    }
    async fn load_workflow(&self, id: &str, version: u64) -> Result<Option<Workflow>, String> {
        Ok(self.mem().workflows.get(&(id.to_owned(), version)).cloned())
    }
    async fn append_event(&self, id: &str, event: &StoredEvent) -> Result<(), String> {
        {
            let mut mem = self.mem();
            if !mem.tasks.contains_key(id) {
                return Err(format!("no task {id}"));
            }
            mem.events
                .entry(id.to_owned())
                .or_default()
                .push(event.clone());
        }
        self.flush();
        Ok(())
    }
}

impl StoreFixture for TruncOnOpen {
    type Store = Self;
    type Error = String;
    type Persisted = (Arc<Mutex<Option<Disk>>>, bool);
    fn store(&self) -> &Self {
        self
    }
    fn insert_workflow(&self, workflow: &Workflow) {
        self.mem()
            .workflows
            .insert((workflow.id.clone(), workflow.version), workflow.clone());
        self.flush();
    }
    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        self.mem().events.get(task_id).cloned().unwrap_or_default()
    }
    fn persisted(&self) -> Self::Persisted {
        (Arc::clone(&self.file), self.truncates)
    }
    fn boot((file, truncates): &Self::Persisted) -> Self {
        Self::open(Arc::clone(file), *truncates)
    }
}

/// Verifier r5 `SharedMem`: SQLite shared-cache in memory. The data lives
/// while any handle to it is open and is gone when the last one closes; the
/// persisted state is only the database's name.
pub struct SharedMem {
    db: Arc<FakeStore>,
    name: Arc<Mutex<Weak<FakeStore>>>,
}

impl SharedMem {
    fn open(name: Arc<Mutex<Weak<FakeStore>>>) -> Self {
        let db = {
            let mut slot = name.lock().unwrap();
            slot.upgrade().unwrap_or_else(|| {
                let db = Arc::new(FakeStore::new());
                *slot = Arc::downgrade(&db);
                db
            })
        };
        Self { db, name }
    }
}

impl Store for SharedMem {
    type Error = FakeError;
    async fn load_task(&self, id: &str) -> Result<Option<VersionedTask>, FakeError> {
        self.db.load_task(id).await
    }
    async fn create_task(&self, task: &Task) -> Result<(), FakeError> {
        self.db.create_task(task).await
    }
    async fn compare_and_swap_task(&self, task: &Task, v: u64) -> Result<CasResult, FakeError> {
        self.db.compare_and_swap_task(task, v).await
    }
    async fn load_workflow(&self, id: &str, v: u64) -> Result<Option<Workflow>, FakeError> {
        self.db.load_workflow(id, v).await
    }
    async fn append_event(&self, id: &str, event: &StoredEvent) -> Result<(), FakeError> {
        self.db.append_event(id, event).await
    }
}

impl StoreFixture for SharedMem {
    type Store = Self;
    type Error = FakeError;
    type Persisted = Arc<Mutex<Weak<FakeStore>>>;
    fn store(&self) -> &Self {
        self
    }
    fn insert_workflow(&self, workflow: &Workflow) {
        self.db.insert_workflow(workflow.clone());
    }
    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        self.db.events(task_id)
    }
    fn persisted(&self) -> Arc<Mutex<Weak<FakeStore>>> {
        Arc::clone(&self.name)
    }
    fn boot(name: &Arc<Mutex<Weak<FakeStore>>>) -> Self {
        Self::open(Arc::clone(name))
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
                    open: |_| FakeStore::new(),
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
        // STO-12 (verifier r5 R5-2): opening empties the file; data survives
        // only while every boot writes something.
        Mutant {
            rule: "STO-12",
            name: "TruncOnOpen",
            run: |name| store::run(name, || TruncOnOpen::open(Arc::default(), true)),
        },
        // STO-12 (round 6, helper mutation H2): what a restarted daemon
        // writes is never persisted; the last boot reads stale data.
        Mutant {
            rule: "STO-12",
            name: "FrozenDatabase",
            run: |name| store::run(name, || TruncOnOpen::open(Arc::default(), false)),
        },
        // STO-12 (verifier r5 R5-5): shared in-memory database; the data is
        // gone once no handle is open.
        Mutant {
            rule: "STO-12",
            name: "SharedMem",
            run: |name| store::run(name, || SharedMem::open(Arc::default())),
        },
    ]
}
