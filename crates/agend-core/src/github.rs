//! Durable ownership shared by the GitHub adapter and store.
use alloc::string::String;
use core::future::Future;
use serde::{Deserialize, Serialize};

/// Immutable identity, committed before any remote branch or PR mutation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIdentity {
    pub task_id: String,
    pub local_repo: String,
    pub repository: String,
    pub repository_id: u64,
    pub base: String,
    pub branch: String,
    pub nonce: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubCleanup {
    pub close_attempted: bool,
    pub delete_attempted: bool,
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubChange {
    pub identity: GithubIdentity,
    pub pull_number: Option<u64>,
    /// The last head confirmed on the owned remote branch.
    pub pushed_head: Option<String>,
    /// Persisted before pushing; a lost response must reconcile this head first.
    pub push_intent: Option<String>,
    /// Persisted before a PR create attempt. Never cleared to authorize replay.
    pub create_attempted: bool,
    #[serde(default)]
    pub cleanup: GithubCleanup,
}

impl GithubChange {
    /// Only one durable identity and PR may ever belong to a task.
    pub fn follows(&self, previous: Option<&Self>) -> bool {
        let i = &self.identity;
        if i.task_id.is_empty()
            || i.local_repo.is_empty()
            || i.repository.is_empty()
            || i.repository_id == 0
            || i.base.is_empty()
            || i.branch.is_empty()
            || i.nonce.is_empty()
            || self.pull_number == Some(0)
            || self
                .pushed_head
                .iter()
                .chain(self.push_intent.iter())
                .any(|s| !full_sha(s))
            || (self.pull_number.is_some() && !self.create_attempted)
        {
            return false;
        }
        let Some(old) = previous else {
            return self.pull_number.is_none()
                && self.pushed_head.is_none()
                && self.push_intent.is_none()
                && !self.create_attempted
                && self.cleanup == GithubCleanup::default();
        };
        if (old.cleanup.complete && self != old)
            || (old.cleanup.close_attempted && !self.cleanup.close_attempted)
            || (old.cleanup.delete_attempted && !self.cleanup.delete_attempted)
            || i != &old.identity
            || (old.create_attempted && !self.create_attempted)
            || (self.pull_number.is_some() && !old.create_attempted)
            || (!old.create_attempted
                && self.create_attempted
                && (old.pushed_head.is_none() || old.push_intent.is_some()))
            || old.pull_number.is_some_and(|n| self.pull_number != Some(n))
        {
            return false;
        }
        // A pending push cannot be replaced. Confirm exactly that head before
        // starting another push; it is the next force-with-lease expectation.
        if old.push_intent.is_some() && self.push_intent != old.push_intent {
            return self.push_intent.is_none() && self.pushed_head == old.push_intent;
        }
        self.pushed_head == old.pushed_head
    }
}

fn full_sha(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedGithubChange {
    pub revision: u64,
    pub change: GithubChange,
}

pub trait GithubStore: Sync {
    type Error: Send;
    fn github_change<'a>(
        &'a self,
        task: &'a str,
    ) -> impl Future<Output = Result<Option<VersionedGithubChange>, Self::Error>> + Send + 'a;
    /// None means insert only. False means a stale revision; nothing changed.
    fn save_github_change<'a>(
        &'a self,
        expected: Option<u64>,
        change: &'a GithubChange,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send + 'a;
}
