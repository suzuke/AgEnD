//! Workflow definitions (TOML; the DB is the source of truth, each save is a
//! new version) and the save-time checks of D19. TOML parsing and persistence
//! are performed by adapters; this module validates typed values.
//!
//! Must NOT: parse from disk or touch the DB.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use serde::{Deserialize, Serialize};

use super::stage::{FanoutJoin, StageKind};

pub const DEFAULT_STAGE_TIMEOUT_MS: u64 = 300_000;

/// Values for command placeholders. `pr` is the change id returned by the
/// submit stage (for example a pull request number); `None` when the forge
/// returned none. `head` and `branch` come from the branch work product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandContext<'a> {
    pub pr: Option<&'a str>,
    pub head: Option<&'a str>,
    pub branch: Option<&'a str>,
}

/// The placeholders a command template may use, with their names.
const PLACEHOLDERS: [(&str, &str); 3] =
    [("{pr}", "pr"), ("{head}", "head"), ("{branch}", "branch")];

fn placeholder_at(rest: &str) -> Option<(&'static str, &'static str)> {
    PLACEHOLDERS
        .into_iter()
        .find(|(placeholder, _)| rest.starts_with(placeholder))
}

/// Names of the placeholders a command template uses, in first-use order.
/// `Err(name)` when a placeholder is written inside shell quotes or after a
/// backslash: expansion already emits each value as a single-quoted word, so
/// a quoted placeholder would put literal quote characters into the value.
pub fn command_placeholders(template: &str) -> Result<Vec<&'static str>, &'static str> {
    #[derive(PartialEq)]
    enum Quote {
        None,
        Single,
        Double,
    }
    let mut quote = Quote::None;
    let mut used = Vec::new();
    let mut offset = 0;
    while offset < template.len() {
        let rest = &template[offset..];
        if let Some((placeholder, name)) = placeholder_at(rest) {
            if quote != Quote::None {
                return Err(name);
            }
            if !used.contains(&name) {
                used.push(name);
            }
            offset += placeholder.len();
            continue;
        }
        let character = rest.chars().next().expect("offset is on a char boundary");
        offset += character.len_utf8();
        match (&quote, character) {
            (Quote::Single, '\'') | (Quote::Double, '"') => quote = Quote::None,
            (Quote::None, '\'') => quote = Quote::Single,
            (Quote::None, '"') => quote = Quote::Double,
            (Quote::None | Quote::Double, '\\') => {
                if let Some((_, name)) = placeholder_at(&template[offset..]) {
                    return Err(name);
                }
                if let Some(escaped) = template[offset..].chars().next() {
                    offset += escaped.len_utf8();
                }
            }
            _ => {}
        }
    }
    Ok(used)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTemplateError {
    MissingValue(&'static str),
}

impl fmt::Display for CommandTemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue(name) => write!(f, "command placeholder `{{{name}}}` has no value"),
        }
    }
}

impl core::error::Error for CommandTemplateError {}

