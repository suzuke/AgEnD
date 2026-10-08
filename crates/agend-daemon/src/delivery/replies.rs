//! Lifetimes of content-bearing server replies, including socket backpressure.
//! A fence captures only replies already started. Once SQLite has paused new
//! reservations, later requests cannot add new content to that captured set.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

#[derive(Default)]
pub struct Replies {
    active: Mutex<BTreeMap<String, Vec<Weak<()>>>>,
}

pub struct ReplyFence(Vec<Weak<()>>);
impl ReplyFence {
    /// Local writes have finished or their connection tasks were dropped.
    /// This does not prove the backend consumed content or completed its turn.
    pub fn drained(&self) -> bool {
        self.0.iter().all(|token| token.strong_count() == 0)
    }
}

pub(crate) struct ReplyGuard {
    token: Option<Arc<()>>,
    replies: Arc<Replies>,
    instance: String,
}
impl Replies {
    pub(crate) fn begin(self: &Arc<Self>, instance: &str) -> ReplyGuard {
        let token = Arc::new(());
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(instance.to_owned())
            .or_default()
            .push(Arc::downgrade(&token));
        ReplyGuard {
            token: Some(token),
            replies: self.clone(),
            instance: instance.into(),
        }
    }

    /// Call only after new content reservation is durably paused. Unrelated
    /// instances and subsequent empty polls cannot starve this fence.
    pub fn fence(&self, instance: &str) -> ReplyFence {
        ReplyFence(
            self.active
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(instance)
                .cloned()
                .unwrap_or_default(),
        )
    }
}
impl Drop for ReplyGuard {
    fn drop(&mut self) {
        self.token.take();
        let mut active = self
            .replies
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(tokens) = active.get_mut(&self.instance) {
            tokens.retain(|token| token.strong_count() != 0);
            if tokens.is_empty() {
                active.remove(&self.instance);
            }
        }
    }
}
