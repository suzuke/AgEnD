//! The fleet view and the event log of the client protocol (gate 8 P4, P5).
//!
//! One mutex holds what clients see (instances, tasks, needs-you items) and
//! the last [`RETAINED_EVENTS`] events. Every change updates the view and
//! publishes its event under that lock, so a fleet view and its
//! `as_of_event_id`, and a subscription's backlog and its live receiver,
//! always fit together: nothing is missed or repeated between them.
//!
//! - Event ids: the first is `base + 1`, `base` = boot time in unix ms × 1000.
//!   Events live in memory only; after a restart clients fetch the view
//!   again.
//! - Cursor rules: see `agend_core::protocol::client` and [`Fleet::subscribe`].
//! - Live events go through a `tokio::sync::broadcast` of
//!   [`RETAINED_EVENTS`]; a receiver that falls further behind gets
//!   `Lagged` and the server closes that connection (P8).
//! - Needs-you items here are the `failed` instances (P5); `waiting_since`
//!   is when this daemon first saw the failure.
//!
//! Must NOT: hold the lock across an await, or do I/O.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard};

use agend_core::model::DEFAULT_TEAM;
use agend_core::protocol::client::{
    AttentionAction, AttentionRequiredData, AttentionResolvedData, DaemonEvent, EventData,
    FleetView, InstanceChangedData, InstanceView, RETAINED_EVENTS, TaskView, TeamView,
    order_attention,
};
use tokio::sync::broadcast;

/// The needs-you id of a failed instance.
pub fn failed_attention_id(instance_id: &str) -> String {
    format!("instance-failed:{instance_id}")
}

pub struct Fleet {
    inner: Mutex<Inner>,
    live: broadcast::Sender<EventData>,
}

struct Inner {
    latest: u64,
    log: VecDeque<EventData>,
    instances: BTreeMap<String, InstanceView>,
    tasks: Vec<TaskView>,
    attention: BTreeMap<String, AttentionRequiredData>,
}

/// A subscription that can continue: the events after its cursor, then
/// the live ones.
pub struct Subscription {
    pub backlog: Vec<EventData>,
    pub live: broadcast::Receiver<EventData>,
}

