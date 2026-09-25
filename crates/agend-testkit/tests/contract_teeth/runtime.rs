//! Runtime mutants: a `FakeRuntime` with one method replaced. The fixture's
//! `is_running` always looks at the fake's holder table ([`FakeHolders`]),
//! like a real fixture looking at processes and sockets. A mutant's
//! persisted state is that table plus whatever the mutant itself persists;
//! `boot` builds the next mutant over it with the same replaced methods and
//! an empty memo: what one daemon remembered is gone. `DaemonScoped`
//! (verifier r3), `Handoff` (verifier r4), `RewriteOnChange` and
//! `LastOneOut` (verifier r5) are standalone.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, Weak};

use agend_core::traits::{HolderHandle, HolderLaunch, Runtime};
use agend_testkit::block_on;
use agend_testkit::contract::runtime::{self, RuntimeFixture};
use agend_testkit::fakes::{FakeError, FakeHolders, FakeRuntime};

use super::Mutant;

/// Implements `RuntimeFixture` for a mutant with a `FakeRuntime` field
/// `$rt`: `$save` maps the mutant to its persisted state (the first element
/// is the fake's holder table), `$boot` builds a mutant from it.
macro_rules! runtime_fixture {
    ($ty:ty, $rt:ident, $persisted:ty, |$me:ident| $save:expr, |$p:ident| $boot:expr) => {
        impl RuntimeFixture for $ty {
            type Runtime = Self;
            type Error = FakeError;
            type Persisted = $persisted;
            fn runtime(&self) -> &Self {
                self
            }
            fn launch(&self, id: &str) -> HolderLaunch {
                RuntimeFixture::launch(&self.$rt, id)
            }
            fn is_running(persisted: &$persisted, handle: &HolderHandle) -> bool {
                FakeRuntime::is_running(&persisted.0, handle)
            }
            fn persisted(&self) -> $persisted {
                let $me = self;
                $save
            }
            fn boot($p: &$persisted) -> Self {
                $boot
            }
        }
    };
}

type Start = fn(&M, &HolderLaunch) -> Result<HolderHandle, FakeError>;
type Stop = fn(&M, &str) -> Result<(), FakeError>;
type Recover = fn(&M) -> Result<Vec<HolderHandle>, FakeError>;

/// A fake runtime with `start_holder`, `stop_holder` or `recover_holders`
/// replaced.
pub struct M {
    rt: FakeRuntime,
    memo: Mutex<BTreeMap<String, HolderHandle>>,
    start: Start,
    stop: Stop,
    recover: Recover,
}

impl M {
    fn new() -> Self {
        Self {
            rt: FakeRuntime::new(),
            memo: Mutex::new(BTreeMap::new()),
            start: |m, l| m.real_start(l),
            stop: |m, id| m.real_stop(id),
            recover: |m| m.real_recover(),
        }
    }

    /// Start that also remembers the handle in this daemon's memo.
    fn start_and_remember(&self, launch: &HolderLaunch) -> Result<HolderHandle, FakeError> {
        let handle = self.real_start(launch)?;
        self.remember(&handle);
        Ok(handle)
    }

    fn real_start(&self, launch: &HolderLaunch) -> Result<HolderHandle, FakeError> {
        block_on(self.rt.start_holder(launch))
    }

    fn real_stop(&self, id: &str) -> Result<(), FakeError> {
        block_on(self.rt.stop_holder(id))
    }

    fn real_recover(&self) -> Result<Vec<HolderHandle>, FakeError> {
        block_on(self.rt.recover_holders())
    }

    fn remembered(&self, id: &str) -> Option<HolderHandle> {
        self.memo.lock().unwrap().get(id).cloned()
    }

    fn remember(&self, handle: &HolderHandle) {
        self.memo
            .lock()
            .unwrap()
            .insert(handle.instance_id.clone(), handle.clone());
    }
}

