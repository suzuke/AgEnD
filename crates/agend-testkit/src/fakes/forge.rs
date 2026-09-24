use std::collections::BTreeMap;
use std::sync::Mutex;

use agend_core::traits::{Forge, MergeRequest, MergeResult, Submission, SubmittedChange};

use super::{Failures, FakeError, lock};

const OPERATIONS: &[&str] = &["submit", "head", "merge_if_head_is"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgeCall {
    Submit(Submission),
    Head { branch: String },
    MergeIfHeadIs(MergeRequest),
}

/// A forge without git: branches are lists of generated commit ids (40 hex
/// digits from a counter). `push` is the fixture for "someone committed".
/// Beyond the contract: submitting a branch again returns the same change id
/// with the current head, like updating an open pull request.
#[derive(Debug)]
pub struct FakeForge {
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    branches: BTreeMap<String, String>,
    changes: BTreeMap<String, SubmittedChange>,
    merges: Vec<(String, String)>,
    next_commit: u64,
    calls: Vec<ForgeCall>,
    failures: Failures,
}

impl State {
    fn new_commit(&mut self) -> String {
        self.next_commit += 1;
        format!("{:040x}", self.next_commit)
    }
}

impl FakeForge {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                branches: BTreeMap::new(),
                changes: BTreeMap::new(),
                merges: Vec::new(),
                next_commit: 0,
                calls: Vec::new(),
                failures: Failures::new(OPERATIONS),
            }),
        }
    }

    /// Adds a new commit on `branch` (creating it) and returns the new head.
    pub fn push(&self, branch: &str) -> String {
        let mut state = lock(&self.state);
        let head = state.new_commit();
        state.branches.insert(branch.to_owned(), head.clone());
        head
    }

    /// `(branch, merge_commit)` for every successful merge, in order.
    pub fn merges(&self) -> Vec<(String, String)> {
        lock(&self.state).merges.clone()
    }

    pub fn fail_next(&self, operation: &str, message: &str) {
        lock(&self.state).failures.push(operation, message);
    }

    pub fn calls(&self) -> Vec<ForgeCall> {
        lock(&self.state).calls.clone()
    }
}

impl Default for FakeForge {
    fn default() -> Self {
        Self::new()
    }
}

fn unknown_branch(operation: &'static str, branch: &str) -> FakeError {
    FakeError::new(operation, format!("unknown branch {branch}"))
}

impl Forge for FakeForge {
    type Error = FakeError;

    async fn submit(&self, change: &Submission) -> Result<SubmittedChange, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(ForgeCall::Submit(change.clone()));
        if let Some(error) = state.failures.take("submit") {
            return Err(error);
        }
        let Some(head) = state.branches.get(&change.branch).cloned() else {
            return Err(unknown_branch("submit", &change.branch));
        };
        let number = state.changes.len() + 1;
        let submitted = state
            .changes
            .entry(change.branch.clone())
            .or_insert_with(|| SubmittedChange {
                id: format!("change-{number}"),
                url: Some(format!("fake://forge/changes/{number}")),
                head: head.clone(),
            });
        submitted.head = head;
        Ok(submitted.clone())
    }

    async fn head(&self, branch: &str) -> Result<String, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(ForgeCall::Head {
            branch: branch.to_owned(),
        });
        if let Some(error) = state.failures.take("head") {
            return Err(error);
        }
        state
            .branches
            .get(branch)
            .cloned()
            .ok_or_else(|| unknown_branch("head", branch))
    }

    async fn merge_if_head_is(&self, request: &MergeRequest) -> Result<MergeResult, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(ForgeCall::MergeIfHeadIs(request.clone()));
        if let Some(error) = state.failures.take("merge_if_head_is") {
            return Err(error);
        }
        let Some(current) = state.branches.get(&request.branch).cloned() else {
            return Err(unknown_branch("merge_if_head_is", &request.branch));
        };
        if current != request.expected_head {
            return Ok(MergeResult::HeadChanged {
                actual_head: current,
            });
        }
        let merge_commit = state.new_commit();
        state
            .merges
            .push((request.branch.clone(), merge_commit.clone()));
        Ok(MergeResult::Merged { merge_commit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;

    #[test]
    fn resubmitting_keeps_the_change_id_and_reports_the_new_head() {
        let forge = FakeForge::new();
        let first_head = forge.push("agend/T-1/fix");
        let submission = Submission {
            task_id: "T-1".into(),
            branch: "agend/T-1/fix".into(),
            title: "fix".into(),
            body: String::new(),
        };
        let first = block_on(forge.submit(&submission)).unwrap();
        assert_eq!(first.head, first_head);
        let second_head = forge.push("agend/T-1/fix");
        let second = block_on(forge.submit(&submission)).unwrap();
        assert_eq!((second.id, second.head), (first.id, second_head));
        assert_eq!(forge.calls().len(), 2);
    }
}