impl Fleet {
    /// An empty fleet whose first event id is `boot_unix_ms * 1000 + 1`.
    pub fn new(boot_unix_ms: u64) -> Self {
        let (live, _) = broadcast::channel(RETAINED_EVENTS);
        Self {
            inner: Mutex::new(Inner {
                latest: boot_unix_ms.saturating_mul(1000),
                log: VecDeque::new(),
                instances: BTreeMap::new(),
                tasks: Vec::new(),
                attention: BTreeMap::new(),
            }),
            live,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Appends `event` and sends it to every subscriber. Returns its id.
    pub fn publish(&self, event: DaemonEvent) -> u64 {
        let mut inner = self.lock();
        self.push(&mut inner, event)
    }

    fn push(&self, inner: &mut Inner, event: DaemonEvent) -> u64 {
        inner.latest += 1;
        let data = EventData {
            event_id: inner.latest,
            event,
        };
        inner.log.push_back(data.clone());
        if inner.log.len() > RETAINED_EVENTS {
            inner.log.pop_front();
        }
        // No subscriber is not an error.
        let _ = self.live.send(data);
        inner.latest
    }

    /// Sets the tasks of the view (read from the DB at boot; nothing
    /// changes them before gate 10).
    pub fn set_tasks(&self, tasks: Vec<TaskView>) {
        self.lock().tasks = tasks;
    }

    /// Shows `instance`; publishes `instance_changed` (with `summary`) when
    /// its view changed.
    pub fn set_instance(&self, instance: InstanceView, summary: String) {
        let mut inner = self.lock();
        if inner.instances.get(&instance.instance_id) == Some(&instance) {
            return;
        }
        inner
            .instances
            .insert(instance.instance_id.clone(), instance.clone());
        let event = DaemonEvent::InstanceChanged {
            data: InstanceChangedData {
                instance_id: instance.instance_id.clone(),
                summary,
                instance: Some(instance),
            },
        };
        self.push(&mut inner, event);
    }

    pub fn instance(&self, id: &str) -> Option<InstanceView> {
        self.lock().instances.get(id).cloned()
    }

    /// Adds a needs-you item (it must have an `attention_id`) and publishes
    /// `attention_required`; an item already listed is left as it is.
    pub fn raise(&self, item: AttentionRequiredData) {
        let Some(id) = item.attention_id.clone() else {
            return;
        };
        let mut inner = self.lock();
        if inner.attention.contains_key(&id) {
            return;
        }
        inner.attention.insert(id, item.clone());
        self.push(&mut inner, DaemonEvent::AttentionRequired { data: item });
    }

    pub fn attention(&self, attention_id: &str) -> Option<AttentionRequiredData> {
        self.lock().attention.get(attention_id).cloned()
    }

    /// Takes the item off the list and publishes `attention_resolved`, if
    /// it is listed with `action` among its actions; returns the item. Two
    /// resolves of one item: only the first gets it.
    pub fn resolve(
        &self,
        attention_id: &str,
        action: AttentionAction,
    ) -> Option<AttentionRequiredData> {
        let mut inner = self.lock();
        if !inner
            .attention
            .get(attention_id)
            .is_some_and(|item| item.actions.contains(&action))
        {
            return None;
        }
        let item = inner.attention.remove(attention_id)?;
        let event = DaemonEvent::AttentionResolved {
            data: AttentionResolvedData {
                attention_id: attention_id.to_owned(),
                action,
            },
        };
        self.push(&mut inner, event);
        Some(item)
    }

    /// The fleet view now; subscribing after its `as_of_event_id` gives every
    /// later change.
    pub fn view(&self) -> FleetView {
        let inner = self.lock();
        let mut attention: Vec<AttentionRequiredData> = inner.attention.values().cloned().collect();
        order_attention(&mut attention);
        FleetView {
            as_of_event_id: inner.latest,
            teams: vec![TeamView {
                team_id: DEFAULT_TEAM.to_owned(),
            }],
            tasks: inner.tasks.clone(),
            instances: inner.instances.values().cloned().collect(),
            attention,
        }
    }

    /// Subscribes after `after`. `None`: every retained event, then live
    /// ones (protocol 1.0). `Some(cursor)` from "oldest retained − 1" to the
    /// newest id (the base while there is no event): the events after it.
    /// Anything else is `Err` with the `event_gap` message.
    pub fn subscribe(&self, after: Option<u64>) -> Result<Subscription, String> {
        let inner = self.lock();
        let oldest = inner.log.front().map_or(inner.latest + 1, |e| e.event_id);
        if let Some(after) = after
            && !(after.saturating_add(1) >= oldest && after <= inner.latest)
        {
            return Err(format!(
                "cannot continue after event {after}: this daemon has events {oldest} to {}; \
                 fetch the fleet view again",
                inner.latest
            ));
        }
        let after = after.unwrap_or(0);
        Ok(Subscription {
            backlog: inner
                .log
                .iter()
                .filter(|e| e.event_id > after)
                .cloned()
                .collect(),
            live: self.live.subscribe(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::protocol::client::{AgentState, TaskChangedData};

    const BOOT: u64 = 1_790_000_000_000;
    const BASE: u64 = BOOT * 1000;

    fn event(n: u64) -> DaemonEvent {
        DaemonEvent::TaskChanged {
            data: TaskChangedData {
                task_id: format!("T-{n}"),
                summary: String::new(),
                task: None,
            },
        }
    }

    fn ids(sub: &Subscription) -> Vec<u64> {
        sub.backlog.iter().map(|e| e.event_id).collect()
    }

    #[test]
    fn the_first_event_is_the_base_plus_one() {
        let fleet = Fleet::new(BOOT);
        assert_eq!(fleet.view().as_of_event_id, BASE);
        assert_eq!(fleet.publish(event(1)), BASE + 1);
    }

    #[test]
    fn without_events_only_the_base_continues() {
        let fleet = Fleet::new(BOOT);
        assert!(fleet.subscribe(Some(BASE)).is_ok());
        for bad in [0, 1, BASE - 1, BASE + 1] {
            assert!(fleet.subscribe(Some(bad)).is_err(), "{bad}");
        }
        assert_eq!(ids(&fleet.subscribe(None).unwrap()), Vec::<u64>::new());
    }

    #[test]
    fn cursor_from_oldest_minus_one_to_latest_continues_everything_else_is_a_gap() {
        let fleet = Fleet::new(BOOT);
        for n in 0..RETAINED_EVENTS as u64 + 10 {
            fleet.publish(event(n));
        }
        let latest = BASE + RETAINED_EVENTS as u64 + 10;
        let oldest = latest - RETAINED_EVENTS as u64 + 1;
        assert_eq!(
            ids(&fleet.subscribe(Some(latest)).unwrap()),
            Vec::<u64>::new()
        );
        assert_eq!(
            ids(&fleet.subscribe(Some(oldest - 1)).unwrap()).len(),
            RETAINED_EVENTS
        );
        assert_eq!(ids(&fleet.subscribe(None).unwrap())[0], oldest);
        let error = fleet.subscribe(Some(oldest - 2)).err().unwrap();
        assert!(error.contains("fetch the fleet view again"), "{error}");
        assert!(fleet.subscribe(Some(latest + 1)).is_err());
    }

    /// Known risk (P4): the clock stepped back between two boots. A cursor
    /// newer than the new daemon's ids is a gap; one that falls inside the
    /// new range is taken (events can be missed), which is why a 1.1 client
    /// never reuses a cursor after reconnecting.
    #[test]
    fn a_clock_stepped_back_is_a_gap_unless_the_old_cursor_falls_in_range() {
        let old = Fleet::new(BOOT);
        let old_cursor = old.publish(event(1));
        let new = Fleet::new(BOOT - 5);
        assert!(
            new.subscribe(Some(old_cursor)).is_err(),
            "newer than latest"
        );
        let new = Fleet::new(BOOT - 1);
        while new.view().as_of_event_id < old_cursor {
            new.publish(event(0));
        }
        assert!(new.subscribe(Some(old_cursor)).is_ok(), "the accepted risk");
    }

    #[test]
    fn view_and_backlog_fit_together_and_items_come_and_go_once() {
        let fleet = Fleet::new(BOOT);
        let view = InstanceView {
            instance_id: "g8-1".into(),
            team_id: DEFAULT_TEAM.into(),
            backend: "claude".into(),
            state: AgentState::Starting,
        };
        fleet.set_instance(view.clone(), "starting".into());
        fleet.set_instance(view.clone(), "starting".into());
        let as_of = fleet.view().as_of_event_id;
        assert_eq!(as_of, BASE + 1, "an unchanged instance publishes nothing");
        let mut sub = fleet.subscribe(Some(as_of)).unwrap();
        let item = AttentionRequiredData {
            reason: "g8-1 failed".into(),
            task_id: None,
            ask: None,
            recap: None,
            attention_id: Some(failed_attention_id("g8-1")),
            unblocks: Some(0),
            waiting_since_unix_ms: Some(BOOT),
            if_ignored: None,
            actions: vec![AttentionAction::Retry],
            instance_id: Some("g8-1".into()),
        };
        fleet.raise(item.clone());
        fleet.raise(item);
        assert_eq!(fleet.view().attention.len(), 1);
        assert!(
            fleet
                .resolve("instance-failed:g8-1", AttentionAction::Unknown)
                .is_none(),
            "an action the item does not list"
        );
        assert!(
            fleet
                .resolve("instance-failed:g8-1", AttentionAction::Retry)
                .is_some()
        );
        assert!(
            fleet
                .resolve("instance-failed:g8-1", AttentionAction::Retry)
                .is_none()
        );
        assert!(fleet.view().attention.is_empty());
        let live: Vec<u64> = (0..2)
            .map(|_| sub.live.try_recv().unwrap().event_id)
            .collect();
        assert_eq!(live, [BASE + 2, BASE + 3]);
        assert!(sub.live.try_recv().is_err());
    }
}
