//! Runtime mutants: a `FakeRuntime` with one method replaced. The fixture's
//! `is_running` always looks at the fake's own table, like a real fixture
//! looking at processes and sockets.

use std::collections::BTreeMap;
use std::sync::Mutex;

use agend_core::traits::{HolderHandle, HolderLaunch, Runtime};
use agend_testkit::block_on;
use agend_testkit::contract::runtime::{self, RuntimeFixture};
use agend_testkit::fakes::{FakeError, FakeRuntime};

use super::Mutant;

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

impl RuntimeFixture for M {
    type Runtime = Self;
    type Error = FakeError;
    fn runtime(&self) -> &Self {
        self
    }
    fn launch(&self, id: &str) -> HolderLaunch {
        RuntimeFixture::launch(&self.rt, id)
    }
    fn is_running(&self, handle: &HolderHandle) -> bool {
        RuntimeFixture::is_running(&self.rt, handle)
    }
}

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
                    start: |m, l| {
                        let handle = m.real_start(l)?;
                        m.remember(&handle);
                        Ok(handle)
                    },
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
    ]
}
