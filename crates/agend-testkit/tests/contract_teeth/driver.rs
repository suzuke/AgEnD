//! Driver mutants: a `FakeDriver` with one method replaced. Each mutant's
//! persisted state is the fake backend ([`FakeBackend`]) plus whatever the
//! mutant itself persists; `boot` builds the next mutant over it (with the
//! same replaced methods). `ObjDedup`, `Gap` (verifier r4) and `LastIdDedup`,
//! `ReadAck`, `LiveJournal` (verifier r5) are standalone.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agend_core::model::DeliveryState;
use agend_core::policy::busy::BusyLevel;
use agend_core::traits::{AgentMessage, DeliveryReceipt, Driver, DriverEvent, DriverEventKind};
use agend_testkit::block_on;
use agend_testkit::contract::driver::{self, DriverFixture};
use agend_testkit::contract::fakes::FakeDriverFixture;
use agend_testkit::fakes::{FakeBackend, FakeError};

use super::Mutant;

type Deliver = fn(&M, &str, &AgentMessage, BusyLevel) -> Result<DeliveryReceipt, FakeError>;
type Events = fn(&M, &str, Option<&str>) -> Result<Vec<DriverEvent>, FakeError>;

const INSTANCE: &str = FakeDriverFixture::INSTANCE;

/// The fake completes a turn inside `deliver`; no need to wait long.
const TURN_TIMEOUT: Duration = Duration::from_millis(200);

fn event_count(fx: &FakeDriverFixture) -> usize {
    block_on(fx.driver.events(INSTANCE, None)).map_or(0, |e| e.len())
}

/// Implements `DriverFixture` for a mutant with a `fx: FakeDriverFixture`
/// field: `$persisted` maps the mutant to its persisted state (the first
/// element is the fake backend), `$boot` builds a mutant from it.
macro_rules! driver_fixture {
    ($ty:ty, $persisted:ty, |$me:ident| $save:expr, |$p:ident| $boot:expr) => {
        impl DriverFixture for $ty {
            type Driver = Self;
            type Error = FakeError;
            type Persisted = $persisted;
            fn driver(&self) -> &Self {
                self
            }
            fn instance_id(&self) -> &str {
                INSTANCE
            }
            fn turn_timeout(&self) -> Duration {
                TURN_TIMEOUT
            }
            fn persisted(&self) -> $persisted {
                let $me = self;
                $save
            }
            fn boot($p: &$persisted) -> Self {
                $boot
            }
            fn emit_while_down(persisted: &$persisted) {
                FakeDriverFixture::emit_while_down(&persisted.0);
            }
        }
    };
}

/// A fake driver whose `deliver` or `events` is replaced.
pub struct M {
    fx: FakeDriverFixture,
    seen: Mutex<BTreeSet<String>>,
    /// How many events the backend had when this driver was created.
    born: usize,
    deliver: Deliver,
    events: Events,
}

impl M {
    fn new() -> Self {
        Self {
            fx: FakeDriverFixture::new(),
            seen: Mutex::new(BTreeSet::new()),
            born: 0,
            deliver: |m, id, msg, mode| m.real_deliver(id, msg, mode),
            events: |m, id, after| m.real_events(id, after),
        }
    }

    fn deliver(self, deliver: Deliver) -> Self {
        Self { deliver, ..self }
    }

    fn events(self, events: Events) -> Self {
        Self { events, ..self }
    }

