//! Task relations and operations that are not workflow stages (D15, D18,
//! D21). A task pins the workflow version from its creation time.
//!
//! Must NOT: resolve identities or read storage.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Open,
    Running,
    Blocked,
    Done,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub team_id: String,
    pub workflow_id: String,
    pub workflow_version: u64,
    pub parent: Option<String>,
    pub depends_on: Vec<String>,
    pub superseded_by: Option<String>,
    pub assignee: Option<String>,
    pub status: TaskStatus,
    pub requires_repo: bool,
    pub merge_commit: Option<String>,
}

impl Task {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        team_id: impl Into<String>,
        workflow_id: impl Into<String>,
        workflow_version: u64,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            team_id: team_id.into(),
            workflow_id: workflow_id.into(),
            workflow_version,
            parent: None,
            depends_on: Vec::new(),
            superseded_by: None,
            assignee: None,
            status: TaskStatus::Open,
            requires_repo: false,
            merge_commit: None,
        }
    }

    pub fn set_requires_repo(mut self, requires_repo: bool) -> Self {
        self.requires_repo = requires_repo;
        self
    }

    pub fn apply(&self, operation: TaskOperation) -> Result<Self, TaskError> {
        let mut next = self.clone();
        match operation {
            TaskOperation::Reassign { assignee } => {
                if matches!(self.status, TaskStatus::Done | TaskStatus::Superseded) {
                    return Err(TaskError::Closed);
                }
                next.assignee = Some(assignee);
                next.status = TaskStatus::Running;
            }
            TaskOperation::Reopen => {
                if self.status == TaskStatus::Superseded {
                    return Err(TaskError::Closed);
                }
                if self.status != TaskStatus::Done {
                    return Err(TaskError::NotDone);
                }
                next.status = TaskStatus::Open;
                next.merge_commit = None;
            }
            TaskOperation::Block => {
                if matches!(self.status, TaskStatus::Done | TaskStatus::Superseded) {
                    return Err(TaskError::Closed);
                }
                next.status = TaskStatus::Blocked;
            }
            TaskOperation::Unblock => {
                if self.status != TaskStatus::Blocked {
                    return Err(TaskError::NotBlocked);
                }
                next.status = if self.assignee.is_some() {
                    TaskStatus::Running
                } else {
                    TaskStatus::Open
                };
            }
            TaskOperation::Complete { merge_commit } => {
                if matches!(self.status, TaskStatus::Done | TaskStatus::Superseded) {
                    return Err(TaskError::Closed);
                }
                if self.requires_repo && merge_commit.is_none() {
                    return Err(TaskError::MissingMergeRecord);
                }
                next.status = TaskStatus::Done;
                next.merge_commit = merge_commit;
            }
            TaskOperation::Supersede { new_task_id } => {
                if matches!(self.status, TaskStatus::Done | TaskStatus::Superseded) {
                    return Err(TaskError::Closed);
                }
                if new_task_id == self.id {
                    return Err(TaskError::SelfSupersede);
                }
                next.status = TaskStatus::Superseded;
                next.superseded_by = Some(new_task_id);
            }
            TaskOperation::SetRelations { parent, depends_on } => {
                if parent.as_deref() == Some(&self.id) || depends_on.contains(&self.id) {
                    return Err(TaskError::SelfDependency);
                }
                next.parent = parent;
                next.depends_on = depends_on;
            }
        }
        Ok(next)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOperation {
    Reassign {
        assignee: String,
    },
    Reopen,
    Block,
    Unblock,
    Complete {
        merge_commit: Option<String>,
    },
    Supersede {
        new_task_id: String,
    },
    SetRelations {
        parent: Option<String>,
        depends_on: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskError {
    Closed,
    NotDone,
    NotBlocked,
    SelfSupersede,
    SelfDependency,
    MissingMergeRecord,
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("a closed task cannot be changed"),
            Self::NotDone => f.write_str("only a done task can be reopened"),
            Self::NotBlocked => f.write_str("only a blocked task can be unblocked"),
            Self::SelfSupersede => f.write_str("a task cannot supersede itself"),
            Self::SelfDependency => f.write_str("a task cannot depend on itself"),
            Self::MissingMergeRecord => {
                f.write_str("a repository task can only complete with a recorded merge commit")
            }
        }
    }
}

impl core::error::Error for TaskError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Task {
        Task::new("T-1", "Add task model", "general", "code", 3)
    }

    #[test]
    fn task_keeps_its_created_workflow_version() {
        let task = task();
        assert_eq!(task.workflow_id, "code");
        assert_eq!(task.workflow_version, 3);
    }

    #[test]
    fn reopen_is_only_allowed_after_completion() {
        assert_eq!(task().apply(TaskOperation::Reopen), Err(TaskError::NotDone));
        let done = task()
            .apply(TaskOperation::Complete { merge_commit: None })
            .unwrap();
        let reopened = done.apply(TaskOperation::Reopen).unwrap();
        assert_eq!(reopened.status, TaskStatus::Open);
        assert_eq!(reopened.merge_commit, None);
    }

    #[test]
    fn repository_task_requires_a_merge_commit_to_complete() {
        let task = task().set_requires_repo(true);
        assert_eq!(
            task.apply(TaskOperation::Complete { merge_commit: None }),
            Err(TaskError::MissingMergeRecord)
        );
        let done = task
            .apply(TaskOperation::Complete {
                merge_commit: Some("M-1".into()),
            })
            .unwrap();
        assert_eq!(done.status, TaskStatus::Done);
        assert_eq!(done.merge_commit.as_deref(), Some("M-1"));
    }

    #[test]
    fn relation_updates_reject_self_dependency() {
        let result = task().apply(TaskOperation::SetRelations {
            parent: None,
            depends_on: alloc::vec!["T-1".into()],
        });
        assert_eq!(result, Err(TaskError::SelfDependency));
    }

    #[test]
    fn supersede_records_the_successor_and_closes_the_old_task() {
        let superseded = task()
            .apply(TaskOperation::Supersede {
                new_task_id: "T-2".into(),
            })
            .unwrap();
        assert_eq!(superseded.status, TaskStatus::Superseded);
        assert_eq!(superseded.superseded_by.as_deref(), Some("T-2"));
        assert_eq!(
            superseded.apply(TaskOperation::Reopen),
            Err(TaskError::Closed)
        );
    }
}
