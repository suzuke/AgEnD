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

/// A forge without git: commits are generated ids (40 hex digits from a
/// counter) with parents, so ancestry can be read like `git merge-base
/// --is-ancestor`. `push` is the fixture for "someone committed": a new
/// branch starts from the current base. The base branch starts at
/// [`FakeForge::BASE_ROOT`] and moves to each merge commit, whose parents
/// are the base before the merge and the merged head.
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
    base: String,
    /// Parents of every commit except `BASE_ROOT`.
    parents: BTreeMap<String, Vec<String>>,
    next_commit: u64,
    calls: Vec<ForgeCall>,
    failures: Failures,
}

impl State {
    fn new_commit(&mut self, parents: Vec<String>) -> String {
        self.next_commit += 1;
        let commit = format!("{:040x}", self.next_commit);
        self.parents.insert(commit.clone(), parents);
        commit
    }

    fn is_ancestor(&self, ancestor: &str, commit: &str) -> bool {
        let mut todo = vec![commit];
        while let Some(c) = todo.pop() {
            if c == ancestor {
                return true;
            }
            todo.extend(
                self.parents
                    .get(c)
                    .into_iter()
                    .flatten()
                    .map(String::as_str),
            );
        }
        false
    }
}

impl FakeForge {
    /// Base head before any merge (commit number 0; pushes start at 1).
    pub const BASE_ROOT: &'static str = "0000000000000000000000000000000000000000";

    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                branches: BTreeMap::new(),
                changes: BTreeMap::new(),
                merges: Vec::new(),
                base: Self::BASE_ROOT.to_owned(),
                parents: BTreeMap::new(),
                next_commit: 0,
                calls: Vec::new(),
                failures: Failures::new(OPERATIONS),
            }),
        }
    }

    /// Adds a new commit on `branch` (creating it from the base) and returns
    /// the new head.
    pub fn push(&self, branch: &str) -> String {
        let mut state = lock(&self.state);
        let parent = state.branches.get(branch).unwrap_or(&state.base).clone();
        let head = state.new_commit(vec![parent]);
        state.branches.insert(branch.to_owned(), head.clone());
        head
    }

    /// `(branch, merge_commit)` for every successful merge, in order.
    pub fn merges(&self) -> Vec<(String, String)> {
        lock(&self.state).merges.clone()
    }

    /// Head of the base branch: the last merge commit, or [`Self::BASE_ROOT`].
    pub fn base_head(&self) -> String {
        lock(&self.state).base.clone()
    }

    /// Whether `commit` is the base head or one of its ancestors.
    pub fn base_contains(&self, commit: &str) -> bool {
        let state = lock(&self.state);
        state.is_ancestor(commit, &state.base)
    }

    /// Moves the base branch to `commit`, like a force push (scripting:
    /// someone else moved the base). Panics if `commit` does not exist.
    pub fn set_base(&self, commit: &str) {
        let mut state = lock(&self.state);
        assert!(
            commit == Self::BASE_ROOT || state.parents.contains_key(commit),
            "FakeForge::set_base: unknown commit {commit}"
        );
        state.base = commit.to_owned();
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
                id: Some(format!("change-{number}")),
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
        let base = state.base.clone();
        let merge_commit = state.new_commit(vec![base, current]);
        state.base = merge_commit.clone();
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

    #[test]
    fn merges_keep_earlier_merges_and_set_base_can_drop_them() {
        let forge = FakeForge::new();
        let a = forge.push("a");
        let b = forge.push("b");
        for (branch, head) in [("a", &a), ("b", &b)] {
            let request = MergeRequest {
                branch: branch.into(),
                expected_head: head.clone(),
            };
            block_on(forge.merge_if_head_is(&request)).unwrap();
        }
        assert!(forge.base_contains(&a) && forge.base_contains(&b));
        assert!(forge.base_contains(FakeForge::BASE_ROOT));
        let unmerged = forge.push("c");
        assert!(!forge.base_contains(&unmerged));
        forge.set_base(&b);
        assert!(!forge.base_contains(&a));
    }
}
