//! Pure workflow execution state machine (D11). Callers provide observed
//! events; returned actions describe side effects for the daemon to perform.
//!
//! Must NOT: call a trait, execute a command, or access storage.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use super::stage::{FanoutJoin, StageKind};
use super::workflow::{FanoutSource, Stage, TimeoutAction, WorkOutput, Workflow, WorkflowStage};
use crate::policy::merge_gate::{
    ApprovalAfterRebase, RebaseOutcome, approval_after_rebase, evaluate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassedCheck {
    pub stage_id: String,
    pub head: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    pub stage_id: String,
    pub head: Option<String>,
    pub patch_id: Option<String>,
    pub reviewers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkProduct {
    Branch {
        branch: String,
        head: String,
        patch_id: String,
    },
    Result {
        summary: String,
        output: Option<String>,
    },
    Plan {
        items: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineState {
    pub task_id: String,
    pub workflow: Workflow,
    pub stage_index: usize,
    pub status: PipelineStatus,
    pub branch: Option<String>,
    pub current_head: Option<String>,
    pub patch_id: Option<String>,
    pub work_product: Option<WorkProduct>,
    pub approval_reviewers: Vec<String>,
    pub approvals: Vec<ApprovalRecord>,
    pub passed_checks: Vec<PassedCheck>,
    pub fanout_child_task_ids: Vec<String>,
    pub selected_fanout_child: Option<String>,
    pub merge_commit: Option<String>,
}

impl PipelineState {
    pub fn new(task_id: impl Into<String>, workflow: Workflow) -> Self {
        Self {
            task_id: task_id.into(),
            workflow,
            stage_index: 0,
            status: PipelineStatus::Pending,
            branch: None,
            current_head: None,
            patch_id: None,
            work_product: None,
            approval_reviewers: Vec::new(),
            approvals: Vec::new(),
            passed_checks: Vec::new(),
            fanout_child_task_ids: Vec::new(),
            selected_fanout_child: None,
            merge_commit: None,
        }
    }

    pub fn current_stage(&self) -> Option<&WorkflowStage> {
        self.workflow.stages.get(self.stage_index)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineEvent {
    Start,
    WorkCompleted {
        product: WorkProduct,
    },
    Submitted,
    CommandFinished {
        stage_id: String,
        head: String,
        exit_code: Option<i32>,
    },
    ApprovalGranted {
        reviewer: String,
        head: Option<String>,
        selected_child: Option<String>,
    },
    CommitCreated {
        head: String,
        patch_id: String,
    },
    MainAdvanced {
        rebased_head: String,
        patch_id: String,
        conflict: bool,
    },
    FanoutCompleted {
        stage_id: String,
        child_task_ids: Vec<String>,
        selected_child: Option<String>,
    },
    StageFailed {
        stage_id: String,
        reason: String,
    },
    StageTimedOut {
        stage_id: String,
    },
    MergeCompleted {
        head: String,
        merge_commit: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineAction {
    AssignWork {
        stage_id: String,
        role: String,
    },
    Submit {
        stage_id: String,
        forge: String,
    },
    RunCommand {
        stage_id: String,
        command: String,
        head: Option<String>,
        branch: Option<String>,
        timeout_ms: u64,
        timeout_action: TimeoutAction,
    },
    RequestApproval {
        stage_id: String,
        bind_head: bool,
        choices: Vec<String>,
    },
    Merge {
        stage_id: String,
        head: String,
    },
    Fanout {
        stage_id: String,
        source: FanoutSource,
        join: FanoutJoin,
        work_product: Option<WorkProduct>,
    },
    CancelFanoutSiblings {
        winner_task_id: String,
        sibling_task_ids: Vec<String>,
    },
    ScheduleTimeout {
        stage_id: String,
        timeout_ms: u64,
        action: TimeoutAction,
    },
    NotifyTimeout {
        stage_id: String,
    },
    ReassignStage {
        stage_id: String,
    },
    TaskCancelled {
        stage_id: String,
    },
    TaskFailed {
        stage_id: String,
        reason: String,
    },
    ReturnToWork,
    TaskDone {
        merge_commit: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    NotPending,
    NotRunning,
    WrongStage,
    MissingHead,
    ApprovalHeadMismatch,
    MergeGateClosed,
    NoFailureTarget,
    EmptyWorkflow,
    StaleResult,
    InvalidWorkProduct,
    InvalidStageIndex,
    InvalidFanoutChildren,
    FanoutSelectionRequired,
    InvalidFanoutSelection,
    UnexpectedFanoutSelection,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotPending => "pipeline has already started",
            Self::NotRunning => "pipeline is not running",
            Self::WrongStage => "event does not match the current stage",
            Self::MissingHead => "the current stage requires a commit head",
            Self::ApprovalHeadMismatch => "approval must match the current head",
            Self::MergeGateClosed => {
                "merge requires passing checks and approval for the current head"
            }
            Self::NoFailureTarget => "the failed stage has no valid failure target",
            Self::EmptyWorkflow => "workflow has no stages",
            Self::StaleResult => "stage result belongs to an outdated stage or head",
            Self::InvalidWorkProduct => "work output does not match the current work stage",
            Self::InvalidStageIndex => "pipeline state has an invalid stage index",
            Self::InvalidFanoutChildren => "fanout completion has no unique child task ids",
            Self::FanoutSelectionRequired => "a fanout winner must be selected for approval",
            Self::InvalidFanoutSelection => "selected task is not a completed fanout child",
            Self::UnexpectedFanoutSelection => "this approval does not select a fanout child",
        })
    }
}

impl core::error::Error for TransitionError {}

/// Apply one observed event and return a new state plus the side effects to run.
pub fn step(
    state: &PipelineState,
    event: PipelineEvent,
) -> Result<(PipelineState, Vec<PipelineAction>), TransitionError> {
    let mut next = state.clone();
    let mut actions = Vec::new();

    match event {
        PipelineEvent::Start => {
            if state.status != PipelineStatus::Pending {
                return Err(TransitionError::NotPending);
            }
            if state.workflow.stages.is_empty() {
                return Err(TransitionError::EmptyWorkflow);
            }
            next.status = PipelineStatus::Running;
            enter_stage(&mut next, 0, &mut actions)?;
        }
        PipelineEvent::WorkCompleted { product } => {
            ensure_running(state)?;
            let Some(WorkflowStage {
                stage: Stage::Work { output, .. },
                ..
            }) = state.current_stage()
            else {
                return Err(TransitionError::WrongStage);
            };
            if !matches!(
                (output, &product),
                (WorkOutput::Branch, WorkProduct::Branch { .. })
                    | (WorkOutput::Result, WorkProduct::Result { .. })
                    | (WorkOutput::Plan, WorkProduct::Plan { .. })
            ) {
                return Err(TransitionError::InvalidWorkProduct);
            }
            if let WorkProduct::Branch {
                branch,
                head,
                patch_id,
            } = &product
            {
                if state.current_head.as_deref() != Some(head) {
                    clear_head_bound_approvals(&mut next);
                    next.passed_checks.clear();
                }
                next.branch = Some(branch.clone());
                next.current_head = Some(head.clone());
                next.patch_id = Some(patch_id.clone());
            }
            if matches!(&product, WorkProduct::Plan { .. }) {
                clear_fanout(&mut next);
            }
            next.work_product = Some(product);
            enter_stage(&mut next, state.stage_index + 1, &mut actions)?;
        }
        PipelineEvent::Submitted => {
            ensure_current_stage(state, StageKind::Submit)?;
            enter_stage(&mut next, state.stage_index + 1, &mut actions)?;
        }
        PipelineEvent::CommandFinished {
            stage_id,
            head,
            exit_code,
        } => {
            ensure_current_stage(state, StageKind::Command)?;
            if state.current_stage().map(|stage| stage.id.as_str()) != Some(stage_id.as_str())
                || state.current_head.as_deref() != Some(head.as_str())
            {
                return Err(TransitionError::StaleResult);
            }
            if exit_code == Some(0) {
                if !next
                    .passed_checks
                    .iter()
                    .any(|check| check.stage_id == stage_id && check.head == head)
                {
                    next.passed_checks.push(PassedCheck { stage_id, head });
                }
                enter_stage(&mut next, state.stage_index + 1, &mut actions)?;
            } else {
                fail_stage(&mut next, state.stage_index, "command failed", &mut actions)?;
            }
        }
        PipelineEvent::ApprovalGranted {
            reviewer,
            head,
            selected_child,
        } => {
            ensure_current_stage(state, StageKind::Approval)?;
            let Some(WorkflowStage {
                stage: Stage::Approval {
                    bind_head, count, ..
                },
                ..
            }) = state.current_stage()
            else {
                return Err(TransitionError::WrongStage);
            };
            if *bind_head && head.as_deref() != state.current_head.as_deref() {
                return Err(TransitionError::ApprovalHeadMismatch);
            }
            if pick_fanout_index_for_approval(&state.workflow, state.stage_index).is_some() {
                let winner = selected_child
                    .or_else(|| next.selected_fanout_child.clone())
                    .ok_or(TransitionError::FanoutSelectionRequired)?;
                if !next.fanout_child_task_ids.contains(&winner) {
                    return Err(TransitionError::InvalidFanoutSelection);
                }
                if next
                    .selected_fanout_child
                    .as_ref()
                    .is_some_and(|selected| selected != &winner)
                {
                    return Err(TransitionError::InvalidFanoutSelection);
                }
                next.selected_fanout_child = Some(winner);
            } else if selected_child.is_some() {
                return Err(TransitionError::UnexpectedFanoutSelection);
            }
            if !next.approval_reviewers.contains(&reviewer) {
                next.approval_reviewers.push(reviewer);
            }
            if next.approval_reviewers.len() < usize::from(*count) {
                return Ok((next, actions));
            }
            let stage_id = state
                .current_stage()
                .ok_or(TransitionError::WrongStage)?
                .id
                .clone();
            let approved_head = if *bind_head { head } else { None };
            next.approvals
                .retain(|approval| approval.stage_id != stage_id);
            next.approvals.push(ApprovalRecord {
                stage_id,
                head: approved_head,
                patch_id: if *bind_head {
                    state.patch_id.clone()
                } else {
                    None
                },
                reviewers: core::mem::take(&mut next.approval_reviewers),
            });
            if pick_fanout_index_for_approval(&state.workflow, state.stage_index).is_some() {
                let winner_task_id = next
                    .selected_fanout_child
                    .clone()
                    .ok_or(TransitionError::FanoutSelectionRequired)?;
                let sibling_task_ids = next
                    .fanout_child_task_ids
                    .iter()
                    .filter(|task_id| **task_id != winner_task_id)
                    .cloned()
                    .collect();
                actions.push(PipelineAction::CancelFanoutSiblings {
                    winner_task_id,
                    sibling_task_ids,
                });
            }
            enter_stage(&mut next, state.stage_index + 1, &mut actions)?;
        }
        PipelineEvent::CommitCreated { head, patch_id } => {
            ensure_running(state)?;
            if state.current_head.is_none() || state.current_head.as_deref() == Some(&head) {
                return Err(TransitionError::WrongStage);
            }
            next.current_head = Some(head);
            next.patch_id = Some(patch_id);
            clear_head_bound_approvals(&mut next);
            next.passed_checks.clear();
            next.approval_reviewers.clear();
            let target = first_recheck_stage(state).ok_or(TransitionError::WrongStage)?;
            enter_stage(&mut next, target, &mut actions)?;
        }
        PipelineEvent::MainAdvanced {
            rebased_head,
            patch_id,
            conflict,
        } => {
            ensure_running(state)?;
            if !state
                .workflow
                .stages
                .iter()
                .any(|stage| stage.stage.kind() == StageKind::Merge)
            {
                return Err(TransitionError::WrongStage);
            }
            let old_head = state
                .current_head
                .as_deref()
                .ok_or(TransitionError::MissingHead)?;
            let comparison_patch = state
                .approvals
                .iter()
                .find(|approval| approval.head.as_deref() == Some(old_head))
                .and_then(|approval| approval.patch_id.as_deref())
                .or(state.patch_id.as_deref())
                .ok_or(TransitionError::MissingHead)?;
            let decision = approval_after_rebase(&RebaseOutcome {
                conflict,
                rebased_head: Some(rebased_head.clone()),
                previous_patch_id: comparison_patch.into(),
                rebased_patch_id: (!conflict).then_some(patch_id.clone()),
            });
            next.passed_checks.clear();
            match decision {
                ApprovalAfterRebase::Keep { new_approved_head } => {
                    next.current_head = Some(new_approved_head.clone());
                    next.patch_id = Some(patch_id);
                    for approval in &mut next.approvals {
                        if approval.head.as_deref() == Some(old_head)
                            && approval.patch_id.as_deref() == Some(comparison_patch)
                        {
                            approval.head = Some(new_approved_head.clone());
                        }
                    }
                    let target = first_command_before_merge(state)
                        .ok_or(TransitionError::MergeGateClosed)?;
                    enter_stage(&mut next, target, &mut actions)?;
                }
                ApprovalAfterRebase::ReturnToWork => {
                    clear_head_bound_approvals(&mut next);
                    next.approval_reviewers.clear();
                    next.current_head = (!conflict).then_some(rebased_head);
                    next.patch_id = (!conflict).then_some(patch_id);
                    let work_index = first_kind(state, StageKind::Work)
                        .ok_or(TransitionError::NoFailureTarget)?;
                    clear_fanout(&mut next);
                    actions.push(PipelineAction::ReturnToWork);
                    enter_stage(&mut next, work_index, &mut actions)?;
                }
            }
        }
        PipelineEvent::FanoutCompleted {
            stage_id,
            child_task_ids,
            selected_child,
        } => {
            ensure_current_stage(state, StageKind::Fanout)?;
            if state.current_stage().map(|stage| stage.id.as_str()) != Some(stage_id.as_str()) {
                return Err(TransitionError::StaleResult);
            }
            if child_task_ids.is_empty()
                || child_task_ids.iter().any(String::is_empty)
                || child_task_ids
                    .iter()
                    .enumerate()
                    .any(|(index, task_id)| child_task_ids[..index].contains(task_id))
            {
                return Err(TransitionError::InvalidFanoutChildren);
            }
            let Some(WorkflowStage {
                stage: Stage::Fanout { join, .. },
                ..
            }) = state.current_stage()
            else {
                return Err(TransitionError::WrongStage);
            };
            next.selected_fanout_child = None;
            match join {
                FanoutJoin::All if selected_child.is_some() => {
                    return Err(TransitionError::UnexpectedFanoutSelection);
                }
                FanoutJoin::First => {
                    let winner = selected_child
                        .as_ref()
                        .ok_or(TransitionError::FanoutSelectionRequired)?;
                    if !child_task_ids.contains(winner) {
                        return Err(TransitionError::InvalidFanoutSelection);
                    }
                    actions.push(PipelineAction::CancelFanoutSiblings {
                        winner_task_id: winner.clone(),
                        sibling_task_ids: child_task_ids
                            .iter()
                            .filter(|task_id| task_id.as_str() != winner)
                            .cloned()
                            .collect(),
                    });
                    next.selected_fanout_child = Some(winner.clone());
                }
                FanoutJoin::Pick if selected_child.is_some() => {
                    return Err(TransitionError::UnexpectedFanoutSelection);
                }
                FanoutJoin::All | FanoutJoin::Pick => {}
            }
            next.fanout_child_task_ids = child_task_ids;
            enter_stage(&mut next, state.stage_index + 1, &mut actions)?;
        }
        PipelineEvent::StageFailed { stage_id, reason } => {
            ensure_running(state)?;
            if state.current_stage().map(|stage| stage.id.as_str()) != Some(stage_id.as_str()) {
                return Err(TransitionError::StaleResult);
            }
            fail_stage(&mut next, state.stage_index, &reason, &mut actions)?;
        }
        PipelineEvent::StageTimedOut { stage_id } => {
            ensure_running(state)?;
            let Some(stage) = state.current_stage() else {
                return Err(TransitionError::InvalidStageIndex);
            };
            if stage.id != stage_id {
                return Err(TransitionError::StaleResult);
            }
            match stage.effective_timeout_action() {
                TimeoutAction::Notify => actions.push(PipelineAction::NotifyTimeout { stage_id }),
                TimeoutAction::Reassign => actions.push(PipelineAction::ReassignStage { stage_id }),
                TimeoutAction::Cancel => {
                    next.status = PipelineStatus::Failed;
                    next.approval_reviewers.clear();
                    actions.push(PipelineAction::TaskCancelled { stage_id });
                }
            }
        }
        PipelineEvent::MergeCompleted { head, merge_commit } => {
            ensure_current_stage(state, StageKind::Merge)?;
            let gate = merge_gate(state, state.stage_index);
            if state.current_head.as_deref() != Some(head.as_str()) || !gate.allowed {
                return Err(TransitionError::MergeGateClosed);
            }
            next.merge_commit = Some(merge_commit.clone());
            next.status = PipelineStatus::Done;
            actions.push(PipelineAction::TaskDone {
                merge_commit: Some(merge_commit),
            });
        }
    }

    Ok((next, actions))
}

fn ensure_running(state: &PipelineState) -> Result<(), TransitionError> {
    if state.status != PipelineStatus::Running {
        return Err(TransitionError::NotRunning);
    }
    if state.current_stage().is_none() {
        return Err(TransitionError::InvalidStageIndex);
    }
    Ok(())
}

fn ensure_current_stage(state: &PipelineState, kind: StageKind) -> Result<(), TransitionError> {
    ensure_running(state)?;
    if current_kind(state, kind) {
        Ok(())
    } else {
        Err(TransitionError::WrongStage)
    }
}

fn current_kind(state: &PipelineState, kind: StageKind) -> bool {
    state
        .current_stage()
        .is_some_and(|stage| stage.stage.kind() == kind)
}

fn first_kind(state: &PipelineState, kind: StageKind) -> Option<usize> {
    state
        .workflow
        .stages
        .iter()
        .position(|stage| stage.stage.kind() == kind)
}

fn previous_work(state: &PipelineState, before: usize) -> Option<usize> {
    state
        .workflow
        .stages
        .get(..before)?
        .iter()
        .rposition(|stage| stage.stage.kind() == StageKind::Work)
}

fn first_command_before_merge(state: &PipelineState) -> Option<usize> {
    let merge = first_kind(state, StageKind::Merge)?;
    state.workflow.stages[..merge]
        .iter()
        .position(|stage| stage.stage.kind() == StageKind::Command)
}

fn first_recheck_stage(state: &PipelineState) -> Option<usize> {
    let merge = first_kind(state, StageKind::Merge)?;
    state.workflow.stages[..merge].iter().position(|stage| {
        stage.stage.kind() == StageKind::Command
            || matches!(
                stage.stage,
                Stage::Approval {
                    bind_head: true,
                    ..
                }
            )
    })
}

fn clear_head_bound_approvals(state: &mut PipelineState) {
    state.approvals.retain(|approval| approval.head.is_none());
}

fn clear_fanout(state: &mut PipelineState) {
    state.fanout_child_task_ids.clear();
    state.selected_fanout_child = None;
}

fn approval_for_stage(state: &PipelineState, stage: &WorkflowStage) -> bool {
    let Stage::Approval { bind_head, .. } = stage.stage else {
        return false;
    };
    state.approvals.iter().any(|approval| {
        approval.stage_id == stage.id
            && if bind_head {
                approval.head.as_deref() == state.current_head.as_deref()
            } else {
                approval.head.is_none()
            }
    })
}

fn pick_fanout_index_for_approval(workflow: &Workflow, approval_index: usize) -> Option<usize> {
    workflow
        .stages
        .get(..approval_index)?
        .iter()
        .enumerate()
        .rev()
        .find_map(|(fanout_index, stage)| {
            let Stage::Fanout {
                join: FanoutJoin::Pick,
                ..
            } = &stage.stage
            else {
                return None;
            };
            let first_approval_after_fanout = workflow
                .stages
                .get(fanout_index + 1..)?
                .iter()
                .position(|following| following.stage.kind() == StageKind::Approval)?
                + fanout_index
                + 1;
            (first_approval_after_fanout == approval_index).then_some(fanout_index)
        })
}

fn all_checks_passed(state: &PipelineState, before: usize) -> bool {
    let Some(head) = state.current_head.as_deref() else {
        return false;
    };
    state.workflow.stages[..before]
        .iter()
        .filter(|stage| stage.stage.kind() == StageKind::Command)
        .all(|stage| {
            state
                .passed_checks
                .iter()
                .any(|check| check.stage_id == stage.id && check.head == head)
        })
}

fn merge_gate(state: &PipelineState, before: usize) -> crate::policy::merge_gate::MergeGateResult {
    let current_head = state.current_head.as_deref().unwrap_or_default();
    let approved_head = state.workflow.stages[..before]
        .iter()
        .filter(|stage| {
            matches!(
                stage.stage,
                Stage::Approval {
                    bind_head: true,
                    ..
                }
            )
        })
        .find_map(|stage| {
            state
                .approvals
                .iter()
                .find(|approval| {
                    approval.stage_id == stage.id && approval.head.as_deref() == Some(current_head)
                })
                .and_then(|approval| approval.head.as_deref())
        });
    evaluate(
        all_checks_passed(state, before),
        approved_head,
        current_head,
        state.workflow.allow_unreviewed,
    )
}

fn fail_stage(
    state: &mut PipelineState,
    failed_index: usize,
    reason: &str,
    actions: &mut Vec<PipelineAction>,
) -> Result<(), TransitionError> {
    let failed = state
        .workflow
        .stages
        .get(failed_index)
        .ok_or(TransitionError::InvalidStageIndex)?;
    let explicit_target = failed.on_fail.as_deref().and_then(|target| {
        state
            .workflow
            .stages
            .iter()
            .position(|stage| stage.id == target)
    });
    let default_target = (failed.stage.kind() == StageKind::Command)
        .then(|| previous_work(state, failed_index))
        .flatten();
    let target = explicit_target.or(default_target);
    let failed_id = failed.id.clone();

    if let Some(target) = target {
        state.passed_checks.retain(|check| {
            stage_position(&state.workflow, &check.stage_id).is_some_and(|i| i < target)
        });
        state.approvals.retain(|approval| {
            stage_position(&state.workflow, &approval.stage_id).is_some_and(|i| i < target)
        });
        state.approval_reviewers.clear();
        if state
            .workflow
            .stages
            .get(target)
            .is_some_and(|stage| stage.stage.kind() == StageKind::Work)
        {
            clear_fanout(state);
            state.current_head = None;
            state.patch_id = None;
            state.branch = None;
            state.work_product = None;
            actions.push(PipelineAction::ReturnToWork);
        }
        enter_stage(state, target, actions)
    } else {
        state.status = PipelineStatus::Failed;
        state.approval_reviewers.clear();
        actions.push(PipelineAction::TaskFailed {
            stage_id: failed_id,
            reason: reason.into(),
        });
        Ok(())
    }
}

fn stage_position(workflow: &Workflow, stage_id: &str) -> Option<usize> {
    workflow
        .stages
        .iter()
        .position(|stage| stage.id == stage_id)
}

fn enter_stage(
    state: &mut PipelineState,
    mut index: usize,
    actions: &mut Vec<PipelineAction>,
) -> Result<(), TransitionError> {
    loop {
        let Some(stage) = state.workflow.stages.get(index) else {
            state.stage_index = state.workflow.stages.len();
            state.status = PipelineStatus::Done;
            actions.push(PipelineAction::TaskDone {
                merge_commit: state.merge_commit.clone(),
            });
            return Ok(());
        };
        state.stage_index = index;
        state.approval_reviewers.clear();

        if approval_for_stage(state, stage) {
            index += 1;
            continue;
        }

        actions.push(PipelineAction::ScheduleTimeout {
            stage_id: stage.id.clone(),
            timeout_ms: stage.effective_timeout_ms(),
            action: stage.effective_timeout_action(),
        });
        match &stage.stage {
            Stage::Work { role, .. } => actions.push(PipelineAction::AssignWork {
                stage_id: stage.id.clone(),
                role: role.clone(),
            }),
            Stage::Submit { forge } => actions.push(PipelineAction::Submit {
                stage_id: stage.id.clone(),
                forge: forge.clone(),
            }),
            Stage::Command { command } => actions.push(PipelineAction::RunCommand {
                stage_id: stage.id.clone(),
                command: command.clone(),
                head: state.current_head.clone(),
                branch: state.branch.clone(),
                timeout_ms: stage.effective_timeout_ms(),
                timeout_action: stage.effective_timeout_action(),
            }),
            Stage::Approval { bind_head, .. } => actions.push(PipelineAction::RequestApproval {
                stage_id: stage.id.clone(),
                bind_head: *bind_head,
                choices: if pick_fanout_index_for_approval(&state.workflow, index).is_some() {
                    state.fanout_child_task_ids.clone()
                } else {
                    Vec::new()
                },
            }),
            Stage::Merge => {
                let gate = merge_gate(state, index);
                if !gate.allowed {
                    return Err(TransitionError::MergeGateClosed);
                }
                let head = state
                    .current_head
                    .clone()
                    .ok_or(TransitionError::MissingHead)?;
                actions.push(PipelineAction::Merge {
                    stage_id: stage.id.clone(),
                    head,
                });
            }
            Stage::Fanout { source, join } => actions.push(PipelineAction::Fanout {
                stage_id: stage.id.clone(),
                source: source.clone(),
                join: *join,
                work_product: state.work_product.clone(),
            }),
        }
        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::workflow::Approver;
    use alloc::vec;

    fn initial() -> PipelineState {
        PipelineState::new("T-1", Workflow::builtin_code())
    }

    fn work(head: &str, patch: &str) -> PipelineEvent {
        PipelineEvent::WorkCompleted {
            product: WorkProduct::Branch {
                branch: "agend/T-1/demo".into(),
                head: head.into(),
                patch_id: patch.into(),
            },
        }
    }

    fn command(state: &PipelineState, exit_code: Option<i32>) -> PipelineEvent {
        PipelineEvent::CommandFinished {
            stage_id: state.current_stage().unwrap().id.clone(),
            head: state.current_head.clone().unwrap(),
            exit_code,
        }
    }

    fn approve(state: &PipelineState, reviewer: &str) -> PipelineEvent {
        PipelineEvent::ApprovalGranted {
            reviewer: reviewer.into(),
            head: state.current_head.clone(),
            selected_child: None,
        }
    }

    fn apply(state: &PipelineState, event: PipelineEvent) -> (PipelineState, Vec<PipelineAction>) {
        step(state, event).unwrap()
    }

    fn run_to_merge(mut state: PipelineState, head: &str, patch: &str) -> PipelineState {
        state = apply(&state, PipelineEvent::Start).0;
        state = apply(&state, work(head, patch)).0;
        state = apply(&state, PipelineEvent::Submitted).0;
        state = apply(&state, command(&state, Some(0))).0;
        apply(&state, approve(&state, "reviewer-1")).0
    }

    #[test]
    fn code_workflow_requires_checks_and_head_bound_approval() {
        let mut state = apply(&initial(), PipelineEvent::Start).0;
        state = apply(&state, work("H1", "P1")).0;
        state = apply(&state, PipelineEvent::Submitted).0;
        assert_eq!(
            step(&state, approve(&state, "reviewer-1")),
            Err(TransitionError::WrongStage)
        );
        state = apply(&state, command(&state, Some(0))).0;
        state = apply(&state, approve(&state, "reviewer-1")).0;
        let (done, actions) = apply(
            &state,
            PipelineEvent::MergeCompleted {
                head: "H1".into(),
                merge_commit: "M1".into(),
            },
        );
        assert_eq!(done.status, PipelineStatus::Done);
        assert_eq!(
            actions,
            [PipelineAction::TaskDone {
                merge_commit: Some("M1".into())
            }]
        );
    }

    #[test]
    fn each_command_stage_must_pass_on_the_current_head() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages.insert(
            3,
            WorkflowStage::new(
                "lint",
                Stage::Command {
                    command: "cargo fmt".into(),
                },
            ),
        );
        let mut state = apply(&PipelineState::new("T-1", workflow), PipelineEvent::Start).0;
        state = apply(&state, work("H1", "P1")).0;
        state = apply(&state, PipelineEvent::Submitted).0;
        state = apply(&state, command(&state, Some(0))).0;
        assert_eq!(state.current_stage().unwrap().id, "lint");
        state = apply(&state, command(&state, Some(0))).0;
        state = apply(&state, approve(&state, "reviewer-1")).0;
        assert!(matches!(state.current_stage().unwrap().stage, Stage::Merge));
        state
            .passed_checks
            .retain(|check| check.stage_id != "checks");
        assert_eq!(
            step(
                &state,
                PipelineEvent::MergeCompleted {
                    head: "H1".into(),
                    merge_commit: "M1".into()
                }
            ),
            Err(TransitionError::MergeGateClosed)
        );
    }

    #[test]
    fn approval_on_a_later_stage_is_never_skipped() {
        let mut workflow = Workflow::builtin_code();
        let reviewer = workflow.stages.remove(3);
        workflow.stages.insert(2, reviewer);
        workflow.stages.insert(
            4,
            WorkflowStage::new(
                "human",
                Stage::Approval {
                    by: Approver::Human,
                    count: 1,
                    bind_head: true,
                },
            ),
        );
        let mut state = apply(&PipelineState::new("T-1", workflow), PipelineEvent::Start).0;
        state = apply(&state, work("H1", "P1")).0;
        state = apply(&state, PipelineEvent::Submitted).0;
        state = apply(&state, approve(&state, "reviewer-1")).0;
        state = apply(&state, command(&state, Some(0))).0;
        assert_eq!(state.current_stage().unwrap().id, "human");
        state = apply(&state, approve(&state, "operator")).0;
        assert!(matches!(state.current_stage().unwrap().stage, Stage::Merge));
    }

    #[test]
    fn stale_command_result_cannot_pass_checks_for_a_new_head() {
        let state = run_to_merge(initial(), "H1", "P1");
        let (updated, _) = apply(
            &state,
            PipelineEvent::CommitCreated {
                head: "H2".into(),
                patch_id: "P2".into(),
            },
        );
        assert_eq!(
            step(
                &updated,
                PipelineEvent::CommandFinished {
                    stage_id: "checks".into(),
                    head: "H1".into(),
                    exit_code: Some(0)
                }
            ),
            Err(TransitionError::StaleResult)
        );
    }

    #[test]
    fn main_advance_restarts_all_checks_and_retains_matching_approvals() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages.insert(
            3,
            WorkflowStage::new(
                "lint",
                Stage::Command {
                    command: "cargo fmt".into(),
                },
            ),
        );
        let mut state = apply(&PipelineState::new("T-1", workflow), PipelineEvent::Start).0;
        state = apply(&state, work("H1", "P1")).0;
        state = apply(&state, PipelineEvent::Submitted).0;
        state = apply(&state, command(&state, Some(0))).0;
        state = apply(&state, command(&state, Some(0))).0;
        state = apply(&state, approve(&state, "reviewer-1")).0;
        let (rebased, _) = apply(
            &state,
            PipelineEvent::MainAdvanced {
                rebased_head: "H1-rebased".into(),
                patch_id: "P1".into(),
                conflict: false,
            },
        );
        assert_eq!(rebased.current_stage().unwrap().id, "checks");
        assert!(rebased.passed_checks.is_empty());
        assert!(
            rebased
                .approvals
                .iter()
                .any(|approval| approval.head.as_deref() == Some("H1-rebased"))
        );
        let rebased = apply(&rebased, command(&rebased, Some(0))).0;
        assert_eq!(rebased.current_stage().unwrap().id, "lint");
        let rebased = apply(&rebased, command(&rebased, Some(0))).0;
        assert!(matches!(
            rebased.current_stage().unwrap().stage,
            Stage::Merge
        ));
    }

    #[test]
    fn main_advance_uses_the_approved_patch_and_works_during_checks() {
        let mut state = run_to_merge(initial(), "H1", "P1");
        state.stage_index = 2;
        state.patch_id = Some("stale-unapproved-patch".into());
        let (rebased, _) = apply(
            &state,
            PipelineEvent::MainAdvanced {
                rebased_head: "H1-rebased".into(),
                patch_id: "P1".into(),
                conflict: false,
            },
        );
        assert_eq!(rebased.current_head.as_deref(), Some("H1-rebased"));
        assert_eq!(rebased.current_stage().unwrap().id, "checks");
    }

    #[test]
    fn main_advance_rebases_an_allow_unreviewed_workflow_without_an_approval_record() {
        let mut workflow = Workflow::builtin_code();
        workflow.allow_unreviewed = true;
        workflow
            .stages
            .retain(|stage| !matches!(stage.stage, Stage::Approval { .. }));
        let mut state = apply(&PipelineState::new("T-1", workflow), PipelineEvent::Start).0;
        state = apply(&state, work("H1", "P1")).0;
        assert_eq!(state.current_stage().unwrap().id, "submit");
        let (rebased, actions) = apply(
            &state,
            PipelineEvent::MainAdvanced {
                rebased_head: "H2".into(),
                patch_id: "P1".into(),
                conflict: false,
            },
        );
        assert_eq!(rebased.current_head.as_deref(), Some("H2"));
        assert_eq!(rebased.current_stage().unwrap().id, "checks");
        assert!(rebased.approvals.is_empty());
        assert!(actions.iter().any(|action| matches!(
            action,
            PipelineAction::RunCommand { head: Some(head), .. } if head == "H2"
        )));
    }

    #[test]
    fn changed_patch_after_main_advance_returns_to_work() {
        let state = run_to_merge(initial(), "H1", "P1");
        let (changed, actions) = apply(
            &state,
            PipelineEvent::MainAdvanced {
                rebased_head: "H1-other".into(),
                patch_id: "P2".into(),
                conflict: false,
            },
        );
        assert!(
            changed
                .approvals
                .iter()
                .all(|approval| approval.head.is_none())
        );
        assert!(matches!(
            changed.current_stage().unwrap().stage,
            Stage::Work { .. }
        ));
        assert!(actions.contains(&PipelineAction::ReturnToWork));
    }

    #[test]
    fn fanout_and_result_products_can_finish_builtin_epic() {
        let mut workflow = Workflow::builtin_epic();
        let Stage::Fanout { source, .. } = workflow.stages[1].stage.clone() else {
            unreachable!();
        };
        workflow.stages[1].stage = Stage::Fanout {
            source,
            join: FanoutJoin::Pick,
        };
        let mut state = apply(&PipelineState::new("T-1", workflow), PipelineEvent::Start).0;
        state = apply(
            &state,
            PipelineEvent::WorkCompleted {
                product: WorkProduct::Plan {
                    items: vec!["one".into(), "two".into()],
                },
            },
        )
        .0;
        assert!(matches!(
            state.current_stage().unwrap().stage,
            Stage::Fanout { .. }
        ));
        let (state, actions) = step(
            &state,
            PipelineEvent::FanoutCompleted {
                stage_id: "fanout".into(),
                child_task_ids: vec!["child-1".into(), "child-2".into()],
                selected_child: None,
            },
        )
        .unwrap();
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, PipelineAction::RequestApproval { .. }))
        );
        let approval_request = actions
            .iter()
            .find_map(|action| match action {
                PipelineAction::RequestApproval { choices, .. } => Some(choices),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            approval_request
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["child-1", "child-2"]
        );
        assert_eq!(
            step(
                &state,
                PipelineEvent::ApprovalGranted {
                    reviewer: "operator".into(),
                    head: None,
                    selected_child: None,
                }
            ),
            Err(TransitionError::FanoutSelectionRequired)
        );
        let (state, actions) = apply(
            &state,
            PipelineEvent::ApprovalGranted {
                reviewer: "operator".into(),
                head: None,
                selected_child: Some("child-2".into()),
            },
        );
        assert_eq!(state.status, PipelineStatus::Done);
        assert!(actions.contains(&PipelineAction::CancelFanoutSiblings {
            winner_task_id: "child-2".into(),
            sibling_task_ids: vec!["child-1".into()],
        }));

        let research = PipelineState::new("T-2", Workflow::builtin_research());
        let state = apply(&research, PipelineEvent::Start).0;
        let state = apply(
            &state,
            PipelineEvent::WorkCompleted {
                product: WorkProduct::Result {
                    summary: "done".into(),
                    output: None,
                },
            },
        )
        .0;
        assert_eq!(state.current_stage().unwrap().id, "review");
    }

    #[test]
    fn fanout_first_cancels_children_after_the_selected_one() {
        let mut workflow = Workflow::builtin_epic();
        let Stage::Fanout { source, .. } = workflow.stages[1].stage.clone() else {
            unreachable!();
        };
        workflow.stages[1].stage = Stage::Fanout {
            source,
            join: FanoutJoin::First,
        };
        let mut state = apply(&PipelineState::new("T-1", workflow), PipelineEvent::Start).0;
        state = apply(
            &state,
            PipelineEvent::WorkCompleted {
                product: WorkProduct::Plan {
                    items: vec!["one".into(), "two".into()],
                },
            },
        )
        .0;
        let (_, actions) = apply(
            &state,
            PipelineEvent::FanoutCompleted {
                stage_id: "fanout".into(),
                child_task_ids: vec!["child-1".into(), "child-2".into()],
                selected_child: Some("child-2".into()),
            },
        );
        assert!(actions.contains(&PipelineAction::CancelFanoutSiblings {
            winner_task_id: "child-2".into(),
            sibling_task_ids: vec!["child-1".into()],
        }));
    }

    #[test]
    fn fanout_all_waits_for_child_set_without_requesting_a_winner() {
        let mut state = apply(
            &PipelineState::new("T-1", Workflow::builtin_epic()),
            PipelineEvent::Start,
        )
        .0;
        state = apply(
            &state,
            PipelineEvent::WorkCompleted {
                product: WorkProduct::Plan {
                    items: vec!["one".into(), "two".into()],
                },
            },
        )
        .0;
        let (state, actions) = apply(
            &state,
            PipelineEvent::FanoutCompleted {
                stage_id: "fanout".into(),
                child_task_ids: vec!["child-1".into(), "child-2".into()],
                selected_child: None,
            },
        );
        let choices = actions
            .iter()
            .find_map(|action| match action {
                PipelineAction::RequestApproval { choices, .. } => Some(choices),
                _ => None,
            })
            .unwrap();
        assert!(choices.is_empty());
        assert_eq!(state.fanout_child_task_ids, ["child-1", "child-2"]);
    }

    #[test]
    fn timeout_actions_are_emitted_and_cancel_fails_the_pipeline() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[0].on_timeout = Some(TimeoutAction::Reassign);
        let state = apply(&PipelineState::new("T-1", workflow), PipelineEvent::Start).0;
        let (state, actions) = apply(
            &state,
            PipelineEvent::StageTimedOut {
                stage_id: "work".into(),
            },
        );
        assert_eq!(state.status, PipelineStatus::Running);
        assert!(actions.contains(&PipelineAction::ReassignStage {
            stage_id: "work".into()
        }));

        let mut workflow = Workflow::builtin_code();
        workflow.stages[0].on_timeout = Some(TimeoutAction::Cancel);
        let state = apply(&PipelineState::new("T-1", workflow), PipelineEvent::Start).0;
        let (state, actions) = apply(
            &state,
            PipelineEvent::StageTimedOut {
                stage_id: "work".into(),
            },
        );
        assert_eq!(state.status, PipelineStatus::Failed);
        assert!(actions.contains(&PipelineAction::TaskCancelled {
            stage_id: "work".into()
        }));
    }

    #[test]
    fn non_command_failure_uses_on_fail_target() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[1].on_fail = Some("work".into());
        let mut state = apply(&PipelineState::new("T-1", workflow), PipelineEvent::Start).0;
        state = apply(&state, work("H1", "P1")).0;
        let (state, _) = apply(
            &state,
            PipelineEvent::StageFailed {
                stage_id: "submit".into(),
                reason: "forge is unavailable".into(),
            },
        );
        assert_eq!(state.current_stage().unwrap().id, "work");

        let state = apply(&initial(), PipelineEvent::Start).0;
        let (state, actions) = apply(
            &state,
            PipelineEvent::StageFailed {
                stage_id: "work".into(),
                reason: "driver failed".into(),
            },
        );
        assert_eq!(state.status, PipelineStatus::Failed);
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, PipelineAction::TaskFailed { .. }))
        );
    }

    #[test]
    fn merge_workflow_requires_a_command_check() {
        let mut workflow = Workflow::builtin_code();
        workflow
            .stages
            .retain(|stage| stage.stage.kind() != StageKind::Command);
        assert!(
            workflow
                .validate(&["dev".into(), "reviewer".into()])
                .unwrap_err()
                .iter()
                .any(|error| matches!(
                    error,
                    crate::pipeline::workflow::WorkflowError::MergeWithoutCommand { .. }
                ))
        );
    }

    #[test]
    fn invalid_stage_index_does_not_panic() {
        let mut state = initial();
        state.status = PipelineStatus::Running;
        state.stage_index = usize::MAX;
        assert_eq!(
            step(&state, PipelineEvent::Submitted),
            Err(TransitionError::InvalidStageIndex)
        );
    }
}