impl Runtime for M {
    type Error = FakeError;
    async fn start_holder(&self, launch: &HolderLaunch) -> Result<HolderHandle, FakeError> {
        (self.start)(self, launch)
    }
    async fn stop_holder(&self, id: &str) -> Result<(), FakeError> {
        (self.stop)(self, id)
    }
    async fn recover_holders(&self) -> Result<Vec<HolderHandle>, FakeError> {
        (self.recover)(self)
    }
}

runtime_fixture!(
    M,
    rt,
    (FakeHolders, Start, Stop, Recover),
    |m| (m.rt.holders(), m.start, m.stop, m.recover),
    |p| Self {
        rt: FakeRuntime::on(&p.0),
        memo: Mutex::new(BTreeMap::new()),
        start: p.1,
        stop: p.2,
        recover: p.3,
    }
);

/// Verifier r3 `DaemonScoped`: a runtime scoped to one daemon process. It
/// recovers only the holders it started itself and kills them when it is
/// dropped (the daemon exits), which is v1's behaviour (V1-LESSONS #7).
pub struct DaemonScoped {
    world: FakeRuntime,
    mine: Mutex<BTreeSet<String>>,
}

impl DaemonScoped {
    fn new(world: FakeRuntime) -> Self {
        Self {
            world,
            mine: Mutex::new(BTreeSet::new()),
        }
    }
}

impl Drop for DaemonScoped {
    fn drop(&mut self) {
        for id in self.mine.lock().unwrap().iter() {
            let _ = block_on(self.world.stop_holder(id));
        }
    }
}

impl Runtime for DaemonScoped {
    type Error = FakeError;
    async fn start_holder(&self, l: &HolderLaunch) -> Result<HolderHandle, FakeError> {
        let h = self.world.start_holder(l).await?;
        self.mine.lock().unwrap().insert(l.instance_id.clone());
        Ok(h)
    }
    async fn stop_holder(&self, id: &str) -> Result<(), FakeError> {
        self.world.stop_holder(id).await?;
        self.mine.lock().unwrap().remove(id);
        Ok(())
    }
    async fn recover_holders(&self) -> Result<Vec<HolderHandle>, FakeError> {
        let mine = self.mine.lock().unwrap().clone();
        Ok(self
            .world
            .recover_holders()
            .await?
            .into_iter()
            .filter(|h| mine.contains(&h.instance_id))
            .collect())
    }
}

runtime_fixture!(
    DaemonScoped,
    world,
    (FakeHolders,),
    |m| (m.world.holders(),),
    |p| Self::new(FakeRuntime::on(&p.0))
);

/// Verifier r4 `Handoff`: persists a registry of started holders, but
/// recovery reads and deletes it (like consuming a handoff file) and keeps
/// the ids only in the runtime object. The first restart recovers them; the
/// one after that orphans them.
pub struct Handoff {
    world: FakeRuntime,
    registry: Arc<Mutex<BTreeSet<String>>>,
    mine: Mutex<BTreeSet<String>>,
}

impl Handoff {
    fn over(world: FakeRuntime, registry: Arc<Mutex<BTreeSet<String>>>) -> Self {
        Self {
            world,
            registry,
            mine: Mutex::new(BTreeSet::new()),
        }
    }
}

impl Runtime for Handoff {
    type Error = FakeError;
    async fn start_holder(&self, l: &HolderLaunch) -> Result<HolderHandle, FakeError> {
        let h = self.world.start_holder(l).await?;
        self.registry.lock().unwrap().insert(l.instance_id.clone());
        Ok(h)
    }
    async fn stop_holder(&self, id: &str) -> Result<(), FakeError> {
        self.world.stop_holder(id).await?;
        self.registry.lock().unwrap().remove(id);
        self.mine.lock().unwrap().remove(id);
        Ok(())
    }
    async fn recover_holders(&self) -> Result<Vec<HolderHandle>, FakeError> {
        let taken = std::mem::take(&mut *self.registry.lock().unwrap());
        let mine = {
            let mut mine = self.mine.lock().unwrap();
            mine.extend(taken);
            mine.clone()
        };
        let registry = self.registry.lock().unwrap().clone();
        Ok(self
            .world
            .recover_holders()
            .await?
            .into_iter()
            .filter(|h| mine.contains(&h.instance_id) || registry.contains(&h.instance_id))
            .collect())
    }
}

