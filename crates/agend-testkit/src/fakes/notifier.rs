use std::sync::Mutex;

use agend_core::traits::{Notification, Notifier};

use super::{Failures, FakeError, lock};

const OPERATIONS: &[&str] = &["notify"];

/// Records notifications. `calls()` has every attempt; `delivered()` only
/// the ones that did not fail.
#[derive(Debug)]
pub struct FakeNotifier {
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    calls: Vec<Notification>,
    delivered: Vec<Notification>,
    failures: Failures,
}

impl FakeNotifier {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                calls: Vec::new(),
                delivered: Vec::new(),
                failures: Failures::new(OPERATIONS),
            }),
        }
    }

    pub fn fail_next(&self, operation: &str, message: &str) {
        lock(&self.state).failures.push(operation, message);
    }

    pub fn calls(&self) -> Vec<Notification> {
        lock(&self.state).calls.clone()
    }

    pub fn delivered(&self) -> Vec<Notification> {
        lock(&self.state).delivered.clone()
    }
}

impl Default for FakeNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifier for FakeNotifier {
    type Error = FakeError;

    async fn notify(&self, notification: &Notification) -> Result<(), FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(notification.clone());
        if let Some(error) = state.failures.take("notify") {
            return Err(error);
        }
        state.delivered.push(notification.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use agend_core::traits::NotificationSeverity;

    #[test]
    fn failed_notifications_are_attempted_but_not_delivered() {
        let notifier = FakeNotifier::new();
        let note = Notification {
            severity: NotificationSeverity::Attention,
            title: "needs you".into(),
            body: "T-1 waits for approval".into(),
            task_id: Some("T-1".into()),
        };
        notifier.fail_next("notify", "telegram down");
        assert!(block_on(notifier.notify(&note)).is_err());
        block_on(notifier.notify(&note)).unwrap();
        assert_eq!(notifier.calls().len(), 2);
        assert_eq!(notifier.delivered(), vec![note]);
    }
}