/// Expand placeholders as single-quoted POSIX shell words. Workflow authors
/// must leave placeholders unquoted; [`Workflow::validate`] rejects quoted ones
/// (see [`command_placeholders`]). Other braces stay untouched so shell syntax
/// such as `awk '{print $1}'` remains valid.
pub fn expand_command_placeholders(
    template: &str,
    context: CommandContext<'_>,
) -> Result<String, CommandTemplateError> {
    let mut expanded = String::new();
    let mut offset = 0;
    while offset < template.len() {
        let rest = &template[offset..];
        if let Some((placeholder, name)) = placeholder_at(rest) {
            let value = match name {
                "pr" => context.pr,
                "head" => context.head,
                _ => context.branch,
            };
            let value = value.ok_or(CommandTemplateError::MissingValue(name))?;
            expanded.push('\'');
            for character in value.chars() {
                if character == '\'' {
                    expanded.push_str("'\\''");
                } else {
                    expanded.push(character);
                }
            }
            expanded.push('\'');
            offset += placeholder.len();
        } else {
            let character = rest.chars().next().expect("offset is on a char boundary");
            expanded.push(character);
            offset += character.len_utf8();
        }
    }
    Ok(expanded)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRequirement {
    Repo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkOutput {
    Branch,
    Result,
    Plan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approver {
    Human,
    Role(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FanoutSource {
    WorkOutput,
    Listed(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutAction {
    Notify,
    Reassign,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stage {
    Work {
        role: String,
        instructions: String,
        output: WorkOutput,
    },
    Command {
        command: String,
    },
    Approval {
        by: Approver,
        count: u8,
        bind_head: bool,
    },
    Submit {
        forge: String,
    },
    Merge,
    Fanout {
        source: FanoutSource,
        join: FanoutJoin,
    },
}

impl Stage {
    pub const fn kind(&self) -> StageKind {
        match self {
            Self::Work { .. } => StageKind::Work,
            Self::Command { .. } => StageKind::Command,
            Self::Approval { .. } => StageKind::Approval,
            Self::Submit { .. } => StageKind::Submit,
            Self::Merge => StageKind::Merge,
            Self::Fanout { .. } => StageKind::Fanout,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStage {
    pub id: String,
    pub stage: Stage,
    pub timeout_ms: Option<u64>,
    pub on_timeout: Option<TimeoutAction>,
    pub on_fail: Option<String>,
}

impl WorkflowStage {
    pub fn new(id: impl Into<String>, stage: Stage) -> Self {
        Self {
            id: id.into(),
            stage,
            timeout_ms: None,
            on_timeout: None,
            on_fail: None,
        }
    }

    pub fn effective_timeout_ms(&self) -> u64 {
        self.timeout_ms.unwrap_or(DEFAULT_STAGE_TIMEOUT_MS)
    }

    pub const fn effective_timeout_action(&self) -> TimeoutAction {
        match self.on_timeout {
            Some(action) => action,
            None => TimeoutAction::Notify,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub version: u64,
    pub requires: Vec<WorkflowRequirement>,
    pub allow_unreviewed: bool,
    pub stages: Vec<WorkflowStage>,
}

impl Workflow {
    pub fn requires_repo(&self) -> bool {
        self.requires.contains(&WorkflowRequirement::Repo)
    }

    /// Validate rules that need the team role list at workflow save time.
    pub fn validate(&self, team_roles: &[String]) -> Result<(), Vec<WorkflowError>> {
        let mut errors = Vec::new();
        let mut ids: Vec<&str> = Vec::new();

        for (index, stage) in self.stages.iter().enumerate() {
            if ids.contains(&stage.id.as_str()) {
                errors.push(WorkflowError::DuplicateStageId(stage.id.clone()));
            }
            ids.push(&stage.id);

            if stage.timeout_ms.is_some_and(|timeout| timeout == 0) {
                errors.push(WorkflowError::ZeroTimeout {
                    stage_id: stage.id.clone(),
                });
            }

            if let Some(target) = &stage.on_fail {
                match self
                    .stages
                    .iter()
                    .position(|candidate| candidate.id == *target)
                {
                    // Failures go back to an earlier work stage: its task holder
                    // gets the reason (D33).
                    Some(target_index)
                        if target_index < index
                            && self.stages[target_index].stage.kind() == StageKind::Work => {}
                    _ => errors.push(WorkflowError::InvalidFailureTarget {
                        stage_id: stage.id.clone(),
                        target: target.clone(),
                    }),
                }
            }

            match &stage.stage {
                Stage::Work { role, .. } => {
                    if !team_roles.iter().any(|candidate| candidate == role) {
                        errors.push(WorkflowError::UnknownRole {
                            stage_id: stage.id.clone(),
                            role: role.clone(),
                        });
                    }
                }
                Stage::Approval {
                    by: Approver::Role(role),
                    count,
                    ..
                } => {
                    if !team_roles.iter().any(|candidate| candidate == role) {
                        errors.push(WorkflowError::UnknownRole {
                            stage_id: stage.id.clone(),
                            role: role.clone(),
                        });
                    }
                    if *count == 0 {
                        errors.push(WorkflowError::ZeroApprovalCount {
                            stage_id: stage.id.clone(),
                        });
                    }
                }
                Stage::Approval { count, .. } if *count == 0 => {
                    errors.push(WorkflowError::ZeroApprovalCount {
                        stage_id: stage.id.clone(),
                    });
                }
                Stage::Command { command } if command.trim().is_empty() => {
                    errors.push(WorkflowError::InvalidCommand {
                        stage_id: stage.id.clone(),
                    });
                }
                Stage::Command { command } => match command_placeholders(command) {
                    Err(placeholder) => errors.push(WorkflowError::QuotedPlaceholder {
                        stage_id: stage.id.clone(),
                        placeholder: placeholder.into(),
                    }),
                    Ok(used) => {
                        let earlier = &self.stages[..index];
                        for placeholder in used {
                            let has_source = if placeholder == "pr" {
                                earlier
                                    .iter()
                                    .any(|stage| stage.stage.kind() == StageKind::Submit)
                            } else {
                                earlier.iter().any(|stage| {
                                    matches!(
                                        stage.stage,
                                        Stage::Work {
                                            output: WorkOutput::Branch,
                                            ..
                                        }
                                    )
                                })
                            };
                            if !has_source {
                                errors.push(WorkflowError::PlaceholderWithoutSource {
                                    stage_id: stage.id.clone(),
                                    placeholder: placeholder.into(),
                                });
                            }
                        }
                    }
                },
                _ => {}
            }
        }

        let first_submit = self
            .stages
            .iter()
            .position(|stage| stage.stage.kind() == StageKind::Submit);
        if let Some(submit_index) = first_submit {
            let has_branch_work = self.stages[..submit_index].iter().any(|stage| {
                matches!(
                    stage.stage,
                    Stage::Work {
                        output: WorkOutput::Branch,
                        ..
                    }
                )
            });
            if !has_branch_work {
                errors.push(WorkflowError::SubmitWithoutBranchWork);
            }
        }

        let has_repo_stage = self
            .stages
            .iter()
            .any(|stage| matches!(stage.stage.kind(), StageKind::Submit | StageKind::Merge));
        if has_repo_stage && !self.requires_repo() {
            errors.push(WorkflowError::RepoRequired);
        }

        for (merge_index, merge) in self
            .stages
            .iter()
            .enumerate()
            .filter(|(_, stage)| stage.stage.kind() == StageKind::Merge)
        {
            let has_prior_command = self.stages[..merge_index]
                .iter()
                .any(|stage| stage.stage.kind() == StageKind::Command);
            if !has_prior_command {
                errors.push(WorkflowError::MergeWithoutCommand {
                    stage_id: merge.id.clone(),
                });
            }
            if merge_index + 1 != self.stages.len() {
                errors.push(WorkflowError::MergeNotLast {
                    stage_id: merge.id.clone(),
                });
            }
            let has_bound_approval = self.stages[..merge_index].iter().any(|stage| {
                matches!(
                    stage.stage,
                    Stage::Approval {
                        bind_head: true,
                        ..
                    }
                )
            });
            if !has_bound_approval && !self.allow_unreviewed {
                errors.push(WorkflowError::MergeWithoutBoundApproval {
                    stage_id: merge.id.clone(),
                });
            }
        }

        for (index, stage) in self.stages.iter().enumerate() {
            if let Stage::Fanout { source, join } = &stage.stage {
                if let FanoutSource::Listed(children) = source
                    && (children.is_empty()
                        || children.iter().any(|child| child.trim().is_empty())
                        || children
                            .iter()
                            .enumerate()
                            .any(|(child_index, child)| children[..child_index].contains(child)))
                {
                    errors.push(WorkflowError::InvalidFanoutSource {
                        stage_id: stage.id.clone(),
                    });
                }
                if matches!(source, FanoutSource::WorkOutput)
                    && !self.stages[..index].iter().any(|previous| {
                        matches!(
                            previous.stage,
                            Stage::Work {
                                output: WorkOutput::Plan,
                                ..
                            }
                        )
                    })
                {
                    errors.push(WorkflowError::FanoutWithoutPlan {
                        stage_id: stage.id.clone(),
                    });
                }
                if *join == FanoutJoin::Pick
                    && !self.stages[index + 1..]
                        .iter()
                        .any(|following| following.stage.kind() == StageKind::Approval)
                {
                    errors.push(WorkflowError::PickWithoutApproval {
                        stage_id: stage.id.clone(),
                    });
                }
            }
        }

        // A branch work stage produces a new head; a check or head-bound
        // approval before it could never cover the head that is merged.
        if let Some(last_branch_work) = self.stages.iter().rposition(is_branch_work) {
            for stage in self.stages[..last_branch_work].iter().filter(|stage| {
                matches!(
                    stage.stage,
                    Stage::Command { .. }
                        | Stage::Approval {
                            bind_head: true,
                            ..
                        }
                )
            }) {
                errors.push(WorkflowError::HeadBoundBeforeWork {
                    stage_id: stage.id.clone(),
                    work_stage_id: self.stages[last_branch_work].id.clone(),
                });
            }
        }
        // A failed check or requested change returns to an earlier work stage
        // and its task holder (D33), so there must be one.
        for (index, stage) in self.stages.iter().enumerate() {
            if matches!(stage.stage.kind(), StageKind::Command | StageKind::Approval)
                && !self.stages[..index]
                    .iter()
                    .any(|earlier| earlier.stage.kind() == StageKind::Work)
            {
                errors.push(WorkflowError::NoEarlierWork {
                    stage_id: stage.id.clone(),
                });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Validate and wrap: only a validated workflow can start a pipeline.
    pub fn validated(self, team_roles: &[String]) -> Result<ValidatedWorkflow, Vec<WorkflowError>> {
        self.validate(team_roles)?;
        Ok(ValidatedWorkflow(self))
    }

    pub fn builtin_code() -> Self {
        Self {
            id: "code".to_string(),
            version: 1,
            requires: alloc::vec![WorkflowRequirement::Repo],
            allow_unreviewed: false,
            stages: alloc::vec![
                WorkflowStage::new(
                    "work",
                    Stage::Work {
                        role: "dev".to_string(),
                        instructions: String::new(),
                        output: WorkOutput::Branch,
                    },
                ),
                WorkflowStage::new(
                    "submit",
                    Stage::Submit {
                        forge: "local".to_string(),
                    },
                ),
                WorkflowStage {
                    id: "checks".to_string(),
                    stage: Stage::Command {
                        command: "cargo test".to_string(),
                    },
                    timeout_ms: Some(300_000),
                    on_timeout: None,
                    on_fail: None,
                },
                WorkflowStage::new(
                    "review",
                    Stage::Approval {
                        by: Approver::Role("reviewer".to_string()),
                        count: 1,
                        bind_head: true,
                    },
                ),
                WorkflowStage::new("merge", Stage::Merge),
            ],
        }
    }

    pub fn builtin_research() -> Self {
        Self {
            id: "research".to_string(),
            version: 1,
            requires: Vec::new(),
            allow_unreviewed: false,
            stages: alloc::vec![
                WorkflowStage::new(
                    "work",
                    Stage::Work {
                        role: "researcher".to_string(),
                        instructions: String::new(),
                        output: WorkOutput::Result,
                    },
                ),
                WorkflowStage::new(
                    "review",
                    Stage::Approval {
                        by: Approver::Role("reviewer".to_string()),
                        count: 1,
                        bind_head: false,
                    },
                ),
            ],
        }
    }

    /// D34: a human reviews the plan (cheapest to correct); agents review
    /// and check the implementation.
    pub fn builtin_planned() -> Self {
        Self {
            id: "planned".to_string(),
            version: 1,
            requires: alloc::vec![WorkflowRequirement::Repo],
            allow_unreviewed: false,
            stages: alloc::vec![
                WorkflowStage::new(
                    "plan",
                    Stage::Work {
                        role: "planner".to_string(),
                        instructions: String::new(),
                        output: WorkOutput::Result,
                    },
                ),
                WorkflowStage::new(
                    "plan_review",
                    Stage::Approval {
                        by: Approver::Human,
                        count: 1,
                        bind_head: false,
                    },
                ),
                WorkflowStage::new(
                    "work",
                    Stage::Work {
                        role: "dev".to_string(),
                        instructions: String::new(),
                        output: WorkOutput::Branch,
                    },
                ),
                WorkflowStage::new(
                    "submit",
                    Stage::Submit {
                        forge: "local".to_string(),
                    },
                ),
                WorkflowStage {
                    id: "checks".to_string(),
                    stage: Stage::Command {
                        command: "cargo test".to_string(),
                    },
                    timeout_ms: Some(300_000),
                    on_timeout: None,
                    on_fail: None,
                },
                WorkflowStage::new(
                    "review",
                    Stage::Approval {
                        by: Approver::Role("reviewer".to_string()),
                        count: 1,
                        bind_head: true,
                    },
                ),
                WorkflowStage::new("merge", Stage::Merge),
            ],
        }
    }

    pub fn builtin_epic() -> Self {
        Self {
            id: "epic".to_string(),
            version: 1,
            requires: Vec::new(),
            allow_unreviewed: false,
            stages: alloc::vec![
                WorkflowStage::new(
                    "plan",
                    Stage::Work {
                        role: "planner".to_string(),
                        instructions: String::new(),
                        output: WorkOutput::Plan,
                    },
                ),
                WorkflowStage::new(
                    "fanout",
                    Stage::Fanout {
                        source: FanoutSource::WorkOutput,
                        join: FanoutJoin::All,
                    },
                ),
                WorkflowStage::new(
                    "review",
                    Stage::Approval {
                        by: Approver::Human,
                        count: 1,
                        bind_head: false,
                    },
                ),
            ],
        }
    }
}

fn is_branch_work(stage: &WorkflowStage) -> bool {
    matches!(
        stage.stage,
        Stage::Work {
            output: WorkOutput::Branch,
            ..
        }
    )
}

/// A workflow that passed [`Workflow::validate`]; the only input a pipeline
/// accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedWorkflow(Workflow);

impl ValidatedWorkflow {
    pub fn workflow(&self) -> &Workflow {
        &self.0
    }

    pub fn into_inner(self) -> Workflow {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    DuplicateStageId(String),
    ZeroTimeout {
        stage_id: String,
    },
    InvalidFailureTarget {
        stage_id: String,
        target: String,
    },
    UnknownRole {
        stage_id: String,
        role: String,
    },
    ZeroApprovalCount {
        stage_id: String,
    },
    InvalidCommand {
        stage_id: String,
    },
    QuotedPlaceholder {
        stage_id: String,
        placeholder: String,
    },
    PlaceholderWithoutSource {
        stage_id: String,
        placeholder: String,
    },
    HeadBoundBeforeWork {
        stage_id: String,
        work_stage_id: String,
    },
    MergeNotLast {
        stage_id: String,
    },
    NoEarlierWork {
        stage_id: String,
    },
    SubmitWithoutBranchWork,
    RepoRequired,
    MergeWithoutBoundApproval {
        stage_id: String,
    },
    MergeWithoutCommand {
        stage_id: String,
    },
    FanoutWithoutPlan {
        stage_id: String,
    },
    InvalidFanoutSource {
        stage_id: String,
    },
    PickWithoutApproval {
        stage_id: String,
    },
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateStageId(id) => write!(f, "duplicate stage id `{id}`"),
            Self::ZeroTimeout { stage_id } => {
                write!(f, "stage `{stage_id}` timeout must be greater than zero")
            }
            Self::InvalidFailureTarget { stage_id, target } => write!(
                f,
                "stage `{stage_id}` on_fail must point to an earlier work stage; got `{target}`"
            ),
            Self::UnknownRole { stage_id, role } => {
                write!(f, "stage `{stage_id}` refers to unknown role `{role}`")
            }
            Self::ZeroApprovalCount { stage_id } => {
                write!(
                    f,
                    "stage `{stage_id}` approval count must be greater than zero"
                )
            }
            Self::InvalidCommand { stage_id } => {
                write!(f, "stage `{stage_id}` command must not be empty")
            }
            Self::QuotedPlaceholder {
                stage_id,
                placeholder,
            } => write!(
                f,
                "stage `{stage_id}` writes `{{{placeholder}}}` inside quotes or after a backslash; leave placeholders unquoted, expansion quotes the value"
            ),
            Self::PlaceholderWithoutSource {
                stage_id,
                placeholder,
            } => write!(
                f,
                "stage `{stage_id}` uses `{{{placeholder}}}` but no earlier stage provides it (`pr` needs a submit stage; `head` and `branch` need a work stage with branch output)"
            ),
            Self::HeadBoundBeforeWork {
                stage_id,
                work_stage_id,
            } => write!(
                f,
                "stage `{stage_id}` checks or approves a head before work stage `{work_stage_id}` produces the final one; move it after that work stage"
            ),
            Self::MergeNotLast { stage_id } => {
                write!(f, "merge stage `{stage_id}` must be the last stage")
            }
            Self::NoEarlierWork { stage_id } => write!(
                f,
                "stage `{stage_id}` needs an earlier work stage to return to when it fails or changes are requested"
            ),
            Self::SubmitWithoutBranchWork => {
                f.write_str("submit requires an earlier work stage that produces a branch")
            }
            Self::RepoRequired => f.write_str("submit and merge require `requires = [\"repo\"]`"),
            Self::MergeWithoutBoundApproval { stage_id } => write!(
                f,
                "merge stage `{stage_id}` requires an earlier head-bound approval or allow_unreviewed = true"
            ),
            Self::MergeWithoutCommand { stage_id } => write!(
                f,
                "merge stage `{stage_id}` requires at least one earlier command check"
            ),
            Self::FanoutWithoutPlan { stage_id } => write!(
                f,
                "fanout stage `{stage_id}` with work_output source requires an earlier plan work stage"
            ),
            Self::InvalidFanoutSource { stage_id } => write!(
                f,
                "fanout stage `{stage_id}` listed source must contain unique, non-empty child names"
            ),
            Self::PickWithoutApproval { stage_id } => write!(
                f,
                "fanout stage `{stage_id}` with pick join requires a following approval stage"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn roles() -> Vec<String> {
        ["dev", "reviewer", "researcher", "planner"]
            .into_iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn built_in_code_workflow_satisfies_save_rules() {
        let workflow = Workflow::builtin_code();
        assert!(workflow.requires_repo());
        assert_eq!(
            workflow
                .stages
                .iter()
                .map(|stage| stage.stage.kind())
                .collect::<Vec<_>>(),
            [
                StageKind::Work,
                StageKind::Submit,
                StageKind::Command,
                StageKind::Approval,
                StageKind::Merge,
            ]
        );
        assert_eq!(workflow.validate(&roles()), Ok(()));
    }

    #[test]
    fn built_in_workflows_match_their_repo_requirements() {
        assert_eq!(Workflow::builtin_research().validate(&roles()), Ok(()));
        assert_eq!(Workflow::builtin_epic().validate(&roles()), Ok(()));
    }

    #[test]
    fn save_rules_report_invalid_merge_and_forward_failure_target() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[0].on_fail = Some("merge".to_string());
        if let Stage::Approval { bind_head, .. } = &mut workflow.stages[3].stage {
            *bind_head = false;
        }
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, WorkflowError::InvalidFailureTarget { .. }))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, WorkflowError::MergeWithoutBoundApproval { .. }))
        );
    }

    #[test]
    fn command_as_a_stage_is_not_a_checks_kind() {
        let workflow = Workflow::builtin_code();
        assert_eq!(workflow.stages[2].stage.kind(), StageKind::Command);
        assert_eq!(StageKind::parse("checks"), None);
    }

    #[test]
    fn command_placeholders_expand_without_changing_other_braces() {
        let command = "gh pr checks {pr} --head {head} --branch {branch}";
        assert_eq!(
            expand_command_placeholders(
                command,
                CommandContext {
                    pr: Some("42"),
                    head: Some("abc123"),
                    branch: Some("agend/T-1/demo"),
                }
            ),
            Ok("gh pr checks '42' --head 'abc123' --branch 'agend/T-1/demo'".into())
        );
        assert_eq!(
            expand_command_placeholders(
                "awk '{print $1}' {head}",
                CommandContext {
                    pr: None,
                    head: Some("abc123"),
                    branch: Some("main"),
                }
            ),
            Ok("awk '{print $1}' 'abc123'".into())
        );
    }

    #[test]
    fn command_placeholder_requires_a_pull_request_value_when_used() {
        assert_eq!(
            expand_command_placeholders(
                "gh pr checks {pr}",
                CommandContext {
                    pr: None,
                    head: Some("abc123"),
                    branch: Some("main"),
                }
            ),
            Err(CommandTemplateError::MissingValue("pr"))
        );
    }

    #[test]
    fn placeholder_values_are_not_recursively_expanded_or_shell_interpreted() {
        assert_eq!(
            expand_command_placeholders(
                "echo {pr}",
                CommandContext {
                    pr: Some("{head}"),
                    head: Some("abc123"),
                    branch: Some("main"),
                }
            ),
            Ok("echo '{head}'".into())
        );
        assert_eq!(
            expand_command_placeholders(
                "echo {pr}",
                CommandContext {
                    pr: Some("42'; touch /tmp/not-run; echo '"),
                    head: Some("abc123"),
                    branch: Some("main"),
                }
            ),
            Ok("echo '42'\\''; touch /tmp/not-run; echo '\\'''".into())
        );
    }

    #[test]
    fn workflow_save_rules_reject_merge_without_command_and_invalid_fanout_shapes() {
        let mut workflow = Workflow::builtin_code();
        workflow
            .stages
            .retain(|stage| stage.stage.kind() != StageKind::Command);
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, WorkflowError::MergeWithoutCommand { .. }))
        );

        let mut workflow = Workflow::builtin_epic();
        workflow.stages[1].stage = Stage::Fanout {
            source: FanoutSource::Listed(vec!["a".into()]),
            join: FanoutJoin::Pick,
        };
        workflow.stages.pop();
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, WorkflowError::PickWithoutApproval { .. }))
        );

        let mut workflow = Workflow::builtin_epic();
        workflow.stages[1].stage = Stage::Fanout {
            source: FanoutSource::Listed(vec!["child".into(), "child".into()]),
            join: FanoutJoin::All,
        };
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            WorkflowError::InvalidFanoutSource { stage_id } if stage_id == "fanout"
        )));
    }

    fn with_checks_command(command: &str) -> Workflow {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[2].stage = Stage::Command {
            command: command.into(),
        };
        workflow
    }

    /// Round-2 review N7: a quoted placeholder would expand to a value with
    /// literal quotes (`"{head}"` -> `"'H'"`), so saving must reject it.
    #[test]
    fn review_n7_quoted_placeholders_are_rejected_at_save_time() {
        for command in [
            "git show \"{head}\"",
            "git show '{head}'",
            "echo \"branch={branch}\"",
            "gh pr view \\{pr}",
        ] {
            let errors = with_checks_command(command).validate(&roles()).unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|error| matches!(error, WorkflowError::QuotedPlaceholder { .. })),
                "{command}: {errors:?}"
            );
        }
        for command in [
            "gh pr checks {pr} --watch",
            "git show {head} -- \"src\"",
            "awk '{print $1}' {branch}",
        ] {
            assert_eq!(
                with_checks_command(command).validate(&roles()),
                Ok(()),
                "{command}"
            );
        }
        assert_eq!(
            command_placeholders("a {head} {pr} {head}"),
            Ok(vec!["head", "pr"])
        );
    }

    #[test]
    fn placeholders_need_an_earlier_stage_that_provides_them() {
        let mut workflow = Workflow::builtin_code();
        let checks = workflow.stages.remove(2);
        workflow.stages.insert(1, checks);
        workflow.stages[1].stage = Stage::Command {
            command: "gh pr checks {pr}".into(),
        };
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            WorkflowError::PlaceholderWithoutSource { placeholder, .. } if placeholder == "pr"
        )));

        let mut workflow = Workflow::builtin_research();
        workflow.stages.insert(
            1,
            WorkflowStage::new(
                "show",
                Stage::Command {
                    command: "git show {head}".into(),
                },
            ),
        );
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            WorkflowError::PlaceholderWithoutSource { placeholder, .. } if placeholder == "head"
        )));
    }

    fn branch_work(id: &str) -> WorkflowStage {
        WorkflowStage::new(
            id,
            Stage::Work {
                role: "dev".into(),
                instructions: String::new(),
                output: WorkOutput::Branch,
            },
        )
    }

    fn reviewer_approval(id: &str, bind_head: bool) -> WorkflowStage {
        WorkflowStage::new(
            id,
            Stage::Approval {
                by: Approver::Role("reviewer".into()),
                count: 1,
                bind_head,
            },
        )
    }

    fn has(errors: &[WorkflowError], check: impl Fn(&WorkflowError) -> bool) -> bool {
        errors.iter().any(check)
    }

    /// Verifier I1: a command or head-bound approval before a later branch
    /// work stage could never cover the merged head, so the task would stall.
    #[test]
    fn verifier_i1_head_bound_stage_before_a_later_branch_work_is_rejected() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages.insert(3, branch_work("fixup"));
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(has(&errors, |error| matches!(
            error,
            WorkflowError::HeadBoundBeforeWork { stage_id, work_stage_id }
                if stage_id == "checks" && work_stage_id == "fixup"
        )));

        let mut workflow = Workflow::builtin_code();
        workflow.stages.insert(1, reviewer_approval("design", true));
        workflow.stages.insert(2, branch_work("implement"));
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(has(&errors, |error| matches!(
            error,
            WorkflowError::HeadBoundBeforeWork { stage_id, .. } if stage_id == "design"
        )));

        // An unbound approval before the branch work (plan review) is fine.
        workflow.stages[1] = reviewer_approval("design", false);
        assert_eq!(workflow.validate(&roles()), Ok(()));
    }

    /// Verifier B1: a stage after merge would never run.
    #[test]
    fn verifier_b1_merge_must_be_the_last_stage() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages.push(WorkflowStage::new(
            "post",
            Stage::Command {
                command: "run post".into(),
            },
        ));
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(has(&errors, |error| matches!(
            error,
            WorkflowError::MergeNotLast { stage_id } if stage_id == "merge"
        )));
    }

    /// Verifier I2: on_fail may only target an earlier work stage, so the
    /// reason always reaches the task holder.
    #[test]
    fn verifier_i2_on_fail_must_target_an_earlier_work_stage() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[3].on_fail = Some("submit".into());
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(has(&errors, |error| matches!(
            error,
            WorkflowError::InvalidFailureTarget { target, .. } if target == "submit"
        )));
        workflow.stages[3].on_fail = Some("work".into());
        assert_eq!(workflow.validate(&roles()), Ok(()));
    }

    /// Verifier I2: a command or approval with no earlier work stage has
    /// nobody to return to (changes requested would fail the task).
    #[test]
    fn verifier_i2_command_and_approval_need_an_earlier_work_stage() {
        let workflow = Workflow {
            id: "gate-first".into(),
            version: 1,
            requires: Vec::new(),
            allow_unreviewed: false,
            stages: vec![
                WorkflowStage::new(
                    "gate",
                    Stage::Approval {
                        by: Approver::Human,
                        count: 1,
                        bind_head: false,
                    },
                ),
                WorkflowStage::new(
                    "work",
                    Stage::Work {
                        role: "dev".into(),
                        instructions: String::new(),
                        output: WorkOutput::Result,
                    },
                ),
            ],
        };
        let errors = workflow.validate(&roles()).unwrap_err();
        assert!(has(&errors, |error| matches!(
            error,
            WorkflowError::NoEarlierWork { stage_id } if stage_id == "gate"
        )));
    }

    #[test]
    fn built_in_planned_workflow_reviews_the_plan_with_a_human_first() {
        let workflow = Workflow::builtin_planned();
        assert_eq!(workflow.validate(&roles()), Ok(()));
        assert!(matches!(
            workflow.stages[1].stage,
            Stage::Approval {
                by: Approver::Human,
                bind_head: false,
                ..
            }
        ));
        assert!(matches!(
            workflow.stages[5].stage,
            Stage::Approval {
                by: Approver::Role(_),
                bind_head: true,
                ..
            }
        ));
        assert!(workflow.clone().validated(&roles()).is_ok());
        assert!(Workflow::builtin_code().validated(&[]).is_err());
    }

    #[test]
    fn invalid_command_message_does_not_mention_timeout() {
        let message = WorkflowError::InvalidCommand {
            stage_id: "checks".into(),
        }
        .to_string();
        assert_eq!(message, "stage `checks` command must not be empty");
    }
}