runtime_fixture!(
    Handoff,
    world,
    (FakeHolders, Arc<Mutex<BTreeSet<String>>>),
    |m| (m.world.holders(), Arc::clone(&m.registry)),
    |p| Self::over(FakeRuntime::on(&p.0), Arc::clone(&p.1))
);

/// A runtime that reads the persisted holder registry into memory when it
/// opens and writes it back on start and stop. Two ways to get it wrong:
///
/// - verifier r5 `RewriteOnChange` (`truncates`): opening truncates the
///   registry (like `File::create` after the read). A daemon that boots,
///   recovers and idles leaves an empty registry, so the next one orphans
///   the holders.
/// - `FrozenRegistry` (round 6): only the daemon that found no registry
///   writes one; a restarted daemon's starts are never recorded, so the
///   holders it started are orphaned at the next restart.
pub struct RewriteOnChange {
    world: FakeRuntime,
    file: Arc<Mutex<BTreeSet<String>>>,
    mine: Mutex<BTreeSet<String>>,
    truncates: bool,
    writes: bool,
}

impl RewriteOnChange {
    fn open(world: FakeRuntime, file: Arc<Mutex<BTreeSet<String>>>, truncates: bool) -> Self {
        let (mine, writes) = {
            let mut registry = file.lock().unwrap();
            let writes = truncates || registry.is_empty();
            let mine = if truncates {
                std::mem::take(&mut *registry)
            } else {
                registry.clone()
            };
            (mine, writes)
        };
        Self {
            world,
            file,
            mine: Mutex::new(mine),
            truncates,
            writes,
        }
    }

    fn flush(&self) {
        if self.writes {
            *self.file.lock().unwrap() = self.mine.lock().unwrap().clone();
        }
    }
}

impl Runtime for RewriteOnChange {
    type Error = FakeError;
    async fn start_holder(&self, l: &HolderLaunch) -> Result<HolderHandle, FakeError> {
        let h = self.world.start_holder(l).await?;
        self.mine.lock().unwrap().insert(l.instance_id.clone());
        self.flush();
        Ok(h)
    }
    async fn stop_holder(&self, id: &str) -> Result<(), FakeError> {
        self.world.stop_holder(id).await?;
        self.mine.lock().unwrap().remove(id);
        self.flush();
        Ok(())
    }
    async fn recover_holders(&self) -> Result<Vec<HolderHandle>, FakeError> {
        let mine = self.mine.lock().unwrap().clone();
        Ok(self
            .world
            .recover_holders()
            .await?
            .into_iter()
            .filter(|h| mine.contains(&h.instance_id))
            .collect())
    }
}

runtime_fixture!(
    RewriteOnChange,
    world,
    (FakeHolders, Arc<Mutex<BTreeSet<String>>>, bool),
    |m| (m.world.holders(), Arc::clone(&m.file), m.truncates),
    |p| Self::open(FakeRuntime::on(&p.0), Arc::clone(&p.1), p.2)
);

/// The daemon process of `LastOneOut`: stops every holder when the last
/// runtime of the lineage goes.
pub struct Process {
    machine: FakeRuntime,
}

impl Drop for Process {
    fn drop(&mut self) {
        for h in self.machine.running() {
            let _ = block_on(self.machine.stop_holder(&h.instance_id));
        }
    }
}

/// Verifier r5 `LastOneOut`: holders outlive one runtime but die when the
/// last runtime alive goes, as if they were children of the daemon process.
/// In one test process this is modelled with a shared `Weak`; the
/// real-process version of the same bug is what gate 6 checks.
pub struct LastOneOut {
    rt: FakeRuntime,
    _process: Arc<Process>,
    slot: Arc<Mutex<Weak<Process>>>,
}

