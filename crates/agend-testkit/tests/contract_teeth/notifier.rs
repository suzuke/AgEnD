//! Notifier mutants: a `FakeNotifier` behind a broken `notify`.

use std::sync::Mutex;

use agend_core::traits::{Notification, NotificationSeverity, Notifier};
use agend_testkit::block_on;
use agend_testkit::contract::notifier::{self, NotifierFixture};
use agend_testkit::fakes::{FakeError, FakeNotifier};

use super::Mutant;

type Notify = fn(&M, &Notification) -> Result<(), FakeError>;

pub struct M {
    channel: FakeNotifier,
    held: Mutex<Vec<Notification>>,
    notify: Notify,
}

impl M {
    fn new(notify: Notify) -> Self {
        Self {
            channel: FakeNotifier::new(),
            held: Mutex::new(Vec::new()),
            notify,
        }
    }

    fn send(&self, n: &Notification) -> Result<(), FakeError> {
        block_on(self.channel.notify(n))
    }

    /// Sends `n` with its body cut by `cut`.
    fn send_cut(&self, n: &Notification, cut: fn(&str) -> String) -> Result<(), FakeError> {
        let mut short = n.clone();
        short.body = cut(&n.body);
        self.send(&short)
    }
}

/// The longest prefix of `s` of at most `max` bytes, cut at a character
/// boundary (what a byte-limited transport does).
fn prefix_bytes(s: &str, max: usize) -> String {
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if i + c.len_utf8() > max {
            break;
        }
        end = i + c.len_utf8();
    }
    s[..end].to_owned()
}

impl Notifier for M {
    type Error = FakeError;
    async fn notify(&self, n: &Notification) -> Result<(), FakeError> {
        (self.notify)(self, n)
    }
}

impl NotifierFixture for M {
    type Notifier = Self;
    type Error = FakeError;
    fn notifier(&self) -> &Self {
        self
    }
    fn received(&self) -> Vec<Notification> {
        self.channel.delivered()
    }
}

pub fn mutants() -> Vec<Mutant> {
    vec![
        // NTF-1 (verifier r2 N2): errors lose their task id.
        Mutant {
            rule: "NTF-1",
            name: "DropsTaskIdOnError",
            run: |name| {
                notifier::run(name, || {
                    M::new(|m, n| {
                        let mut sent = n.clone();
                        if sent.severity == NotificationSeverity::Error {
                            sent.task_id = None;
                        }
                        m.send(&sent)
                    })
                })
            },
        },
        // NTF-1: two severities collapsed into one.
        Mutant {
            rule: "NTF-1",
            name: "AttentionBecomesError",
            run: |name| {
                notifier::run(name, || {
                    M::new(|m, n| {
                        let mut sent = n.clone();
                        if sent.severity == NotificationSeverity::Attention {
                            sent.severity = NotificationSeverity::Error;
                        }
                        m.send(&sent)
                    })
                })
            },
        },
        // NTF-2 (verifier r2 N3): bodies cut at 64 bytes (a push preview).
        Mutant {
            rule: "NTF-2",
            name: "TruncatesAt64Bytes",
            run: |name| {
                notifier::run(name, || {
                    M::new(|m, n| m.send_cut(n, |b| prefix_bytes(b, 64)))
                })
            },
        },
        // NTF-2 (v1 lesson #2): bodies cut at 200 characters.
        Mutant {
            rule: "NTF-2",
            name: "TruncatesAt200Chars",
            run: |name| {
                notifier::run(name, || {
                    M::new(|m, n| m.send_cut(n, |b| b.chars().take(200).collect()))
                })
            },
        },
        // NTF-2: bodies cut at Telegram's 4096-byte message limit.
        Mutant {
            rule: "NTF-2",
            name: "TruncatesAt4096Bytes",
            run: |name| {
                notifier::run(name, || {
                    M::new(|m, n| m.send_cut(n, |b| prefix_bytes(b, 4096)))
                })
            },
        },
        // NTF-3 (verifier r2 N1): title and body trimmed.
        Mutant {
            rule: "NTF-3",
            name: "TrimsWhitespace",
            run: |name| {
                notifier::run(name, || {
                    M::new(|m, n| {
                        let mut sent = n.clone();
                        sent.title = sent.title.trim().into();
                        sent.body = sent.body.trim().into();
                        m.send(&sent)
                    })
                })
            },
        },
        // NTF-4: Info waits until the next more urgent notification.
        Mutant {
            rule: "NTF-4",
            name: "HoldsInfoBack",
            run: |name| {
                notifier::run(name, || {
                    M::new(|m, n| {
                        if n.severity == NotificationSeverity::Info {
                            m.held.lock().unwrap().push(n.clone());
                            return Ok(());
                        }
                        m.send(n)?;
                        let held: Vec<_> = m.held.lock().unwrap().drain(..).collect();
                        held.iter().try_for_each(|h| m.send(h))
                    })
                })
            },
        },
    ]
}
