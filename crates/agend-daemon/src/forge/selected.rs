//! The workflow's explicit forge choice; unknown names never fall back to local.
use super::{github::GithubForge, local::LocalForge};
use agend_core::{
    pipeline::ports::ExecutionError,
    traits::{Forge, MergeRequest, MergeResult, Submission, SubmittedChange},
};

pub enum SelectedForge {
    Local(LocalForge),
    Github(GithubForge),
    Unavailable(String),
}
impl SelectedForge {
    pub async fn find_merge(
        &self,
        task: &str,
        head: &str,
    ) -> Result<Option<(String, bool)>, String> {
        match self {
            Self::Local(forge) => forge.find_merge(task, head).await,
            Self::Github(forge) => forge.find_merge(task, head).await,
            Self::Unavailable(reason) => Err(reason.clone()),
        }
    }
}
impl Forge for SelectedForge {
    type Error = ExecutionError;
    async fn submit(&self, request: &Submission) -> Result<SubmittedChange, ExecutionError> {
        match self {
            Self::Local(forge) => forge.submit(request).await,
            Self::Github(forge) => forge.submit(request).await,
            Self::Unavailable(reason) => Err(ExecutionError::Blocked(reason.clone())),
        }
    }
    async fn head(&self, branch: &str) -> Result<String, ExecutionError> {
        match self {
            Self::Local(forge) => forge.head(branch).await,
            Self::Github(forge) => forge.head(branch).await,
            Self::Unavailable(reason) => Err(ExecutionError::Blocked(reason.clone())),
        }
    }
    async fn merge_if_head_is(
        &self,
        request: &MergeRequest,
    ) -> Result<MergeResult, ExecutionError> {
        match self {
            Self::Local(forge) => forge.merge_if_head_is(request).await,
            Self::Github(forge) => forge.merge_if_head_is(request).await,
            Self::Unavailable(reason) => Err(ExecutionError::Blocked(reason.clone())),
        }
    }
}