impl LastOneOut {
    fn open(rt: FakeRuntime, slot: &Arc<Mutex<Weak<Process>>>) -> Self {
        let process = {
            let mut slot = slot.lock().unwrap();
            slot.upgrade().unwrap_or_else(|| {
                let process = Arc::new(Process {
                    machine: FakeRuntime::on(&rt.holders()),
                });
                *slot = Arc::downgrade(&process);
                process
            })
        };
        Self {
            rt,
            _process: process,
            slot: Arc::clone(slot),
        }
    }
}

impl Runtime for LastOneOut {
    type Error = FakeError;
    async fn start_holder(&self, l: &HolderLaunch) -> Result<HolderHandle, FakeError> {
        self.rt.start_holder(l).await
    }
    async fn stop_holder(&self, id: &str) -> Result<(), FakeError> {
        self.rt.stop_holder(id).await
    }
    async fn recover_holders(&self) -> Result<Vec<HolderHandle>, FakeError> {
        self.rt.recover_holders().await
    }
}

runtime_fixture!(
    LastOneOut,
    rt,
    (FakeHolders, Arc<Mutex<Weak<Process>>>),
    |m| (m.rt.holders(), Arc::clone(&m.slot)),
    |p| Self::open(FakeRuntime::on(&p.0), &p.1)
);

