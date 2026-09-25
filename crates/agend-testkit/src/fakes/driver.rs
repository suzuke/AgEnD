use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use agend_core::model::DeliveryState;
use agend_core::policy::busy::BusyLevel;
use agend_core::traits::{AgentMessage, DeliveryReceipt, Driver, DriverEvent, DriverEventKind};

use super::{Failures, FakeError, lock};

const OPERATIONS: &[&str] = &["deliver", "events"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverCall {
    Deliver {
        instance_id: String,
        message: AgentMessage,
        mode: BusyLevel,
    },
    Events {
        instance_id: String,
        after_cursor: Option<String>,
    },
}

/// A driver without a backend. Instances must be added first; delivering to
/// an unknown one fails. By default each delivery is answered like an idle
/// agent: receipt `Sent` with backend id `fake-<message id>`, then the events
/// `BusyChanged{true}`, `MessageConfirmed`, `TurnCompleted`,
/// `BusyChanged{false}`. `set_auto_turn(false)` turns that off so a test
/// scripts events with `push_event`.
///
/// Delivery is idempotent by message id (per instance): delivering an id
/// again answers with a receipt but starts no second turn.
///
/// The instances, their events and the delivered ids play the part of the
/// backend ([`FakeBackend`]): [`FakeDriver::connect`] is a new driver (after
/// a daemon restart) on it, so it backfills events from cursors earlier
/// drivers handed out. Receipts, `calls()` and `fail_next` stay per driver.
///
/// Cursors are one global zero-padded counter, so they are unique and
/// ordered. Beyond the contract: an unknown cursor is an error.
#[derive(Debug)]
pub struct FakeDriver {
    backend: Arc<Mutex<Backend>>,
    state: Mutex<State>,
}

/// The backend of a [`FakeDriver`], standing in for the agents in their
/// holders: not a driver, and it outlives every driver. Clones share it.
#[derive(Debug, Clone)]
pub struct FakeBackend(Arc<Mutex<Backend>>);

impl FakeBackend {
    /// The agent emits an event on its own (no driver involved); returns
    /// its cursor. The instance must exist.
    pub fn push_event(&self, instance_id: &str, kind: DriverEventKind) -> String {
        let mut backend = lock(&self.0);
        assert!(
            backend.instances.contains_key(instance_id),
            "push_event: add instance {instance_id} first"
        );
        backend.push(instance_id, kind)
    }
}

/// What outlives a driver: the backend's instances, events and history.
#[derive(Debug)]
struct Backend {
    instances: BTreeMap<String, Vec<DriverEvent>>,
    /// `(instance id, message id)` of every accepted delivery.
    delivered: BTreeSet<(String, String)>,
    auto_turn: bool,
    next_cursor: u64,
}

#[derive(Debug)]
struct State {
    receipts: VecDeque<DeliveryReceipt>,
    calls: Vec<DriverCall>,
    failures: Failures,
}

impl Backend {
    fn push(&mut self, instance_id: &str, kind: DriverEventKind) -> String {
        self.next_cursor += 1;
        let cursor = format!("{:010}", self.next_cursor);
        self.instances
            .entry(instance_id.to_owned())
            .or_default()
            .push(DriverEvent {
                cursor: cursor.clone(),
                kind,
            });
        cursor
    }
}

impl FakeDriver {
    pub fn new() -> Self {
        Self::connect(&FakeBackend(Arc::new(Mutex::new(Backend {
            instances: BTreeMap::new(),
            delivered: BTreeSet::new(),
            auto_turn: true,
            next_cursor: 0,
        }))))
    }

    /// A new driver on `backend`, as a restarted daemon reconnects. Its
    /// `calls()` and scripted receipts start empty.
    pub fn connect(backend: &FakeBackend) -> Self {
        Self {
            backend: Arc::clone(&backend.0),
            state: Mutex::new(State {
                receipts: VecDeque::new(),
                calls: Vec::new(),
                failures: Failures::new(OPERATIONS),
            }),
        }
    }

    /// The backend this driver talks to.
    pub fn backend(&self) -> FakeBackend {
        FakeBackend(Arc::clone(&self.backend))
    }

    pub fn with_instance(self, instance_id: &str) -> Self {
        self.add_instance(instance_id);
        self
    }

    pub fn add_instance(&self, instance_id: &str) {
        lock(&self.backend)
            .instances
            .entry(instance_id.to_owned())
            .or_default();
    }

    pub fn set_auto_turn(&self, enabled: bool) {
        lock(&self.backend).auto_turn = enabled;
    }

    /// The receipt for the next delivery instead of the default one.
    pub fn next_receipt(&self, receipt: DeliveryReceipt) {
        lock(&self.state).receipts.push_back(receipt);
    }

    /// Appends an event for a known instance and returns its cursor.
    pub fn push_event(&self, instance_id: &str, kind: DriverEventKind) -> String {
        self.backend().push_event(instance_id, kind)
    }

    pub fn fail_next(&self, operation: &str, message: &str) {
        lock(&self.state).failures.push(operation, message);
    }

    pub fn calls(&self) -> Vec<DriverCall> {
        lock(&self.state).calls.clone()
    }

    /// Messages delivered to `instance_id`, in order (failed calls included).
    pub fn delivered_to(&self, instance_id: &str) -> Vec<AgentMessage> {
        lock(&self.state)
            .calls
            .iter()
            .filter_map(|call| match call {
                DriverCall::Deliver {
                    instance_id: id,
                    message,
                    ..
                } if id == instance_id => Some(message.clone()),
                _ => None,
            })
            .collect()
    }
}

impl Default for FakeDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver for FakeDriver {
    type Error = FakeError;

    async fn deliver(
        &self,
        instance_id: &str,
        message: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(DriverCall::Deliver {
            instance_id: instance_id.to_owned(),
            message: message.clone(),
            mode,
        });
        if let Some(error) = state.failures.take("deliver") {
            return Err(error);
        }
        let mut backend = lock(&self.backend);
        if !backend.instances.contains_key(instance_id) {
            return Err(FakeError::new(
                "deliver",
                format!("unknown instance {instance_id}"),
            ));
        }
        let receipt = state.receipts.pop_front().unwrap_or(DeliveryReceipt {
            backend_message_id: Some(format!("fake-{}", message.id)),
            state: DeliveryState::Sent,
        });
        let first = backend
            .delivered
            .insert((instance_id.to_owned(), message.id.clone()));
        if first && backend.auto_turn {
            backend.push(instance_id, DriverEventKind::BusyChanged { busy: true });
            backend.push(
                instance_id,
                DriverEventKind::MessageConfirmed {
                    message_id: message.id.clone(),
                },
            );
            backend.push(
                instance_id,
                DriverEventKind::TurnCompleted {
                    summary: Some(format!("fake reply to {}", message.id)),
                },
            );
            backend.push(instance_id, DriverEventKind::BusyChanged { busy: false });
        }
        Ok(receipt)
    }

    async fn events(
        &self,
        instance_id: &str,
        after_cursor: Option<&str>,
    ) -> Result<Vec<DriverEvent>, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(DriverCall::Events {
            instance_id: instance_id.to_owned(),
            after_cursor: after_cursor.map(str::to_owned),
        });
        if let Some(error) = state.failures.take("events") {
            return Err(error);
        }
        let backend = lock(&self.backend);
        let Some(events) = backend.instances.get(instance_id) else {
            return Err(FakeError::new(
                "events",
                format!("unknown instance {instance_id}"),
            ));
        };
        let start = match after_cursor {
            None => 0,
            Some(cursor) => match events.iter().position(|e| e.cursor == cursor) {
                Some(index) => index + 1,
                None => {
                    return Err(FakeError::new(
                        "events",
                        format!("unknown cursor {cursor} for {instance_id}"),
                    ));
                }
            },
        };
        Ok(events[start..].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;

    fn message(id: &str) -> AgentMessage {
        AgentMessage {
            id: id.into(),
            from: "operator".into(),
            task_id: None,
            body: "hi".into(),
        }
    }

    #[test]
    fn same_message_id_makes_one_turn_and_a_restarted_driver_backfills() {
        let driver = FakeDriver::new().with_instance("dev-1");
        block_on(driver.deliver("dev-1", &message("m-1"), BusyLevel::Queue)).unwrap();
        let cursor = block_on(driver.events("dev-1", None)).unwrap()[1]
            .cursor
            .clone();
        let again = block_on(driver.deliver("dev-1", &message("m-1"), BusyLevel::Queue));
        assert_eq!(again.unwrap().state, DeliveryState::Sent);
        assert_eq!(block_on(driver.events("dev-1", None)).unwrap().len(), 4);
        let backend = driver.backend();
        drop(driver);
        let restarted = FakeDriver::connect(&backend);
        assert_eq!(
            block_on(restarted.events("dev-1", Some(&cursor)))
                .unwrap()
                .len(),
            2
        );
        block_on(restarted.deliver("dev-1", &message("m-1"), BusyLevel::Queue)).unwrap();
        assert_eq!(block_on(restarted.events("dev-1", None)).unwrap().len(), 4);
        assert_eq!(restarted.calls().len(), 3);
    }

    #[test]
    fn scripted_events_and_receipts_replace_the_auto_turn() {
        let driver = FakeDriver::new().with_instance("dev-1");
        driver.set_auto_turn(false);
        driver.next_receipt(DeliveryReceipt {
            backend_message_id: None,
            state: DeliveryState::Queued,
        });
        let receipt = block_on(driver.deliver("dev-1", &message("m-1"), BusyLevel::Queue));
        assert_eq!(receipt.unwrap().state, DeliveryState::Queued);
        assert!(block_on(driver.events("dev-1", None)).unwrap().is_empty());
        let cursor = driver.push_event(
            "dev-1",
            DriverEventKind::UsageLimit {
                reset_at_unix_ms: Some(9),
            },
        );
        let events = block_on(driver.events("dev-1", None)).unwrap();
        assert_eq!(events.len(), 1);
        assert!(
            block_on(driver.events("dev-1", Some(&cursor)))
                .unwrap()
                .is_empty()
        );
        assert!(block_on(driver.events("dev-1", Some("nope"))).is_err());
        assert_eq!(driver.delivered_to("dev-1"), vec![message("m-1")]);
    }
}