    fn real_deliver(
        &self,
        id: &str,
        msg: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, FakeError> {
        block_on(self.fx.driver.deliver(id, msg, mode))
    }

    fn real_events(&self, id: &str, after: Option<&str>) -> Result<Vec<DriverEvent>, FakeError> {
        block_on(self.fx.driver.events(id, after))
    }
}

impl Driver for M {
    type Error = FakeError;
    async fn deliver(
        &self,
        id: &str,
        msg: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, FakeError> {
        (self.deliver)(self, id, msg, mode)
    }
    async fn events(&self, id: &str, after: Option<&str>) -> Result<Vec<DriverEvent>, FakeError> {
        (self.events)(self, id, after)
    }
}

driver_fixture!(
    M,
    (FakeBackend, Deliver, Events),
    |m| (m.fx.persisted(), m.deliver, m.events),
    |p| {
        let fx = FakeDriverFixture::boot(&p.0);
        Self {
            born: event_count(&fx),
            fx,
            seen: Mutex::new(BTreeSet::new()),
            deliver: p.1,
            events: p.2,
        }
    }
);

/// Verifier r4 `ObjDedup`: deduplicates message ids in the driver object,
/// and gives the backend an id unique to this object, so a daemon restart
/// forgets which ids it has seen.
pub struct ObjDedup {
    fx: FakeDriverFixture,
    seen: Mutex<BTreeSet<String>>,
    generation: u64,
}

impl ObjDedup {
    fn over(fx: FakeDriverFixture) -> Self {
        static GENERATION: AtomicU64 = AtomicU64::new(0);
        Self {
            fx,
            seen: Mutex::new(BTreeSet::new()),
            generation: GENERATION.fetch_add(1, Ordering::SeqCst),
        }
    }
}

impl Driver for ObjDedup {
    type Error = FakeError;
    async fn deliver(
        &self,
        id: &str,
        msg: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, FakeError> {
        if !self.seen.lock().unwrap().insert(msg.id.clone()) {
            return Ok(sent());
        }
        let mut unique = msg.clone();
        unique.id = format!("{}@{}", msg.id, self.generation);
        self.fx.driver.deliver(id, &unique, mode).await
    }
    async fn events(&self, id: &str, after: Option<&str>) -> Result<Vec<DriverEvent>, FakeError> {
        self.fx.driver.events(id, after).await
    }
}

driver_fixture!(ObjDedup, (FakeBackend,), |m| (m.fx.persisted(),), |p| {
    Self::over(FakeDriverFixture::boot(&p.0))
});

fn sent() -> DeliveryReceipt {
    DeliveryReceipt {
        backend_message_id: None,
        state: DeliveryState::Sent,
    }
}

/// The events of `all` from index `start` on, without the indexes in `gaps`.
fn without(all: Vec<DriverEvent>, start: usize, gaps: &[(usize, usize)]) -> Vec<DriverEvent> {
    all.into_iter()
        .enumerate()
        .skip(start)
        .filter(|(k, _)| !gaps.iter().any(|(from, to)| (*from..*to).contains(k)))
        .map(|(_, e)| e)
        .collect()
}

/// Where a backfill from `after` starts in `all`; `None` for an unknown
/// cursor (left to the fake to reject).
fn start_of(all: &[DriverEvent], after: Option<&str>) -> Option<usize> {
    match after {
        None => Some(0),
        Some(cursor) => all.iter().position(|e| e.cursor == cursor).map(|i| i + 1),
    }
}

/// Verifier r4 `Gap`: serves the backend's journal up to where the old
/// driver was dropped, plus live events from when the new driver was born;
/// what the agent emitted in between (the downtime) is lost.
pub struct Gap {
    fx: FakeDriverFixture,
    /// Event count when the last driver of this lineage was dropped.
    dropped_at: Arc<Mutex<Option<usize>>>,
    /// Indexes of events this driver never serves.
    gap: (usize, usize),
}

impl Gap {
    fn fresh(fx: FakeDriverFixture) -> Self {
        let n = event_count(&fx);
        Self {
            fx,
            dropped_at: Arc::default(),
            gap: (n, n),
        }
    }
}

impl Drop for Gap {
    fn drop(&mut self) {
        *self.dropped_at.lock().unwrap() = Some(event_count(&self.fx));
    }
}

impl Driver for Gap {
    type Error = FakeError;
    async fn deliver(
        &self,
        id: &str,
        msg: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, FakeError> {
        self.fx.driver.deliver(id, msg, mode).await
    }
    async fn events(&self, id: &str, after: Option<&str>) -> Result<Vec<DriverEvent>, FakeError> {
        let all = self.fx.driver.events(id, None).await?;
        let Some(start) = start_of(&all, after) else {
            return self.fx.driver.events(id, after).await;
        };
        Ok(without(all, start, &[self.gap]))
    }
}

driver_fixture!(
    Gap,
    (FakeBackend, Arc<Mutex<Option<usize>>>),
    |m| (m.fx.persisted(), Arc::clone(&m.dropped_at)),
    |p| {
        let fx = FakeDriverFixture::boot(&p.0);
        let born = event_count(&fx);
        let from = p.1.lock().unwrap().take().unwrap_or(born);
        Self {
            fx,
            dropped_at: Arc::clone(&p.1),
            gap: (from.min(born), born),
        }
    }
);

/// Verifier r5 `LastIdDedup`: dedup is persisted and survives restarts, but
/// it remembers only the last message id. After a crash every unconfirmed
/// message is resent, not only the last one.
pub struct LastIdDedup {
    fx: FakeDriverFixture,
    /// The persisted dedup record: the last id and a delivery counter.
    last: Arc<Mutex<(Option<String>, u64)>>,
}

impl Driver for LastIdDedup {
    type Error = FakeError;
    async fn deliver(
        &self,
        id: &str,
        msg: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, FakeError> {
        let n = {
            let mut last = self.last.lock().unwrap();
            if last.0.as_deref() == Some(msg.id.as_str()) {
                return Ok(sent());
            }
            last.0 = Some(msg.id.clone());
            last.1 += 1;
            last.1
        };
        let mut unique = msg.clone();
        unique.id = format!("{}#{n}", msg.id);
        self.fx.driver.deliver(id, &unique, mode).await
    }
    async fn events(&self, id: &str, after: Option<&str>) -> Result<Vec<DriverEvent>, FakeError> {
        self.fx.driver.events(id, after).await
    }
}

driver_fixture!(
    LastIdDedup,
    (FakeBackend, Arc<Mutex<(Option<String>, u64)>>),
    |m| (m.fx.persisted(), Arc::clone(&m.last)),
    |p| Self {
        fx: FakeDriverFixture::boot(&p.0),
        last: Arc::clone(&p.1),
    }
);

/// Verifier r5 `ReadAck`: the backend acks on read. A restarted driver
/// serves only the events after the highest index any earlier driver read,
/// whatever cursor it is given, so backfill from an older cursor (a daemon
/// that crashed before it saved its newest cursor) loses events.
pub struct ReadAck {
    fx: FakeDriverFixture,
    /// The persisted ack: how many events some driver has read.
    acked: Arc<Mutex<usize>>,
    floor: usize,
}

impl Driver for ReadAck {
    type Error = FakeError;
    async fn deliver(
        &self,
        id: &str,
        msg: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, FakeError> {
        self.fx.driver.deliver(id, msg, mode).await
    }
    async fn events(&self, id: &str, after: Option<&str>) -> Result<Vec<DriverEvent>, FakeError> {
        let all = self.fx.driver.events(id, None).await?;
        let Some(mut start) = start_of(&all, after) else {
            return self.fx.driver.events(id, after).await;
        };
        if after.is_some() {
            start = start.max(self.floor).min(all.len());
        }
        let mut acked = self.acked.lock().unwrap();
        *acked = (*acked).max(all.len());
        Ok(all[start..].to_vec())
    }
}

driver_fixture!(
    ReadAck,
    (FakeBackend, Arc<Mutex<usize>>),
    |m| (m.fx.persisted(), Arc::clone(&m.acked)),
    |p| Self {
        fx: FakeDriverFixture::boot(&p.0),
        acked: Arc::clone(&p.1),
        floor: *p.1.lock().unwrap(),
    }
);

/// What `LiveJournal` drivers share: how many are alive, and the event
/// ranges emitted while none was.
#[derive(Default)]
pub struct Lineage {
    alive: usize,
    dead_from: Option<usize>,
    gaps: Vec<(usize, usize)>,
}

/// Verifier r5 `LiveJournal`: keeps only the events emitted while at least
/// one driver is alive; what the agent emitted while no driver ran (a real
/// downtime) is never backfilled.
pub struct LiveJournal {
    fx: FakeDriverFixture,
    lineage: Arc<Mutex<Lineage>>,
}

impl LiveJournal {
    fn attach(fx: FakeDriverFixture, lineage: Arc<Mutex<Lineage>>) -> Self {
        {
            let n = event_count(&fx);
            let mut l = lineage.lock().unwrap();
            if l.alive == 0
                && let Some(from) = l.dead_from.take()
            {
                l.gaps.push((from, n));
            }
            l.alive += 1;
        }
        Self { fx, lineage }
    }
}

impl Drop for LiveJournal {
    fn drop(&mut self) {
        let n = event_count(&self.fx);
        let mut l = self.lineage.lock().unwrap();
        l.alive -= 1;
        if l.alive == 0 {
            l.dead_from = Some(n);
        }
    }
}

impl Driver for LiveJournal {
    type Error = FakeError;
    async fn deliver(
        &self,
        id: &str,
        msg: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, FakeError> {
        self.fx.driver.deliver(id, msg, mode).await
    }
    async fn events(&self, id: &str, after: Option<&str>) -> Result<Vec<DriverEvent>, FakeError> {
        let all = self.fx.driver.events(id, None).await?;
        let Some(start) = start_of(&all, after) else {
            return self.fx.driver.events(id, after).await;
        };
        let gaps = self.lineage.lock().unwrap().gaps.clone();
        Ok(without(all, start, &gaps))
    }
}

driver_fixture!(
    LiveJournal,
    (FakeBackend, Arc<Mutex<Lineage>>),
    |m| (m.fx.persisted(), Arc::clone(&m.lineage)),
    |p| Self::attach(FakeDriverFixture::boot(&p.0), Arc::clone(&p.1))
);

pub fn mutants() -> Vec<Mutant> {
    vec![
        // DRV-1: an idle delivery left in the queue.
        Mutant {
            rule: "DRV-1",
            name: "QueuedReceipt",
            run: |name| {
                driver::run(name, || {
                    M::new().deliver(|m, id, msg, mode| {
                        let mut receipt = m.real_deliver(id, msg, mode)?;
                        receipt.state = DeliveryState::Queued;
                        Ok(receipt)
                    })
                })
            },
        },
        // DRV-2: an unknown instance falls back to the default one.
        Mutant {
            rule: "DRV-2",
            name: "DeliversUnknownToDefault",
            run: |name| {
                driver::run(name, || {
                    M::new().deliver(|m, _, msg, mode| m.real_deliver(INSTANCE, msg, mode))
                })
            },
        },
        // DRV-3: the turn-completed event never surfaces.
        Mutant {
            rule: "DRV-3",
            name: "HidesTurnCompleted",
            run: |name| {
                driver::run(name, || {
                    M::new().events(|m, id, after| {
                        let mut events = m.real_events(id, after)?;
                        events.retain(|e| !matches!(e.kind, DriverEventKind::TurnCompleted { .. }));
                        Ok(events)
                    })
                })
            },
        },
        // DRV-4: every event carries the same cursor.
        Mutant {
            rule: "DRV-4",
            name: "ConstantCursor",
            run: |name| {
                driver::run(name, || {
                    M::new().events(|m, id, after| {
                        let mut events = m.real_events(id, after)?;
                        for e in &mut events {
                            e.cursor = "cursor".into();
                        }
                        Ok(events)
                    })
                })
            },
        },
        // DRV-5: the cursor is ignored, everything is replayed.
        Mutant {
            rule: "DRV-5",
            name: "IgnoresCursor",
            run: |name| driver::run(name, || M::new().events(|m, id, _| m.real_events(id, None))),
        },
        // DRV-5: off by one, the cursor's own event comes back.
        Mutant {
            rule: "DRV-5",
            name: "IncludesTheCursorEvent",
            run: |name| {
                driver::run(name, || {
                    M::new().events(|m, id, after| {
                        let all = m.real_events(id, None)?;
                        let start = after
                            .and_then(|c| all.iter().position(|e| e.cursor == c))
                            .unwrap_or(0);
                        Ok(all[start..].to_vec())
                    })
                })
            },
        },
        // DRV-6 (verifier r2 D1): backfill drops the first newer event.
        Mutant {
            rule: "DRV-6",
            name: "SkipsFirstAfterCursor",
            run: |name| {
                driver::run(name, || {
                    M::new().events(|m, id, after| {
                        let mut events = m.real_events(id, after)?;
                        if after.is_some() && !events.is_empty() {
                            events.remove(0);
                        }
                        Ok(events)
                    })
                })
            },
        },
        // DRV-6 (verifier r2 D2): backfill returns only the last two events.
        Mutant {
            rule: "DRV-6",
            name: "OnlyLastTwoAfterCursor",
            run: |name| {
                driver::run(name, || {
                    M::new().events(|m, id, after| {
                        let events = m.real_events(id, after)?;
                        let keep = if after.is_some() { 2 } else { events.len() };
                        Ok(events[events.len().saturating_sub(keep)..].to_vec())
                    })
                })
            },
        },
        // DRV-7: reading from a cursor consumes it; a second read is empty.
        Mutant {
            rule: "DRV-7",
            name: "ConsumesOnRead",
            run: |name| {
                driver::run(name, || {
                    M::new().events(|m, id, after| {
                        if let Some(cursor) = after
                            && !m.seen.lock().unwrap().insert(cursor.to_owned())
                        {
                            return Ok(Vec::new());
                        }
                        m.real_events(id, after)
                    })
                })
            },
        },
        // DRV-8: an unknown instance simply has no events.
        Mutant {
            rule: "DRV-8",
            name: "UnknownInstanceHasNoEvents",
            run: |name| {
                driver::run(name, || {
                    M::new().events(|m, id, after| {
                        if id == INSTANCE {
                            m.real_events(id, after)
                        } else {
                            Ok(Vec::new())
                        }
                    })
                })
            },
        },
        // DRV-6 (verifier r3 M4): a driver only knows the events of its own
        // lifetime; a cursor from before the restart backfills nothing.
        Mutant {
            rule: "DRV-6",
            name: "OnlyOwnLifetimeEvents",
            run: |name| {
                driver::run(name, || {
                    M::new().events(|m, id, after| {
                        let all = m.real_events(id, None)?;
                        let own = &all[m.born.min(all.len())..];
                        let start = after
                            .and_then(|c| own.iter().position(|e| e.cursor == c))
                            .map_or(0, |i| i + 1);
                        Ok(own[start..].to_vec())
                    })
                })
            },
        },
        // DRV-9 (verifier r3 M2): no deduplication by message id; every
        // delivery starts a turn.
        Mutant {
            rule: "DRV-9",
            name: "RedeliversSameId",
            run: |name| {
                driver::run(name, || {
                    M::new().deliver(|m, id, msg, mode| {
                        let mut unique = msg.clone();
                        let n = m.seen.lock().unwrap().len();
                        m.seen.lock().unwrap().insert(format!("{}#{n}", msg.id));
                        unique.id = format!("{}#{n}", msg.id);
                        m.real_deliver(id, &unique, mode)
                    })
                })
            },
        },
        // DRV-9 (verifier r4 R4-3): dedup lives in the driver object; the
        // same id after a daemon restart starts a second turn.
        Mutant {
            rule: "DRV-9",
            name: "ObjDedup",
            run: |name| driver::run(name, || ObjDedup::over(FakeDriverFixture::new())),
        },
        // DRV-6 (verifier r4 R4-4): events the agent emitted while the
        // daemon was down are never backfilled.
        Mutant {
            rule: "DRV-6",
            name: "Gap",
            run: |name| driver::run(name, || Gap::fresh(FakeDriverFixture::new())),
        },
        // DRV-9 (verifier r5 R5-3): dedup remembers only the last id; the
        // older of two ids resent after a crash starts a second turn.
        Mutant {
            rule: "DRV-9",
            name: "LastIdDedup",
            run: |name| {
                driver::run(name, || LastIdDedup {
                    fx: FakeDriverFixture::new(),
                    last: Arc::default(),
                })
            },
        },
        // DRV-6 (verifier r5 R5-4): a restarted driver skips what an
        // earlier driver read; backfill from an older cursor loses events.
        Mutant {
            rule: "DRV-6",
            name: "ReadAck",
            run: |name| {
                driver::run(name, || ReadAck {
                    fx: FakeDriverFixture::new(),
                    acked: Arc::default(),
                    floor: 0,
                })
            },
        },
        // DRV-6 (verifier r5 R5-6): events emitted while no driver is alive
        // are lost.
        Mutant {
            rule: "DRV-6",
            name: "LiveJournal",
            run: |name| {
                driver::run(name, || {
                    LiveJournal::attach(FakeDriverFixture::new(), Arc::default())
                })
            },
        },
    ]
}