pub fn mutants() -> Vec<Mutant> {
    vec![
        // RTM-1: the handle is keyed by the executable, not the instance.
        Mutant {
            rule: "RTM-1",
            name: "HandleNamesTheExecutable",
            run: |name| {
                runtime::run(name, || M {
                    start: |m, l| {
                        let mut handle = m.real_start(l)?;
                        handle.instance_id = l.executable.clone();
                        Ok(handle)
                    },
                    ..M::new()
                })
            },
        },
        // RTM-2: start answers with a handle but starts nothing.
        Mutant {
            rule: "RTM-2",
            name: "StartsNothing",
            run: |name| {
                runtime::run(name, || M {
                    start: |m, l| {
                        let handle = HolderHandle {
                            instance_id: l.instance_id.clone(),
                            process_id: Some(1),
                            socket_path: format!("/fake/agend/run/holders/{}.sock", l.instance_id),
                        };
                        m.remember(&handle);
                        Ok(handle)
                    },
                    stop: |m, id| {
                        m.memo.lock().unwrap().remove(id);
                        Ok(())
                    },
                    recover: |m| Ok(m.memo.lock().unwrap().values().cloned().collect()),
                    ..M::new()
                })
            },
        },
        // RTM-3: recovery forgets every holder.
        Mutant {
            rule: "RTM-3",
            name: "RecoversNothing",
            run: |name| {
                runtime::run(name, || M {
                    recover: |_| Ok(Vec::new()),
                    ..M::new()
                })
            },
        },
        // RTM-4 (verifier r2 R1): recovered handles with a wrong socket, no pid.
        Mutant {
            rule: "RTM-4",
            name: "RecoversWrongHandles",
            run: |name| {
                runtime::run(name, || M {
                    recover: |m| {
                        Ok(m.real_recover()?
                            .into_iter()
                            .map(|h| HolderHandle {
                                instance_id: h.instance_id,
                                process_id: None,
                                socket_path: "/nonexistent".into(),
                            })
                            .collect())
                    },
                    ..M::new()
                })
            },
        },
        // RTM-5 (verifier r2 R2): stop only hides the holder; it keeps running.
        Mutant {
            rule: "RTM-5",
            name: "StopOnlyHides",
            run: |name| {
                runtime::run(name, || M {
                    start: |m, l| match m.memo.lock().unwrap().remove(&l.instance_id) {
                        Some(hidden) => Ok(hidden),
                        None => m.real_start(l),
                    },
                    stop: |m, id| {
                        let handle = m
                            .real_recover()?
                            .into_iter()
                            .find(|h| h.instance_id == id)
                            .ok_or(FakeError {
                                operation: "stop_holder",
                                message: format!("no holder {id}"),
                            })?;
                        m.remember(&handle);
                        Ok(())
                    },
                    recover: |m| {
                        let hidden = m.memo.lock().unwrap().clone();
                        Ok(m.real_recover()?
                            .into_iter()
                            .filter(|h| !hidden.contains_key(&h.instance_id))
                            .collect())
                    },
                    ..M::new()
                })
            },
        },
        // RTM-6: recovery lists every holder ever started.
        Mutant {
            rule: "RTM-6",
            name: "RecoversStoppedHolders",
            run: |name| {
                runtime::run(name, || M {
                    start: M::start_and_remember,
                    recover: |m| Ok(m.memo.lock().unwrap().values().cloned().collect()),
                    ..M::new()
                })
            },
        },
        // RTM-7: a stopped instance id can never be started again.
        Mutant {
            rule: "RTM-7",
            name: "RefusesRestart",
            run: |name| {
                runtime::run(name, || M {
                    start: |m, l| match m.remembered(&l.instance_id) {
                        Some(_) => Err(FakeError {
                            operation: "start_holder",
                            message: format!("{} was stopped", l.instance_id),
                        }),
                        None => m.real_start(l),
                    },
                    stop: |m, id| {
                        if let Some(h) = m.real_recover()?.into_iter().find(|h| h.instance_id == id)
                        {
                            m.remember(&h);
                        }
                        m.real_stop(id)
                    },
                    ..M::new()
                })
            },
        },
        // RTM-8 (verifier r3 M1): holders die with the daemon that started
        // them, and a new daemon only knows its own.
        Mutant {
            rule: "RTM-8",
            name: "DaemonScoped",
            run: |name| runtime::run(name, || DaemonScoped::new(FakeRuntime::new())),
        },
        // RTM-8 (verifier r3 M1, without the kill): holders outlive the
        // daemon but a new one never reconnects them (orphans).
        Mutant {
            rule: "RTM-8",
            name: "RecoversOnlyOwnHolders",
            run: |name| {
                runtime::run(name, || M {
                    start: M::start_and_remember,
                    recover: |m| {
                        let mine = m.memo.lock().unwrap().clone();
                        Ok(m.real_recover()?
                            .into_iter()
                            .filter(|h| mine.contains_key(&h.instance_id))
                            .collect())
                    },
                    ..M::new()
                })
            },
        },
        // RTM-9: a restarted daemon sees the holders but cannot stop the
        // ones an earlier daemon started.
        Mutant {
            rule: "RTM-9",
            name: "StopsOnlyOwnHolders",
            run: |name| {
                runtime::run(name, || M {
                    start: M::start_and_remember,
                    stop: |m, id| match m.memo.lock().unwrap().remove(id) {
                        Some(_) => m.real_stop(id),
                        None => Err(FakeError {
                            operation: "stop_holder",
                            message: format!("{id} was not started by this daemon"),
                        }),
                    },
                    ..M::new()
                })
            },
        },
        // RTM-8 (verifier r4 R4-1): recovery consumes the persisted
        // registry; a daemon restarted twice orphans the holders.
        Mutant {
            rule: "RTM-8",
            name: "Handoff",
            run: |name| runtime::run(name, || Handoff::over(FakeRuntime::new(), Arc::default())),
        },
        // RTM-8 (verifier r5 R5-1): opening truncates the persisted
        // registry; after an idle boot the holders are orphaned.
        Mutant {
            rule: "RTM-8",
            name: "RewriteOnChange",
            run: |name| {
                runtime::run(name, || {
                    RewriteOnChange::open(FakeRuntime::new(), Arc::default(), true)
                })
            },
        },
        // RTM-8 (round 6, helper mutation H2): a restarted daemon never
        // records the holders it starts; the last boot orphans them.
        Mutant {
            rule: "RTM-8",
            name: "FrozenRegistry",
            run: |name| {
                runtime::run(name, || {
                    RewriteOnChange::open(FakeRuntime::new(), Arc::default(), false)
                })
            },
        },
        // RTM-8 (verifier r5 R5-7): holders die when the last runtime goes.
        Mutant {
            rule: "RTM-8",
            name: "LastOneOut",
            run: |name| {
                runtime::run(name, || {
                    LastOneOut::open(FakeRuntime::new(), &Arc::default())
                })
            },
        },
    ]
}
