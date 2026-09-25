//! Driver mutants: a `FakeDriver` with one method replaced. `restart`
//! builds the next mutant over `FakeDriver::restarted` (the same backend)
//! with the same replaced methods.

use std::collections::BTreeSet;
use std::sync::Mutex;

use agend_core::model::DeliveryState;
use agend_core::policy::busy::BusyLevel;
use agend_core::traits::{AgentMessage, DeliveryReceipt, Driver, DriverEvent, DriverEventKind};
use agend_testkit::block_on;
use agend_testkit::contract::driver::{self, DriverFixture};
use agend_testkit::contract::fakes::FakeDriverFixture;
use agend_testkit::fakes::FakeError;

use super::Mutant;

type Deliver = fn(&M, &str, &AgentMessage, BusyLevel) -> Result<DeliveryReceipt, FakeError>;
type Events = fn(&M, &str, Option<&str>) -> Result<Vec<DriverEvent>, FakeError>;

const INSTANCE: &str = FakeDriverFixture::INSTANCE;

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

impl DriverFixture for M {
    type Driver = Self;
    type Error = FakeError;
    fn driver(&self) -> &Self {
        self
    }
    fn instance_id(&self) -> &str {
        INSTANCE
    }
    /// The fake completes a turn inside `deliver`; no need to wait long.
    fn turn_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(200)
    }
    fn restart(&self) -> Self {
        let fx = self.fx.restart();
        let born = block_on(fx.driver.events(INSTANCE, None)).map_or(0, |e| e.len());
        Self {
            fx,
            seen: Mutex::new(BTreeSet::new()),
            born,
            deliver: self.deliver,
            events: self.events,
        }
    }
}

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
    ]
}
