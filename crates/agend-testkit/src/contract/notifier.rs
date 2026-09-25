//! `Notifier` contract (rules NTF-1..4 in CONTRACTS.md): an accepted
//! notification reaches the channel with every field intact: severity, title
//! and task id unchanged, the body whole however long (v1 truncated pushes,
//! docs/V1-LESSONS.md #2), no whitespace trimmed, in the order sent.
//!
//! Not pinned: retrying a failed send (the trait only returns the error).

use std::fmt::Debug;

use agend_core::traits::{Notification, NotificationSeverity, Notifier};

use super::{Case, CaseResult, Report, ensure, ok, run_suite};
use crate::block_on;

/// Characters in the long body: well past v1's 200-character cut and a
/// Telegram preview, and several KiB of multi-byte UTF-8.
pub const LONG_BODY_CHARS: usize = 3_000;

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
            rule: "NTF-1",
            name: "fields_arrive_unchanged",
            check: |fx| fields_arrive_unchanged(&fx),
        },
        Case {
            rule: "NTF-2",
            name: "long_body_arrives_whole",
            check: |fx| long_body_arrives_whole(&fx),
        },
        Case {
            rule: "NTF-3",
            name: "whitespace_is_kept",
            check: |fx| whitespace_is_kept(&fx),
        },
        Case {
            rule: "NTF-4",
            name: "notifications_keep_their_order",
            check: |fx| notifications_keep_their_order(&fx),
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

/// The body of [`long_body_arrives_whole`]: numbered multi-byte lines, so a
/// cut anywhere shows.
pub fn long_body() -> String {
    let line = "第 0000 行：審查要求修改 build.rs 的錯誤處理 ✅\n";
    let per_line = line.chars().count();
    (0..LONG_BODY_CHARS.div_ceil(per_line))
        .map(|n| line.replace("0000", &format!("{n:04}")))
        .collect()
}

fn send_all<F: NotifierFixture>(fx: &F, sent: &[Notification]) -> CaseResult {
    for n in sent {
        ok("notify", block_on(fx.notifier().notify(n)))?;
    }
    let received = fx.received();
    ensure(received == sent, || {
        format!("expected {sent:?}, received {received:?}")
    })
}

fn fields_arrive_unchanged<F: NotifierFixture>(fx: &F) -> CaseResult {
    let sent: Vec<Notification> = [
        NotificationSeverity::Info,
        NotificationSeverity::Attention,
        NotificationSeverity::Error,
    ]
    .into_iter()
    .enumerate()
    .flat_map(|(i, severity)| {
        let with_task = note(severity, 2 * i as u32 + 1);
        let without_task = note(severity, 2 * i as u32 + 2);
        [with_task, without_task]
    })
    .collect();
    send_all(fx, &sent)
}

fn long_body_arrives_whole<F: NotifierFixture>(fx: &F) -> CaseResult {
    let mut long = note(NotificationSeverity::Attention, 1);
    long.body = long_body();
    ok("notify", block_on(fx.notifier().notify(&long)))?;
    let received = fx.received();
    let bodies: Vec<(usize, usize)> = received
        .iter()
        .map(|n| (n.body.chars().count(), n.body.len()))
        .collect();
    ensure(received == [long.clone()], || {
        format!(
            "expected one notification with a {}-character ({} byte) body, received (characters, bytes) {bodies:?}",
            long.body.chars().count(),
            long.body.len()
        )
    })
}

fn whitespace_is_kept<F: NotifierFixture>(fx: &F) -> CaseResult {
    let mut padded = note(NotificationSeverity::Info, 1);
    padded.title = "  padded title \t".into();
    padded.body = "\n  indented first line\nlast line  \n\n".into();
    send_all(fx, &[padded])
}

fn notifications_keep_their_order<F: NotifierFixture>(fx: &F) -> CaseResult {
    send_all(
        fx,
        &[
            note(NotificationSeverity::Info, 1),
            note(NotificationSeverity::Error, 2),
            note(NotificationSeverity::Attention, 3),
        ],
    )
}
