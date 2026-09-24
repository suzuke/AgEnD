//! `Notifier` contract: an accepted notification reaches the channel with
//! every field intact, in the order sent.

use std::fmt::Debug;

use agend_core::traits::{Notification, NotificationSeverity, Notifier};

use super::{Case, CaseResult, Report, ensure, ok, run_suite};
use crate::block_on;

pub trait NotifierFixture {
    type Notifier: Notifier<Error = Self::Error>;
    type Error: Send + Debug;

    fn notifier(&self) -> &Self::Notifier;

    /// What the channel received (for a real channel: read back from a test
    /// chat or a recording transport).
    fn received(&self) -> Vec<Notification>;
}

pub fn cases<F: NotifierFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            name: "notification_arrives_intact",
            check: notification_arrives_intact,
        },
        Case {
            name: "notifications_keep_their_order",
            check: notifications_keep_their_order,
        },
    ]
}

pub fn run<F: NotifierFixture>(implementation: &str, make: impl FnMut() -> F) -> Report {
    run_suite("Notifier", implementation, &cases::<F>(), make)
}

fn note(severity: NotificationSeverity, n: u32) -> Notification {
    Notification {
        severity,
        title: format!("contract notification {n}"),
        body: format!("body {n}: T-contract needs you\nsecond line"),
        task_id: (n % 2 == 1).then(|| "T-contract".into()),
    }
}

fn notification_arrives_intact<F: NotifierFixture>(fx: &F) -> CaseResult {
    let sent = note(NotificationSeverity::Attention, 1);
    ok("notify", block_on(fx.notifier().notify(&sent)))?;
    let received = fx.received();
    ensure(received == [sent.clone()], || {
        format!("expected [{sent:?}], received {received:?}")
    })
}

fn notifications_keep_their_order<F: NotifierFixture>(fx: &F) -> CaseResult {
    let sent = vec![
        note(NotificationSeverity::Info, 1),
        note(NotificationSeverity::Error, 2),
        note(NotificationSeverity::Attention, 3),
    ];
    for n in &sent {
        ok("notify", block_on(fx.notifier().notify(n)))?;
    }
    let received = fx.received();
    ensure(received == sent, || {
        format!("expected {sent:?}, received {received:?}")
    })
}
