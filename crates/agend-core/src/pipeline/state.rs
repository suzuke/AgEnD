//! Pure workflow execution state machine (D11). Callers provide observed
//! events; returned actions describe side effects for the daemon to perform.
//!
//! Head changes (`CommitCreated`, `MainAdvanced`) never move a task forward:
//! in a `work` stage they only record the new head, so rework in progress is
//! kept; elsewhere they send the task back to the first check that must be
//! repeated for the new head (D14). The merge gate is
//! [`crate::policy::merge_gate::evaluate`], fed one fact per stage.
//!
//! Must NOT: call a trait, execute a command, or access storage.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use super::stage::{FanoutJoin, StageKind};
use super::workflow::{
    CommandContext, FanoutSource, Stage, TimeoutAction, ValidatedWorkflow, WorkOutput, Workflow,
    WorkflowError, WorkflowStage, expand_command_placeholders,
};
use crate::policy::merge_gate::{
    ApprovalAfterRebase, GateFact, MergeGateResult, RebaseOutcome, approval_after_rebase,
    approval_satisfied, evaluate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineStatus {
    Pending,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl PipelineStatus {
    /// Done, failed and cancelled pipelines accept no further event.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// A command stage that exited 0 for `head` (`None` when the task has no
/// branch head, as in a workflow without repo stages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassedCheck {
    pub stage_id: String,
    pub head: Option<String>,
}

/// A completed approval stage. `head` and `patch_id` are set only for a
/// head-bound stage: the head it was given for (moved by a clean same-patch
/// rebase, D14) and that head's patch-id.
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
    task_id: String,
    workflow: Workflow,
    stage_index: usize,
    status: PipelineStatus,
    branch: Option<String>,
    current_head: Option<String>,
    patch_id: Option<String>,
    /// Change id returned by the last submit (for example a pull request
    /// number); expands `{pr}` in command stages.
    change_id: Option<String>,
    work_product: Option<WorkProduct>,
    /// Reviewers who approved the current approval stage so far (below its
    /// count).
    approval_reviewers: Vec<String>,
    /// For a pick approval below its count: the winner those reviewers
    /// chose. It becomes `selected_fanout_child` only when the count is met.
    pending_pick: Option<String>,
    /// Per stage: how many times the task entered it (1 for the first time).
    /// Results must echo the current stage's attempt.
    attempts: Vec<u32>,
    approvals: Vec<ApprovalRecord>,
    passed_checks: Vec<PassedCheck>,
    fanout_child_task_ids: Vec<String>,
    selected_fanout_child: Option<String>,
    merge_commit: Option<String>,
    /// Head changes seen while the merge was in flight, in order; applied
    /// only if the forge reports the merge failed.
    pending_head_changes: Vec<PendingHeadChange>,
}

/// A head change observed after the `Merge` action went out. The task stays
/// in the merge stage until the forge reports the result: on `MergeCompleted`
/// these are dropped (the sent head was merged), on `MergeFailed` they are
/// applied in order with the usual rules (D14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingHeadChange {
    CommitCreated {
        head: String,
        patch_id: String,
    },
    MainAdvanced {
        rebased_head: String,
        patch_id: String,
        conflict: bool,
    },
}

/// Read access. Fields are private so a state can only come from
/// [`PipelineState::new`] with a validated workflow plus accepted events.
impl PipelineState {
    pub fn new(task_id: impl Into<String>, workflow: ValidatedWorkflow) -> Self {
        Self::unchecked(task_id, workflow.into_inner())
    }

    fn unchecked(task_id: impl Into<String>, workflow: Workflow) -> Self {
        Self {
            task_id: task_id.into(),
            attempts: alloc::vec![0; workflow.stages.len()],
            workflow,
            stage_index: 0,
            status: PipelineStatus::Pending,
            branch: None,
            current_head: None,
            patch_id: None,
            change_id: None,
            work_product: None,
            approval_reviewers: Vec::new(),
            pending_pick: None,
            approvals: Vec::new(),
            passed_checks: Vec::new(),
            fanout_child_task_ids: Vec::new(),
            selected_fanout_child: None,
            merge_commit: None,
            pending_head_changes: Vec::new(),
        }
    }

    pub fn current_stage(&self) -> Option<&WorkflowStage> {
        self.workflow.stages.get(self.stage_index)
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn workflow(&self) -> &Workflow {
        &self.workflow
    }

    pub fn stage_index(&self) -> usize {
        self.stage_index
    }

    pub fn status(&self) -> PipelineStatus {
        self.status
    }

    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    pub fn current_head(&self) -> Option<&str> {
        self.current_head.as_deref()
    }

    pub fn patch_id(&self) -> Option<&str> {
        self.patch_id.as_deref()
    }

    pub fn change_id(&self) -> Option<&str> {
        self.change_id.as_deref()
    }

    pub fn work_product(&self) -> Option<&WorkProduct> {
        self.work_product.as_ref()
    }

    pub fn approval_reviewers(&self) -> &[String] {
        &self.approval_reviewers
    }

    /// The current stage's attempt: results for it must carry this number.
    pub fn attempt(&self) -> u32 {
        self.attempts.get(self.stage_index).copied().unwrap_or(0)
    }

    /// The winner chosen so far by a pick approval below its count.
    pub fn pending_pick(&self) -> Option<&str> {
        self.pending_pick.as_deref()
    }

    pub fn approvals(&self) -> &[ApprovalRecord] {
        &self.approvals
    }

    pub fn passed_checks(&self) -> &[PassedCheck] {
        &self.passed_checks
    }

    pub fn fanout_child_task_ids(&self) -> &[String] {
        &self.fanout_child_task_ids
    }

    pub fn selected_fanout_child(&self) -> Option<&str> {
        self.selected_fanout_child.as_deref()
    }

    pub fn merge_commit(&self) -> Option<&str> {
        self.merge_commit.as_deref()
    }

    pub fn pending_head_changes(&self) -> &[PendingHeadChange] {
        &self.pending_head_changes
    }

    /// The `Merge` action is out and its result has not arrived.
    pub fn merge_in_flight(&self) -> bool {
        self.status == PipelineStatus::Running
            && self
                .current_stage()
                .is_some_and(|stage| stage.stage.kind() == StageKind::Merge)
    }

    /// The merge gate for the merge stage at `merge_index`, from the recorded
    /// checks and approvals of every stage before it.
    pub fn merge_gate(&self, merge_index: usize) -> MergeGateResult {
        let before = self
            .workflow
            .stages
            .get(..merge_index)
            .unwrap_or(&self.workflow.stages);
        let facts: Vec<GateFact<'_>> = before
            .iter()
            .filter_map(|stage| match stage.stage {
                Stage::Command { .. } => Some(GateFact::Check {
                    stage_id: &stage.id,
                    passed_heads: self
                        .passed_checks
                        .iter()
                        .filter(|check| check.stage_id == stage.id)
                        .filter_map(|check| check.head.as_deref())
                        .collect(),
                }),
                Stage::Approval { bind_head, .. } => Some(GateFact::Approval {
                    stage_id: &stage.id,
                    bind_head,
                    approvals: self.approval_heads(&stage.id),
                }),
                _ => None,
            })
            .collect();
        evaluate(
            self.current_head.as_deref(),
            &facts,
            self.workflow.allow_unreviewed,
        )
    }

    fn approval_heads(&self, stage_id: &str) -> Vec<Option<&str>> {
        self.approvals
            .iter()
            .filter(|approval| approval.stage_id == stage_id)
            .map(|approval| approval.head.as_deref())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Observed events. Every result of a stage carries its identity: the
/// `stage_id` and `attempt` from the action that asked for it, and, for a
/// head-bound stage (command, merge, head-bound approval), the `head` it was
/// about. [`step`] rejects a result whose identity is not the current one with
/// [`TransitionError::StaleResult`] and changes nothing. `Start`,
/// `CommitCreated`, `MainAdvanced` and `Cancel` are observations, not results.
pub enum PipelineEvent {
    Start,
    WorkCompleted {
        stage_id: String,
        attempt: u32,
        product: WorkProduct,
    },
    /// The forge accepted the submission; `change_id` is what it returned
    /// (for example a pull request number), if anything.
    Submitted {
        stage_id: String,
        attempt: u32,
        change_id: Option<String>,
    },
    CommandFinished {
        stage_id: String,
        attempt: u32,
        head: Option<String>,
        exit_code: Option<i32>,
    },
    ApprovalGranted {
        stage_id: String,
        attempt: u32,
        reviewer: String,
        head: Option<String>,
        selected_child: Option<String>,
    },
    /// A reviewer asked for changes: the task goes back to the task holder (D18, D33).
    ChangesRequested {
        stage_id: String,
        attempt: u32,
        reviewer: String,
        head: Option<String>,
        reason: String,
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
    /// `attempt` is the fanout run the children belong to.
    FanoutCompleted {
        stage_id: String,
        attempt: u32,
        child_task_ids: Vec<String>,
        selected_child: Option<String>,
    },
    StageFailed {
        stage_id: String,
        attempt: u32,
        reason: String,
    },
    StageTimedOut {
        stage_id: String,
        attempt: u32,
    },
    /// Operator cancellation.
    Cancel {
        reason: String,
    },
    MergeCompleted {
        stage_id: String,
        attempt: u32,
        head: String,
        merge_commit: String,
    },
    /// The forge did not merge `head` (for example main moved or the forge
    /// refused); `reason` is for the log.
    MergeFailed {
        stage_id: String,
        attempt: u32,
        head: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineAction {
    /// Every action that asks for a result carries `stage_id` and `attempt`
    /// (and the head, where the stage is head-bound); the daemon echoes them
    /// in the result event.
    AssignWork {
        stage_id: String,
        attempt: u32,
        role: String,
    },
    /// Rework: the work stage goes back to the task holder, who has held
    /// the task since it was assigned (D33; `policy::assign::Purpose::Rework`
    /// with `reason` as the review comment).
    ReturnToWork {
        stage_id: String,
        attempt: u32,
        role: String,
        reason: String,
    },
    Submit {
        stage_id: String,
        attempt: u32,
        forge: String,
    },
    /// `command` has its placeholders already expanded (and shell-quoted).
    RunCommand {
        stage_id: String,
        attempt: u32,
        command: String,
        head: Option<String>,
        branch: Option<String>,
        change_id: Option<String>,
        timeout_ms: u64,
        timeout_action: TimeoutAction,
    },
    /// `head` is the head under review for a head-bound stage; `choices` are
    /// the live children of the current fanout run for a pick.
    RequestApproval {
        stage_id: String,
        attempt: u32,
        bind_head: bool,
        head: Option<String>,
        choices: Vec<String>,
    },
    Merge {
        stage_id: String,
        attempt: u32,
        head: String,
    },
    /// `work_product` and `head` are the current ones, also when the
    /// fanout runs again after a head change.
    Fanout {
        stage_id: String,
        attempt: u32,
        source: FanoutSource,
        join: FanoutJoin,
        work_product: Option<WorkProduct>,
        head: Option<String>,
    },
    CancelFanoutSiblings {
        winner_task_id: String,
        sibling_task_ids: Vec<String>,
    },
    ScheduleTimeout {
        stage_id: String,
        attempt: u32,
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
        stage_id: Option<String>,
        reason: String,
    },
    TaskFailed {
        stage_id: String,
        reason: String,
    },
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
    MergeGateClosed,
    EmptyWorkflow,
    StaleResult,
    InvalidWorkProduct,
    InvalidStageIndex,
    InvalidFanoutChildren,
    FanoutSelectionRequired,
    InvalidFanoutSelection,
    UnexpectedFanoutSelection,
    MergeInFlight,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotPending => "pipeline has already started",
            Self::NotRunning => "pipeline is not running",
            Self::WrongStage => "event does not match the current stage",
            Self::MissingHead => "the current stage requires a commit head",
            Self::MergeGateClosed => {
                "merge requires every check to pass and every approval to cover the current head"
            }
            Self::EmptyWorkflow => "workflow has no stages",
            Self::StaleResult => {
                "result does not belong to the current stage, attempt and head; nothing changed"
            }
            Self::InvalidWorkProduct => "work output does not match the current work stage",
            Self::InvalidStageIndex => "pipeline state has an invalid stage index",
            Self::InvalidFanoutChildren => "fanout completion has no unique child task ids",
            Self::FanoutSelectionRequired => "a fanout winner must be selected for approval",
            Self::InvalidFanoutSelection => "selected task is not a completed fanout child",
            Self::UnexpectedFanoutSelection => "this approval does not select a fanout child",
            Self::MergeInFlight => {
                "the merge has been requested; only its result or a head change is accepted until then"
            }
        })
    }
}

impl core::error::Error for TransitionError {}

/// Apply one observed event and return a new state plus the side effects to run.
pub fn step(
    state: &PipelineState,
    event: PipelineEvent,
) -> Result<(PipelineState, Vec<PipelineAction>), TransitionError> {
    // One rule for the in-flight merge: only its result or a head change
    // (recorded as pending) is accepted until the forge answers.
    if state.merge_in_flight()
        && !matches!(
            event,
            PipelineEvent::MergeCompleted { .. }
                | PipelineEvent::MergeFailed { .. }
                | PipelineEvent::CommitCreated { .. }
                | PipelineEvent::MainAdvanced { .. }
        )
    {
        return Err(TransitionError::MergeInFlight);
    }
    // One rule for staleness: a result must carry the identity of the
    // current stage, attempt and (for a head-bound stage) head.
    if let Some(identity) = identity(&event) {
        check_identity(state, &identity)?;
    }

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
        PipelineEvent::WorkCompleted { product, .. } => {
            let Some(Stage::Work { output, .. }) = state.current_stage().map(|s| &s.stage) else {
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
                    next.current_head = Some(head.clone());
                    invalidate_head_bound(&mut next);
                }
                next.branch = Some(branch.clone());
                next.current_head = Some(head.clone());
                next.patch_id = Some(patch_id.clone());
            }
            if matches!(&product, WorkProduct::Plan { .. })
                && fanout_at_or_after(state, state.stage_index)
            {
                clear_fanout(&mut next);
            }
            next.work_product = Some(product);
            enter_stage(&mut next, state.stage_index + 1, &mut actions)?;
        }
        PipelineEvent::Submitted { change_id, .. } => {
            ensure_current_stage(state, StageKind::Submit)?;
            next.change_id = change_id;
            enter_stage(&mut next, state.stage_index + 1, &mut actions)?;
        }
        PipelineEvent::CommandFinished {
            stage_id,
            head,
            exit_code,
            ..
        } => {
            ensure_current_stage(state, StageKind::Command)?;
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
                let reason = match exit_code {
                    Some(code) => format!("command `{stage_id}` failed with exit code {code}"),
                    None => format!("command `{stage_id}` did not exit normally"),
                };
                fail_stage(&mut next, state.stage_index, reason, &mut actions)?;
            }
        }
        PipelineEvent::ApprovalGranted {
            stage_id,
            reviewer,
            head,
            selected_child,
            ..
        } => {
            let (bind_head, count) = current_approval(state)?;
            let picks =
                pick_fanout_index_for_approval(&state.workflow, state.stage_index).is_some();
            if picks {
                // Each reviewer's choice must agree with the ones so far; the
                // winner is fixed only when the count is met.
                let winner = selected_child
                    .or_else(|| next.pending_pick.clone())
                    .ok_or(TransitionError::FanoutSelectionRequired)?;
                if !next.fanout_child_task_ids.contains(&winner)
                    || next
                        .pending_pick
                        .as_ref()
                        .is_some_and(|pending| pending != &winner)
                {
                    return Err(TransitionError::InvalidFanoutSelection);
                }
                next.pending_pick = Some(winner);
            } else if selected_child.is_some() {
                return Err(TransitionError::UnexpectedFanoutSelection);
            }
            if !next.approval_reviewers.contains(&reviewer) {
                next.approval_reviewers.push(reviewer);
            }
            if next.approval_reviewers.len() < usize::from(count) {
                return Ok((next, actions));
            }
            next.approvals
                .retain(|approval| approval.stage_id != stage_id);
            next.approvals.push(ApprovalRecord {
                stage_id,
                head: if bind_head { head } else { None },
                patch_id: if bind_head {
                    state.patch_id.clone()
                } else {
                    None
                },
                reviewers: core::mem::take(&mut next.approval_reviewers),
            });
            if picks {
                let winner = next
                    .pending_pick
                    .take()
                    .ok_or(TransitionError::FanoutSelectionRequired)?;
                keep_only_winner(&mut next, winner, &mut actions);
            }
            enter_stage(&mut next, state.stage_index + 1, &mut actions)?;
        }
        PipelineEvent::ChangesRequested {
            reviewer, reason, ..
        } => {
            current_approval(state)?;
            let reason = format!("changes requested by {reviewer}: {reason}");
            fail_stage(&mut next, state.stage_index, reason, &mut actions)?;
        }
        PipelineEvent::CommitCreated { head, patch_id } => {
            ensure_running(state)?;
            // A workflow without branch work has no head to change.
            if !state.workflow.stages.iter().any(is_branch_work) {
                return Err(TransitionError::WrongStage);
            }
            if state.current_head.as_deref() == Some(head.as_str()) {
                // During an in-flight merge, the branch back at the sent head
                // means it was reset: the pending changes no longer exist.
                if state.merge_in_flight() {
                    next.pending_head_changes.clear();
                }
                return Ok((next, actions));
            }
            let change = PendingHeadChange::CommitCreated { head, patch_id };
            if state.merge_in_flight() {
                next.pending_head_changes.push(change);
            } else {
                apply_head_change(&mut next, change, &mut actions)?;
            }
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
            let change = PendingHeadChange::MainAdvanced {
                rebased_head,
                patch_id,
                conflict,
            };
            if state.merge_in_flight() {
                next.pending_head_changes.push(change);
            } else {
                apply_head_change(&mut next, change, &mut actions)?;
            }
        }
        PipelineEvent::FanoutCompleted {
            child_task_ids,
            selected_child,
            ..
        } => {
            ensure_current_stage(state, StageKind::Fanout)?;
            if child_task_ids.is_empty()
                || child_task_ids.iter().any(String::is_empty)
                || child_task_ids
                    .iter()
                    .enumerate()
                    .any(|(index, task_id)| child_task_ids[..index].contains(task_id))
            {
                return Err(TransitionError::InvalidFanoutChildren);
            }
            let Some(Stage::Fanout { join, .. }) = state.current_stage().map(|s| &s.stage) else {
                return Err(TransitionError::WrongStage);
            };
            next.selected_fanout_child = None;
            next.fanout_child_task_ids = child_task_ids;
            match join {
                FanoutJoin::All | FanoutJoin::Pick if selected_child.is_some() => {
                    return Err(TransitionError::UnexpectedFanoutSelection);
                }
                FanoutJoin::First => {
                    let winner = selected_child.ok_or(TransitionError::FanoutSelectionRequired)?;
                    if !next.fanout_child_task_ids.contains(&winner) {
                        return Err(TransitionError::InvalidFanoutSelection);
                    }
                    keep_only_winner(&mut next, winner, &mut actions);
                }
                FanoutJoin::All | FanoutJoin::Pick => {}
            }
            enter_stage(&mut next, state.stage_index + 1, &mut actions)?;
        }
        PipelineEvent::StageFailed { reason, .. } => {
            fail_stage(&mut next, state.stage_index, reason, &mut actions)?;
        }
        PipelineEvent::StageTimedOut { stage_id, .. } => {
            let stage = state
                .current_stage()
                .ok_or(TransitionError::InvalidStageIndex)?;
            match stage.effective_timeout_action() {
                TimeoutAction::Notify => actions.push(PipelineAction::NotifyTimeout { stage_id }),
                TimeoutAction::Reassign => {
                    // Someone else must produce the result: a new attempt, so
                    // the old holder's result is stale, and the request again.
                    actions.push(PipelineAction::ReassignStage { stage_id });
                    enter_stage(&mut next, state.stage_index, &mut actions)?;
                }
                TimeoutAction::Cancel => {
                    let reason = format!("stage `{stage_id}` timed out");
                    cancel(&mut next, Some(stage_id), reason, &mut actions);
                }
            }
        }
        PipelineEvent::Cancel { reason } => {
            let stage_id = match state.status {
                PipelineStatus::Pending => None,
                PipelineStatus::Running => {
                    ensure_running(state)?;
                    state.current_stage().map(|stage| stage.id.clone())
                }
                _ => return Err(TransitionError::NotRunning),
            };
            cancel(&mut next, stage_id, reason, &mut actions);
        }
        PipelineEvent::MergeFailed { .. } => {
            ensure_current_stage(state, StageKind::Merge)?;
            merge_failed(&mut next, &mut actions)?;
        }
        PipelineEvent::MergeCompleted { merge_commit, .. } => {
            ensure_current_stage(state, StageKind::Merge)?;
            if !state.merge_gate(state.stage_index).allowed {
                return Err(TransitionError::MergeGateClosed);
            }
            next.merge_commit = Some(merge_commit.clone());
            next.pending_head_changes.clear();
            next.status = PipelineStatus::Done;
            actions.push(PipelineAction::TaskDone {
                merge_commit: Some(merge_commit),
            });
        }
    }

    Ok((next, actions))
}

/// The identity a result event carries.
struct Identity<'a> {
    stage_id: &'a str,
    attempt: u32,
    /// `Some` when the event names a head.
    head: Option<Option<&'a str>>,
}

fn identity(event: &PipelineEvent) -> Option<Identity<'_>> {
    let (stage_id, attempt, head) = match event {
        PipelineEvent::WorkCompleted {
            stage_id, attempt, ..
        }
        | PipelineEvent::Submitted {
            stage_id, attempt, ..
        }
        | PipelineEvent::FanoutCompleted {
            stage_id, attempt, ..
        }
        | PipelineEvent::StageFailed {
            stage_id, attempt, ..
        }
        | PipelineEvent::StageTimedOut { stage_id, attempt } => (stage_id, *attempt, None),
        PipelineEvent::CommandFinished {
            stage_id,
            attempt,
            head,
            ..
        }
        | PipelineEvent::ApprovalGranted {
            stage_id,
            attempt,
            head,
            ..
        }
        | PipelineEvent::ChangesRequested {
            stage_id,
            attempt,
            head,
            ..
        } => (stage_id, *attempt, Some(head.as_deref())),
        PipelineEvent::MergeCompleted {
            stage_id,
            attempt,
            head,
            ..
        }
        | PipelineEvent::MergeFailed {
            stage_id,
            attempt,
            head,
            ..
        } => (stage_id, *attempt, Some(Some(head.as_str()))),
        PipelineEvent::Start
        | PipelineEvent::CommitCreated { .. }
        | PipelineEvent::MainAdvanced { .. }
        | PipelineEvent::Cancel { .. } => return None,
    };
    Some(Identity {
        stage_id,
        attempt,
        head,
    })
}

/// The staleness rule: the result is for the current stage and attempt and,
/// when the stage is head-bound, for the current head.
fn check_identity(state: &PipelineState, identity: &Identity<'_>) -> Result<(), TransitionError> {
    ensure_running(state)?;
    let stage = state
        .current_stage()
        .ok_or(TransitionError::InvalidStageIndex)?;
    if stage.id != identity.stage_id || state.attempt() != identity.attempt {
        return Err(TransitionError::StaleResult);
    }
    let head_bound = matches!(
        stage.stage,
        Stage::Command { .. }
            | Stage::Merge
            | Stage::Approval {
                bind_head: true,
                ..
            }
    );
    if head_bound
        && let Some(head) = identity.head
        && head != state.current_head.as_deref()
    {
        return Err(TransitionError::StaleResult);
    }
    Ok(())
}

/// The winner of a fanout is fixed: its siblings are cancelled and no longer
/// offered as choices.
fn keep_only_winner(state: &mut PipelineState, winner: String, actions: &mut Vec<PipelineAction>) {
    let sibling_task_ids = state
        .fanout_child_task_ids
        .iter()
        .filter(|task_id| **task_id != winner)
        .cloned()
        .collect::<Vec<_>>();
    if !sibling_task_ids.is_empty() {
        actions.push(PipelineAction::CancelFanoutSiblings {
            winner_task_id: winner.clone(),
            sibling_task_ids,
        });
    }
    state.fanout_child_task_ids = alloc::vec![winner.clone()];
    state.selected_fanout_child = Some(winner);
}

/// The forge did not merge: apply the head changes seen meanwhile, or, if
/// there were none (or they changed nothing), repeat the checks for the
/// current head before asking for the merge again.
fn merge_failed(
    state: &mut PipelineState,
    actions: &mut Vec<PipelineAction>,
) -> Result<(), TransitionError> {
    // Apply every pending change, but only the last change that did
    // something speaks: earlier ones would ask for checks on a stale head.
    let mut last_actions = Vec::new();
    for change in core::mem::take(&mut state.pending_head_changes) {
        let mut change_actions = Vec::new();
        apply_head_change(state, change, &mut change_actions)?;
        if !change_actions.is_empty() {
            last_actions = change_actions;
        }
    }
    actions.extend(last_actions);
    if state.merge_in_flight() {
        let merge = state.stage_index;
        let target = recheck_target(state, merge).ok_or(TransitionError::MergeGateClosed)?;
        enter_stage(state, target, actions)?;
    }
    Ok(())
}

fn apply_head_change(
    state: &mut PipelineState,
    change: PendingHeadChange,
    actions: &mut Vec<PipelineAction>,
) -> Result<(), TransitionError> {
    if state.status != PipelineStatus::Running {
        return Ok(());
    }
    match change {
        PendingHeadChange::CommitCreated { head, patch_id } => {
            if state.current_head.as_deref() == Some(head.as_str()) {
                return Ok(());
            }
            record_new_head(state, head, patch_id);
            recheck_for_new_head(state, actions)
        }
        PendingHeadChange::MainAdvanced {
            rebased_head,
            patch_id,
            conflict,
        } => main_advanced(state, rebased_head, patch_id, conflict, actions),
    }
}

/// Completability witness, run by `Workflow::validate` after the static
/// rules: drive `step` with the canonical all-success script, then once per
/// command/approval stage with one failure there (rework), then once per
/// stage after a branch exists with a new commit there. Every run
/// must reach done within a bound; otherwise the workflow is rejected with the
/// stage where it got stuck. Deterministic and bounded.
pub(crate) fn completability_witness(workflow: &Workflow) -> Result<(), WorkflowError> {
    witness_run(workflow, None, "success script")?;
    for (index, stage) in workflow.stages.iter().enumerate() {
        if matches!(stage.stage.kind(), StageKind::Command | StageKind::Approval) {
            witness_run(
                workflow,
                Some((index, Disturbance::Fail)),
                &format!("rework after `{}` fails", stage.id),
            )?;
        }
    }
    for (index, stage) in workflow.stages.iter().enumerate() {
        let has_branch_before = workflow.stages[..index].iter().any(|earlier| {
            matches!(
                earlier.stage,
                Stage::Work {
                    output: WorkOutput::Branch,
                    ..
                }
            )
        });
        if has_branch_before {
            witness_run(
                workflow,
                Some((index, Disturbance::NewCommit)),
                &format!("new commit at `{}`", stage.id),
            )?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Disturbance {
    /// The command exits 1, or the reviewer asks for changes.
    Fail,
    /// A new commit arrives (at merge: while in flight, then the merge fails).
    NewCommit,
}

fn witness_run(
    workflow: &Workflow,
    disturbance: Option<(usize, Disturbance)>,
    scenario: &str,
) -> Result<(), WorkflowError> {
    let mut state = PipelineState::unchecked("witness", workflow.clone());
    let mut counter = 0_u32;
    let mut disturbed = false;
    let stuck = |state: &PipelineState, reason: String| WorkflowError::NotCompletable {
        scenario: scenario.into(),
        stage_id: state
            .current_stage()
            .map_or_else(|| "<end>".into(), |stage| stage.id.clone()),
        reason,
    };
    for _ in 0..workflow.stages.len() * 8 + 16 {
        match state.status {
            PipelineStatus::Done => {
                return done_invariants(&state).map_err(|reason| stuck(&state, reason));
            }
            PipelineStatus::Failed | PipelineStatus::Cancelled => {
                return Err(stuck(
                    &state,
                    format!("the task ended as {:?}", state.status),
                ));
            }
            _ => {}
        }
        let events = match disturbance {
            Some((index, kind))
                if !disturbed
                    && state.status == PipelineStatus::Running
                    && state.stage_index == index =>
            {
                disturbed = true;
                disturbance_events(&state, kind, &mut counter)
            }
            _ => success_events(&state, &mut counter),
        };
        let before = (state.stage_index, state.attempt(), state.status);
        let collected_before = !state.approval_reviewers.is_empty() || state.pending_pick.is_some();
        let collecting = state.current_stage().is_some_and(|stage| {
            matches!(stage.stage.kind(), StageKind::Approval | StageKind::Fanout)
        });
        let head_before = state.current_head.clone();
        let in_flight = state.merge_in_flight();
        for event in &events {
            state = step(&state, event.clone())
                .map_err(|error| stuck(&state, error.to_string()))?
                .0;
        }
        // An in-stage invalidation (a head change in an approval or fanout
        // stage, collected approvals cleared in place) starts a new attempt.
        if state.status == PipelineStatus::Running
            && state.stage_index == before.0
            && !in_flight
            && ((collecting && state.current_head != head_before)
                || (collected_before
                    && state.approval_reviewers.is_empty()
                    && state.pending_pick.is_none()))
            && state.attempt() <= before.1
        {
            return Err(stuck(
                &state,
                "the current stage was invalidated without a new attempt".into(),
            ));
        }
        // Once the stage has moved on, the same results again are stale: a
        // duplicate must be rejected and change nothing.
        if (state.stage_index, state.attempt(), state.status) != before {
            for event in events.iter().filter(|event| identity(event).is_some()) {
                if step(&state, event.clone()).is_ok() {
                    return Err(stuck(
                        &state,
                        format!("a duplicate result was accepted: {event:?}"),
                    ));
                }
            }
        }
    }
    Err(stuck(&state, "no progress within the bound".into()))
}

/// What must hold when a task is done, with or without merge: every command
/// passed and every head-bound approval covers the final head, and when the
/// last fanout is a pick, its winner is one of its current children.
fn done_invariants(state: &PipelineState) -> Result<(), String> {
    let head = state.current_head.as_deref();
    for stage in &state.workflow.stages {
        match stage.stage {
            Stage::Command { .. }
                if !state
                    .passed_checks
                    .iter()
                    .any(|check| check.stage_id == stage.id && check.head.as_deref() == head) =>
            {
                return Err(format!("done but `{}` never passed on {head:?}", stage.id));
            }
            Stage::Approval {
                bind_head: true, ..
            } if !state.approvals.iter().any(|approval| {
                approval.stage_id == stage.id && approval.head.as_deref() == head
            }) =>
            {
                return Err(format!("done but `{}` does not cover {head:?}", stage.id));
            }
            _ => {}
        }
    }
    let last_fanout = state
        .workflow
        .stages
        .iter()
        .rev()
        .find(|stage| stage.stage.kind() == StageKind::Fanout);
    if let Some(WorkflowStage {
        stage: Stage::Fanout {
            join: FanoutJoin::Pick,
            ..
        },
        id,
        ..
    }) = last_fanout
        && !state
            .selected_fanout_child
            .as_ref()
            .is_some_and(|winner| state.fanout_child_task_ids.contains(winner))
    {
        return Err(format!("done without a pick among the children of `{id}`"));
    }
    Ok(())
}

fn witness_head(counter: &mut u32) -> (String, String) {
    *counter += 1;
    (
        format!("witness-head-{counter}"),
        format!("witness-patch-{counter}"),
    )
}

/// The events that complete the current stage successfully.
fn success_events(state: &PipelineState, counter: &mut u32) -> Vec<PipelineEvent> {
    if state.status == PipelineStatus::Pending {
        return alloc::vec![PipelineEvent::Start];
    }
    let Some(stage) = state.current_stage() else {
        return Vec::new();
    };
    let stage_id = stage.id.clone();
    let attempt = state.attempt();
    let head = state.current_head.clone();
    match &stage.stage {
        Stage::Work { output, .. } => alloc::vec![PipelineEvent::WorkCompleted {
            stage_id,
            attempt,
            product: match output {
                WorkOutput::Branch => {
                    let (head, patch_id) = witness_head(counter);
                    WorkProduct::Branch {
                        branch: "witness-branch".into(),
                        head,
                        patch_id,
                    }
                }
                WorkOutput::Result => WorkProduct::Result {
                    summary: "witness".into(),
                    output: None,
                },
                WorkOutput::Plan => WorkProduct::Plan {
                    items: alloc::vec!["witness-item".into()],
                },
            },
        }],
        // The local forge returns no change id, so `{pr}` cannot expand there.
        Stage::Submit { forge } => alloc::vec![PipelineEvent::Submitted {
            stage_id,
            attempt,
            change_id: (forge != "local").then(|| "witness-change".into()),
        }],
        Stage::Command { .. } => alloc::vec![PipelineEvent::CommandFinished {
            stage_id,
            attempt,
            head,
            exit_code: Some(0),
        }],
        Stage::Approval { count, .. } => {
            let selected_child = pick_fanout_index_for_approval(&state.workflow, state.stage_index)
                .and_then(|_| {
                    state
                        .selected_fanout_child
                        .clone()
                        .or_else(|| state.fanout_child_task_ids.first().cloned())
                });
            (0..*count)
                .map(|reviewer| PipelineEvent::ApprovalGranted {
                    stage_id: stage_id.clone(),
                    attempt,
                    reviewer: format!("witness-reviewer-{reviewer}"),
                    head: head.clone(),
                    selected_child: selected_child.clone(),
                })
                .collect()
        }
        Stage::Fanout { source, join } => {
            let children: Vec<String> = match source {
                FanoutSource::Listed(children) => children.clone(),
                FanoutSource::WorkOutput => {
                    alloc::vec!["witness-child-1".into(), "witness-child-2".into()]
                }
            };
            alloc::vec![PipelineEvent::FanoutCompleted {
                stage_id,
                attempt,
                selected_child: (*join == FanoutJoin::First)
                    .then(|| children.first().cloned())
                    .flatten(),
                child_task_ids: children,
            }]
        }
        Stage::Merge => alloc::vec![PipelineEvent::MergeCompleted {
            stage_id,
            attempt,
            head: head.unwrap_or_default(),
            merge_commit: "witness-merge".into(),
        }],
    }
}

fn disturbance_events(
    state: &PipelineState,
    kind: Disturbance,
    counter: &mut u32,
) -> Vec<PipelineEvent> {
    let Some(stage) = state.current_stage() else {
        return Vec::new();
    };
    let stage_id = stage.id.clone();
    let attempt = state.attempt();
    let head = state.current_head.clone();
    match (kind, stage.stage.kind()) {
        (Disturbance::Fail, StageKind::Command) => alloc::vec![PipelineEvent::CommandFinished {
            stage_id,
            attempt,
            head,
            exit_code: Some(1),
        }],
        (Disturbance::Fail, _) => alloc::vec![PipelineEvent::ChangesRequested {
            stage_id,
            attempt,
            reviewer: "witness-reviewer".into(),
            head,
            reason: "witness".into(),
        }],
        (Disturbance::NewCommit, StageKind::Merge) => {
            let (new_head, patch_id) = witness_head(counter);
            alloc::vec![
                PipelineEvent::CommitCreated {
                    head: new_head,
                    patch_id,
                },
                PipelineEvent::MergeFailed {
                    stage_id,
                    attempt,
                    head: head.unwrap_or_default(),
                    reason: "witness".into(),
                },
            ]
        }
        (Disturbance::NewCommit, _) => {
            let (head, patch_id) = witness_head(counter);
            alloc::vec![PipelineEvent::CommitCreated { head, patch_id }]
        }
    }
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
    if state
        .current_stage()
        .is_some_and(|stage| stage.stage.kind() == kind)
    {
        Ok(())
    } else {
        Err(TransitionError::WrongStage)
    }
}

/// The current stage is an approval; returns its `bind_head` and `count`.
fn current_approval(state: &PipelineState) -> Result<(bool, u8), TransitionError> {
    ensure_current_stage(state, StageKind::Approval)?;
    let Some(Stage::Approval {
        bind_head, count, ..
    }) = state.current_stage().map(|s| &s.stage)
    else {
        return Err(TransitionError::WrongStage);
    };
    if *bind_head && state.current_head.is_none() {
        return Err(TransitionError::MissingHead);
    }
    Ok((*bind_head, *count))
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

/// Forget the approvals of the current approval stage below its count,
/// together with the winner they chose.
fn clear_partial(state: &mut PipelineState) {
    state.approval_reviewers.clear();
    state.pending_pick = None;
}

/// Drop approval records that no longer count. If the pick approval's record
/// goes, its winner goes with it: the pick has to be made again.
fn drop_approvals(state: &mut PipelineState, keep: impl Fn(&Workflow, &ApprovalRecord) -> bool) {
    let workflow = &state.workflow;
    let mut pick_dropped = false;
    state.approvals.retain(|approval| {
        let kept = keep(workflow, approval);
        if !kept
            && stage_position(workflow, &approval.stage_id)
                .is_some_and(|index| pick_fanout_index_for_approval(workflow, index).is_some())
        {
            pick_dropped = true;
        }
        kept
    });
    if pick_dropped {
        state.selected_fanout_child = None;
    }
    clear_partial(state);
}

fn previous_work(state: &PipelineState, before: usize) -> Option<usize> {
    state
        .workflow
        .stages
        .get(..before)?
        .iter()
        .rposition(|stage| stage.stage.kind() == StageKind::Work)
}

fn is_head_bound(stage: &WorkflowStage) -> bool {
    matches!(
        stage.stage,
        Stage::Command { .. }
            | Stage::Approval {
                bind_head: true,
                ..
            }
    )
}

/// The first stage that must be repeated when the head changes while the task
/// is at `current`: the first command or head-bound approval after the most
/// recent branch work stage, up to and including `current`. `None` in a work stage
/// (the task holder is still working) or when nothing before `current` depends on
/// the head.
fn recheck_target(state: &PipelineState, current: usize) -> Option<usize> {
    let stages = &state.workflow.stages;
    if stages.get(current)?.stage.kind() == StageKind::Work {
        return None;
    }
    let start = stages
        .get(..current)?
        .iter()
        .rposition(|stage| {
            matches!(
                stage.stage,
                Stage::Work {
                    output: WorkOutput::Branch,
                    ..
                }
            )
        })
        .map_or(0, |work| work + 1);
    (start..=current).find(|&index| stages.get(index).is_some_and(is_head_bound))
}

fn record_new_head(state: &mut PipelineState, head: String, patch_id: String) {
    if let Some(WorkProduct::Branch {
        head: product_head,
        patch_id: product_patch,
        ..
    }) = &mut state.work_product
    {
        product_head.clone_from(&head);
        product_patch.clone_from(&patch_id);
    }
    state.current_head = Some(head);
    state.patch_id = Some(patch_id);
    invalidate_head_bound(state);
}

/// After a new head: stay put in work (and in a submit with nothing to
/// re-check before it), otherwise go back to the first check to repeat.
fn recheck_for_new_head(
    state: &mut PipelineState,
    actions: &mut Vec<PipelineAction>,
) -> Result<(), TransitionError> {
    match recheck_target(state, state.stage_index) {
        Some(target) => enter_stage(state, target, actions),
        None => restart_if_collected(state, actions),
    }
}

/// The rule for in-stage invalidation: when a head change voids what the
/// current stage has collected (partial approvals and a tentative pick, or
/// a fanout run started for the old head), the stage starts a new attempt and
/// asks again, so results of the old attempt are stale. Work and submit
/// collect nothing a head change voids: the holder's work goes on.
fn restart_if_collected(
    state: &mut PipelineState,
    actions: &mut Vec<PipelineAction>,
) -> Result<(), TransitionError> {
    let collects = state
        .current_stage()
        .is_some_and(|stage| matches!(stage.stage.kind(), StageKind::Approval | StageKind::Fanout));
    if collects && !state.merge_in_flight() {
        enter_stage(state, state.stage_index, actions)
    } else {
        Ok(())
    }
}

fn main_advanced(
    state: &mut PipelineState,
    rebased_head: String,
    patch_id: String,
    conflict: bool,
    actions: &mut Vec<PipelineAction>,
) -> Result<(), TransitionError> {
    let current = state.stage_index;
    let kind = state
        .current_stage()
        .ok_or(TransitionError::InvalidStageIndex)?
        .stage
        .kind();
    // Before any branch exists there is nothing to rebase.
    if state.branch.is_none() {
        return Ok(());
    }
    if !conflict && state.current_head.as_deref() == Some(rebased_head.as_str()) {
        return Ok(());
    }
    // Work and submit: the task has not reached its checks yet. Record a clean
    // rebase and stay; a conflict leaves the branch where it was.
    if matches!(kind, StageKind::Work | StageKind::Submit) {
        if conflict {
            return Ok(());
        }
        record_new_head(state, rebased_head, patch_id);
        return recheck_for_new_head(state, actions);
    }
    let old_head = state
        .current_head
        .clone()
        .ok_or(TransitionError::MissingHead)?;
    let approved_patch = state
        .approvals
        .iter()
        .find(|approval| approval.head.as_deref() == Some(old_head.as_str()))
        .and_then(|approval| approval.patch_id.clone())
        .or_else(|| state.patch_id.clone())
        .ok_or(TransitionError::MissingHead)?;
    let decision = approval_after_rebase(&RebaseOutcome {
        conflict,
        rebased_head: Some(rebased_head.clone()),
        previous_patch_id: approved_patch.clone(),
        rebased_patch_id: (!conflict).then(|| patch_id.clone()),
    });
    match decision {
        ApprovalAfterRebase::Keep { new_approved_head } => {
            // D14: same diff after a clean rebase keeps review, re-runs checks.
            state.passed_checks.clear();
            clear_partial(state);
            for approval in &mut state.approvals {
                if approval.head.as_deref() == Some(old_head.as_str())
                    && approval.patch_id.as_deref() == Some(approved_patch.as_str())
                {
                    approval.head = Some(new_approved_head.clone());
                }
            }
            record_new_head(state, new_approved_head, patch_id);
            recheck_for_new_head(state, actions)
        }
        ApprovalAfterRebase::ReturnToWork => {
            if !conflict {
                record_new_head(state, rebased_head, patch_id);
            } else {
                invalidate_head_bound(state);
            }
            let reason = if conflict {
                "main advanced and the rebase conflicted"
            } else {
                "main advanced and the rebase changed the patch"
            };
            match previous_work(state, current) {
                Some(work) => return_to_work(state, work, reason.into(), actions),
                None => {
                    let stage_id = state.workflow.stages[current].id.clone();
                    fail_task(state, stage_id, reason.into(), actions);
                    Ok(())
                }
            }
        }
    }
}

/// Drop every record that no longer covers the current head: approvals given
/// for another head, checks passed on another head, partial approvals.
fn invalidate_head_bound(state: &mut PipelineState) {
    let head = state.current_head.clone();
    drop_approvals(state, |_, approval| {
        approval.head.is_none() || approval.head == head
    });
    state.passed_checks.retain(|check| check.head == head);
}

fn fanout_at_or_after(state: &PipelineState, index: usize) -> bool {
    state.workflow.stages.get(index..).is_some_and(|stages| {
        stages
            .iter()
            .any(|stage| stage.stage.kind() == StageKind::Fanout)
    })
}

fn clear_fanout(state: &mut PipelineState) {
    state.fanout_child_task_ids.clear();
    state.selected_fanout_child = None;
}

fn approval_for_stage(state: &PipelineState, stage: &WorkflowStage) -> bool {
    let Stage::Approval { bind_head, .. } = stage.stage else {
        return false;
    };
    approval_satisfied(
        bind_head,
        &state.approval_heads(&stage.id),
        state.current_head.as_deref(),
    )
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

/// A stage did not succeed. Commands and approvals go back to the most recent
/// work stage by default (rework to the task holder, D33); `on_fail` names
/// another earlier work stage; with no target the task fails. Only work
/// stages are targets, so the reason always reaches the task holder.
fn fail_stage(
    state: &mut PipelineState,
    failed_index: usize,
    reason: String,
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
            .get(..failed_index)?
            .iter()
            .position(|stage| stage.id == target && stage.stage.kind() == StageKind::Work)
    });
    let default_target = matches!(
        failed.stage.kind(),
        StageKind::Command | StageKind::Approval
    )
    .then(|| previous_work(state, failed_index))
    .flatten();
    let failed_id = failed.id.clone();

    match explicit_target.or(default_target) {
        Some(target) => return_to_work(state, target, reason, actions),
        None => {
            fail_task(state, failed_id, reason, actions);
            Ok(())
        }
    }
}

/// Forget checks and approvals recorded at or after `target`.
fn forget_from(state: &mut PipelineState, target: usize) {
    let workflow = &state.workflow;
    state
        .passed_checks
        .retain(|check| stage_position(workflow, &check.stage_id).is_some_and(|i| i < target));
    drop_approvals(state, |workflow, approval| {
        stage_position(workflow, &approval.stage_id).is_some_and(|i| i < target)
    });
}

/// Hand the work stage at `target` back to the task holder. The branch and
/// head stay: the task holder continues on the same branch.
fn return_to_work(
    state: &mut PipelineState,
    target: usize,
    reason: String,
    actions: &mut Vec<PipelineAction>,
) -> Result<(), TransitionError> {
    forget_from(state, target);
    // Fanout children belong to their fanout: forget them only when the task
    // goes back to (or before) a fanout that will run again.
    if fanout_at_or_after(state, target) {
        clear_fanout(state);
    }
    state.work_product = None;
    let stage = state
        .workflow
        .stages
        .get(target)
        .ok_or(TransitionError::InvalidStageIndex)?;
    let Stage::Work { role, .. } = &stage.stage else {
        return Err(TransitionError::WrongStage);
    };
    state.stage_index = target;
    state.attempts[target] += 1;
    let attempt = state.attempts[target];
    actions.push(PipelineAction::ScheduleTimeout {
        stage_id: stage.id.clone(),
        attempt,
        timeout_ms: stage.effective_timeout_ms(),
        action: stage.effective_timeout_action(),
    });
    actions.push(PipelineAction::ReturnToWork {
        stage_id: stage.id.clone(),
        attempt,
        role: role.clone(),
        reason,
    });
    Ok(())
}

fn fail_task(
    state: &mut PipelineState,
    stage_id: String,
    reason: String,
    actions: &mut Vec<PipelineAction>,
) {
    state.status = PipelineStatus::Failed;
    clear_partial(state);
    actions.push(PipelineAction::TaskFailed { stage_id, reason });
}

fn cancel(
    state: &mut PipelineState,
    stage_id: Option<String>,
    reason: String,
    actions: &mut Vec<PipelineAction>,
) {
    state.status = PipelineStatus::Cancelled;
    clear_partial(state);
    actions.push(PipelineAction::TaskCancelled { stage_id, reason });
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
        // A fanout that runs (again) makes new children: its old children,
        // the pick among them and every later approval no longer count.
        if state
            .workflow
            .stages
            .get(index)
            .is_some_and(|stage| stage.stage.kind() == StageKind::Fanout)
        {
            clear_fanout(state);
            let workflow = &state.workflow;
            state.approvals.retain(|approval| {
                stage_position(workflow, &approval.stage_id).is_some_and(|i| i < index)
            });
        }
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
        state.pending_pick = None;

        if approval_for_stage(state, stage) {
            index += 1;
            continue;
        }

        // Entering the stage starts a new attempt; results must echo it.
        state.attempts[index] += 1;
        let attempt = state.attempts[index];
        let action = match &stage.stage {
            Stage::Work { role, .. } => PipelineAction::AssignWork {
                stage_id: stage.id.clone(),
                attempt,
                role: role.clone(),
            },
            Stage::Submit { forge } => PipelineAction::Submit {
                stage_id: stage.id.clone(),
                attempt,
                forge: forge.clone(),
            },
            Stage::Command { command } => {
                let context = CommandContext {
                    pr: state.change_id.as_deref(),
                    head: state.current_head.as_deref(),
                    branch: state.branch.as_deref(),
                };
                match expand_command_placeholders(command, context) {
                    Ok(command) => PipelineAction::RunCommand {
                        stage_id: stage.id.clone(),
                        attempt,
                        command,
                        head: state.current_head.clone(),
                        branch: state.branch.clone(),
                        change_id: state.change_id.clone(),
                        timeout_ms: stage.effective_timeout_ms(),
                        timeout_action: stage.effective_timeout_action(),
                    },
                    Err(error) => {
                        let stage_id = stage.id.clone();
                        fail_task(state, stage_id, error.to_string(), actions);
                        return Ok(());
                    }
                }
            }
            Stage::Approval { bind_head, .. } => PipelineAction::RequestApproval {
                stage_id: stage.id.clone(),
                attempt,
                bind_head: *bind_head,
                head: if *bind_head {
                    state.current_head.clone()
                } else {
                    None
                },
                choices: if pick_fanout_index_for_approval(&state.workflow, index).is_some() {
                    state.fanout_child_task_ids.clone()
                } else {
                    Vec::new()
                },
            },
            Stage::Merge => {
                if !state.merge_gate(index).allowed {
                    return Err(TransitionError::MergeGateClosed);
                }
                PipelineAction::Merge {
                    stage_id: stage.id.clone(),
                    attempt,
                    head: state
                        .current_head
                        .clone()
                        .ok_or(TransitionError::MissingHead)?,
                }
            }
            Stage::Fanout { source, join } => PipelineAction::Fanout {
                stage_id: stage.id.clone(),
                attempt,
                source: source.clone(),
                join: *join,
                work_product: state.work_product.clone(),
                head: state.current_head.clone(),
            },
        };
        actions.push(PipelineAction::ScheduleTimeout {
            stage_id: stage.id.clone(),
            attempt,
            timeout_ms: stage.effective_timeout_ms(),
            action: stage.effective_timeout_action(),
        });
        actions.push(action);
        return Ok(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::workflow::Approver;
    use alloc::vec;

    fn roles() -> Vec<String> {
        ["dev", "reviewer", "researcher", "planner"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    fn state_for(workflow: Workflow) -> PipelineState {
        PipelineState::new("T-1", workflow.validated(&roles()).unwrap())
    }

    fn initial() -> PipelineState {
        state_for(Workflow::builtin_code())
    }

    fn work(state: &PipelineState, head: &str, patch: &str) -> PipelineEvent {
        PipelineEvent::WorkCompleted {
            stage_id: stage_id(state),
            attempt: state.attempt(),
            product: WorkProduct::Branch {
                branch: "agend/T-1/demo".into(),
                head: head.into(),
                patch_id: patch.into(),
            },
        }
    }

    fn submitted(state: &PipelineState) -> PipelineEvent {
        PipelineEvent::Submitted {
            stage_id: stage_id(state),
            attempt: state.attempt(),
            change_id: None,
        }
    }

    fn command(state: &PipelineState, exit_code: Option<i32>) -> PipelineEvent {
        PipelineEvent::CommandFinished {
            stage_id: stage_id(state),
            attempt: state.attempt(),
            head: state.current_head.clone(),
            exit_code,
        }
    }

    fn approve(state: &PipelineState, reviewer: &str) -> PipelineEvent {
        PipelineEvent::ApprovalGranted {
            stage_id: stage_id(state),
            attempt: state.attempt(),
            reviewer: reviewer.into(),
            head: state.current_head.clone(),
            selected_child: None,
        }
    }

    fn request_changes(state: &PipelineState) -> PipelineEvent {
        PipelineEvent::ChangesRequested {
            stage_id: stage_id(state),
            attempt: state.attempt(),
            reviewer: "reviewer-1".into(),
            head: state.current_head.clone(),
            reason: "rename the flag".into(),
        }
    }

    fn commit(head: &str, patch: &str) -> PipelineEvent {
        PipelineEvent::CommitCreated {
            head: head.into(),
            patch_id: patch.into(),
        }
    }

    fn main_advanced(head: &str, patch: &str, conflict: bool) -> PipelineEvent {
        PipelineEvent::MainAdvanced {
            rebased_head: head.into(),
            patch_id: patch.into(),
            conflict,
        }
    }

    fn merged(state: &PipelineState, head: &str) -> PipelineEvent {
        PipelineEvent::MergeCompleted {
            stage_id: stage_id(state),
            attempt: state.attempt(),
            head: head.into(),
            merge_commit: "M1".into(),
        }
    }

    fn stage_id(state: &PipelineState) -> String {
        state
            .current_stage()
            .map_or_else(|| "none".into(), |stage| stage.id.clone())
    }

    fn apply(state: &PipelineState, event: PipelineEvent) -> (PipelineState, Vec<PipelineAction>) {
        step(state, event).unwrap()
    }

    fn merge_failed(state: &PipelineState) -> PipelineEvent {
        PipelineEvent::MergeFailed {
            stage_id: stage_id(state),
            attempt: state.attempt(),
            head: state.current_head.clone().unwrap(),
            reason: "main moved".into(),
        }
    }

    /// Apply a head change while the merge is in flight: the task must stay
    /// in the merge stage with no action; then the forge reports the merge
    /// failed and the pending change is applied.
    fn through_merge(
        state: &PipelineState,
        event: PipelineEvent,
    ) -> (PipelineState, Vec<PipelineAction>) {
        assert!(state.merge_in_flight());
        let (held, actions) = apply(state, event);
        assert_eq!(held.stage_index, state.stage_index);
        assert_eq!(held.current_head, state.current_head);
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(held.pending_head_changes.len(), 1);
        apply(&held, merge_failed(state))
    }

    fn at_submit(workflow: Workflow) -> PipelineState {
        let state = apply(&state_for(workflow), PipelineEvent::Start).0;
        apply(&state, work(&state, "H1", "P1")).0
    }

    fn run_to_merge(state: PipelineState, head: &str, patch: &str) -> PipelineState {
        let mut state = apply(&state, PipelineEvent::Start).0;
        state = apply(&state, work(&state, head, patch)).0;
        state = apply(&state, submitted(&state)).0;
        state = apply(&state, command(&state, Some(0))).0;
        apply(&state, approve(&state, "reviewer-1")).0
    }

    fn approval_stage(id: &str, bind_head: bool) -> WorkflowStage {
        WorkflowStage::new(
            id,
            Stage::Approval {
                by: Approver::Human,
                count: 1,
                bind_head,
            },
        )
    }

    #[test]
    fn code_workflow_requires_checks_and_head_bound_approval() {
        let mut state = at_submit(Workflow::builtin_code());
        state = apply(&state, submitted(&state)).0;
        assert_eq!(
            step(&state, approve(&state, "reviewer-1")),
            Err(TransitionError::WrongStage)
        );
        state = apply(&state, command(&state, Some(0))).0;
        state = apply(&state, approve(&state, "reviewer-1")).0;
        let (done, actions) = apply(&state, merged(&state, "H1"));
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
        let mut state = at_submit(workflow);
        state = apply(&state, submitted(&state)).0;
        state = apply(&state, command(&state, Some(0))).0;
        assert_eq!(stage_id(&state), "lint");
        state = apply(&state, command(&state, Some(0))).0;
        state = apply(&state, approve(&state, "reviewer-1")).0;
        assert!(matches!(state.current_stage().unwrap().stage, Stage::Merge));
        state
            .passed_checks
            .retain(|check| check.stage_id != "checks");
        assert_eq!(
            step(&state, merged(&state, "H1")),
            Err(TransitionError::MergeGateClosed)
        );
    }

    /// Round-1 review B2: a passing command must not skip a later approval.
    #[test]
    fn review_b2_approval_on_a_later_stage_is_never_skipped() {
        let mut workflow = Workflow::builtin_code();
        let reviewer = workflow.stages.remove(3);
        workflow.stages.insert(2, reviewer);
        workflow.stages.insert(4, approval_stage("human", true));
        let mut state = at_submit(workflow);
        state = apply(&state, submitted(&state)).0;
        state = apply(&state, approve(&state, "reviewer-1")).0;
        state = apply(&state, command(&state, Some(0))).0;
        assert_eq!(stage_id(&state), "human");
        state = apply(&state, approve(&state, "operator")).0;
        assert!(matches!(state.current_stage().unwrap().stage, Stage::Merge));
    }

    /// Round-1 review B3: a command result for an old head is stale.
    #[test]
    fn review_b3_stale_command_result_cannot_pass_checks_for_a_new_head() {
        let state = run_to_merge(initial(), "H1", "P1");
        let (updated, _) = through_merge(&state, commit("H2", "P2"));
        assert_eq!(
            step(
                &updated,
                PipelineEvent::CommandFinished {
                    attempt: updated.attempt(),
                    stage_id: "checks".into(),
                    head: Some("H1".into()),
                    exit_code: Some(0)
                }
            ),
            Err(TransitionError::StaleResult)
        );
    }

    /// Round-1 review I1/I2: main may advance during checks; the approved
    /// patch (not a later, unapproved one) decides whether review is kept.
    #[test]
    fn review_i1_i2_main_advance_uses_the_approved_patch_and_works_during_checks() {
        let mut state = run_to_merge(initial(), "H1", "P1");
        state.stage_index = 2;
        state.patch_id = Some("stale-unapproved-patch".into());
        let (rebased, _) = apply(&state, main_advanced("H1-rebased", "P1", false));
        assert_eq!(rebased.current_head.as_deref(), Some("H1-rebased"));
        assert_eq!(stage_id(&rebased), "checks");
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
        let mut state = at_submit(workflow);
        state = apply(&state, submitted(&state)).0;
        state = apply(&state, command(&state, Some(0))).0;
        state = apply(&state, command(&state, Some(0))).0;
        state = apply(&state, approve(&state, "reviewer-1")).0;
        let (rebased, _) = through_merge(&state, main_advanced("H1-rebased", "P1", false));
        assert_eq!(stage_id(&rebased), "checks");
        assert!(rebased.passed_checks.is_empty());
        assert!(
            rebased
                .approvals
                .iter()
                .any(|approval| approval.head.as_deref() == Some("H1-rebased"))
        );
        let rebased = apply(&rebased, command(&rebased, Some(0))).0;
        assert_eq!(stage_id(&rebased), "lint");
        let rebased = apply(&rebased, command(&rebased, Some(0))).0;
        assert!(matches!(
            rebased.current_stage().unwrap().stage,
            Stage::Merge
        ));
    }

    /// Round-1 review I1/I3: an allow_unreviewed workflow can be rebased, and
    /// the rebase does not skip submit.
    #[test]
    fn review_i1_i3_allow_unreviewed_workflow_rebases_without_an_approval_record() {
        let mut workflow = Workflow::builtin_code();
        workflow.allow_unreviewed = true;
        workflow
            .stages
            .retain(|stage| !matches!(stage.stage, Stage::Approval { .. }));
        let state = at_submit(workflow);
        let (rebased, actions) = apply(&state, main_advanced("H2", "P1", false));
        assert_eq!(rebased.current_head.as_deref(), Some("H2"));
        assert_eq!(stage_id(&rebased), "submit");
        assert!(actions.is_empty());
        let (checking, actions) = apply(&rebased, submitted(&rebased));
        assert_eq!(stage_id(&checking), "checks");
        assert!(actions.iter().any(|action| matches!(
            action,
            PipelineAction::RunCommand { head: Some(head), .. } if head == "H2"
        )));
        let (merging, _) = apply(&checking, command(&checking, Some(0)));
        assert_eq!(
            apply(&merging, merged(&merging, "H2")).0.status,
            PipelineStatus::Done
        );
    }

    /// Round-1 review I3: a new commit during checks restarts the checks for
    /// the new head.
    #[test]
    fn review_i3_commit_during_checks_restarts_them_on_the_new_head() {
        let state = {
            let submitting = at_submit(Workflow::builtin_code());
            apply(&submitting, submitted(&submitting))
        }
        .0;
        let (state, actions) = apply(&state, commit("H2", "P2"));
        assert_eq!(stage_id(&state), "checks");
        assert!(actions.iter().any(|action| matches!(
            action,
            PipelineAction::RunCommand { head: Some(head), .. } if head == "H2"
        )));
    }

    #[test]
    fn changed_patch_after_main_advance_returns_to_work() {
        let state = run_to_merge(initial(), "H1", "P1");
        let (changed, actions) = through_merge(&state, main_advanced("H1-other", "P2", false));
        assert!(
            changed
                .approvals
                .iter()
                .all(|approval| approval.head.is_none())
        );
        assert_eq!(stage_id(&changed), "work");
        assert!(actions.iter().any(|action| matches!(
            action,
            PipelineAction::ReturnToWork { stage_id, .. } if stage_id == "work"
        )));
    }

    /// Round-2 review N1: a head change while waiting for submit must not
    /// jump to checks; `Submitted` is still required.
    #[test]
    fn review_n1_head_change_in_submit_does_not_skip_submit() {
        for event in [main_advanced("H2", "P1", false), commit("H2", "P2")] {
            let (state, actions) = apply(&at_submit(Workflow::builtin_code()), event.clone());
            assert_eq!(stage_id(&state), "submit", "{event:?}");
            assert_eq!(state.current_head.as_deref(), Some("H2"));
            assert!(actions.is_empty());
            assert_eq!(
                step(&state, command(&state, Some(0))),
                Err(TransitionError::WrongStage)
            );
            let (state, _) = apply(&state, submitted(&state));
            assert_eq!(stage_id(&state), "checks");
        }
    }

    /// Round-2 review N2: a head change during rework keeps the task in work;
    /// the task holder's `WorkCompleted` is still accepted afterwards.
    #[test]
    fn review_n2_head_change_during_rework_keeps_the_task_in_work() {
        let state = run_to_merge(initial(), "H1", "P1");
        let (state, _) = through_merge(&state, main_advanced("H2", "P9", false));
        assert_eq!(stage_id(&state), "work");
        for event in [
            main_advanced("H3", "P9", false),
            commit("H4", "P10"),
            main_advanced("H5", "P10", true),
        ] {
            let (next, actions) = apply(&state, event.clone());
            assert_eq!(stage_id(&next), "work", "{event:?}");
            assert!(actions.is_empty(), "{event:?}: {actions:?}");
        }
        let (state, _) = apply(&state, commit("H4", "P10"));
        let (state, _) = apply(&state, work(&state, "H6", "P11"));
        assert_eq!(stage_id(&state), "submit");
    }

    /// Round-2 review N3: requested changes return the task to the task holder's
    /// work stage, not TaskFailed (D18 rework).
    #[test]
    fn review_n3_changes_requested_return_to_the_task_holder() {
        let state = {
            let submitting = at_submit(Workflow::builtin_code());
            apply(&submitting, submitted(&submitting))
        }
        .0;
        let state = apply(&state, command(&state, Some(0))).0;
        assert_eq!(stage_id(&state), "review");
        let (reworked, actions) = apply(&state, request_changes(&state));
        assert_eq!(reworked.status, PipelineStatus::Running);
        assert_eq!(stage_id(&reworked), "work");
        assert!(actions.contains(&PipelineAction::ReturnToWork {
            stage_id: "work".into(),
            attempt: 2,
            role: "dev".into(),
            reason: "changes requested by reviewer-1: rename the flag".into(),
        }));
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, PipelineAction::TaskFailed { .. }))
        );
        // A plain stage failure of the approval behaves the same way.
        let (reworked, _) = apply(
            &state,
            PipelineEvent::StageFailed {
                attempt: state.attempt(),
                stage_id: "review".into(),
                reason: "reviewer rejected".into(),
            },
        );
        assert_eq!(stage_id(&reworked), "work");
    }

    #[test]
    fn changes_requested_for_an_old_head_or_stage_is_stale() {
        let state = {
            let submitting = at_submit(Workflow::builtin_code());
            apply(&submitting, submitted(&submitting))
        }
        .0;
        let state = apply(&state, command(&state, Some(0))).0;
        let stale_head = PipelineEvent::ChangesRequested {
            attempt: state.attempt(),
            stage_id: "review".into(),
            reviewer: "reviewer-1".into(),
            head: Some("H0".into()),
            reason: "old".into(),
        };
        assert_eq!(step(&state, stale_head), Err(TransitionError::StaleResult));
        let stale_stage = PipelineEvent::ApprovalGranted {
            attempt: state.attempt(),
            stage_id: "checks".into(),
            reviewer: "reviewer-1".into(),
            head: Some("H1".into()),
            selected_child: None,
        };
        assert_eq!(step(&state, stale_stage), Err(TransitionError::StaleResult));
    }

    #[test]
    fn approval_on_failure_uses_on_fail_when_set() {
        let mut workflow = Workflow::builtin_planned();
        workflow.stages[5].on_fail = Some("plan".into());
        let mut state = apply(&state_for(workflow), PipelineEvent::Start).0;
        state = apply(&state, result_work(&state)).0;
        state = apply(&state, approve(&state, "owner")).0;
        state = apply(&state, work(&state, "H1", "P1")).0;
        state = apply(&state, submitted(&state)).0;
        state = apply(&state, command(&state, Some(0))).0;
        let (state, actions) = apply(&state, request_changes(&state));
        assert_eq!(stage_id(&state), "plan");
        assert!(actions.iter().any(|action| matches!(
            action,
            PipelineAction::ReturnToWork { stage_id, .. } if stage_id == "plan"
        )));
    }

    fn result_work(state: &PipelineState) -> PipelineEvent {
        PipelineEvent::WorkCompleted {
            stage_id: stage_id(state),
            attempt: state.attempt(),
            product: WorkProduct::Result {
                summary: "plan".into(),
                output: None,
            },
        }
    }

    fn reviewer_stage(id: &str, count: u8, bind_head: bool) -> WorkflowStage {
        WorkflowStage::new(
            id,
            Stage::Approval {
                by: Approver::Role("reviewer".into()),
                count,
                bind_head,
            },
        )
    }

    fn command_stage(id: &str) -> WorkflowStage {
        WorkflowStage::new(
            id,
            Stage::Command {
                command: format!("run {id}"),
            },
        )
    }

    /// work, submit, c1, a (unbound), b (bound, count 2), c2, merge.
    fn mixed_workflow() -> Workflow {
        let mut workflow = Workflow::builtin_code();
        workflow.stages = vec![
            workflow.stages[0].clone(),
            workflow.stages[1].clone(),
            command_stage("c1"),
            reviewer_stage("a", 1, false),
            reviewer_stage("b", 2, true),
            command_stage("c2"),
            WorkflowStage::new("merge", Stage::Merge),
        ];
        workflow
    }

    /// Fresh verifier scenario: mixed bound/unbound approvals, a command after
    /// an approval, stale results, rework with the same head, and no cancel
    /// once the merge is requested.
    #[test]
    fn verifier_mixed_approvals_rework_with_same_head_and_merge_in_flight() {
        let mut s = apply(&state_for(mixed_workflow()), PipelineEvent::Start).0;
        s = apply(&s, work(&s, "H1", "P1")).0;
        s = apply(&s, submitted(&s)).0;
        s = apply(&s, command(&s, Some(0))).0;
        s = apply(&s, approve(&s, "r1")).0;
        assert_eq!(stage_id(&s), "b");
        s = apply(&s, approve(&s, "r1")).0;
        s = apply(&s, approve(&s, "r1")).0;
        assert_eq!(
            stage_id(&s),
            "b",
            "a duplicate reviewer must not count twice"
        );
        s = apply(&s, approve(&s, "r2")).0;
        assert_eq!(stage_id(&s), "c2");
        s = apply(&s, commit("H2", "P2")).0;
        assert_eq!(stage_id(&s), "c1");
        let stale = PipelineEvent::CommandFinished {
            attempt: s.attempt(),
            stage_id: "c1".into(),
            head: Some("H1".into()),
            exit_code: Some(0),
        };
        assert!(step(&s, stale).is_err());
        s = apply(&s, command(&s, Some(0))).0;
        assert_eq!(
            stage_id(&s),
            "b",
            "unbound a is kept, bound b is asked again"
        );
        let old_head = PipelineEvent::ApprovalGranted {
            attempt: s.attempt(),
            stage_id: "b".into(),
            reviewer: "r1".into(),
            head: Some("H1".into()),
            selected_child: None,
        };
        assert!(step(&s, old_head).is_err());
        s = apply(&s, approve(&s, "r1")).0;
        s = apply(&s, approve(&s, "r2")).0;
        assert_eq!(stage_id(&s), "c2");
        let (reworked, actions) = apply(&s, command(&s, Some(3)));
        assert_eq!(stage_id(&reworked), "work");
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, PipelineAction::ReturnToWork { .. }))
        );
        assert!(reworked.approvals().is_empty() && reworked.passed_checks().is_empty());
        // Rework with the same head still re-runs everything and re-asks a and b.
        let mut s = apply(&reworked, work(&reworked, "H2", "P2")).0;
        s = apply(&s, submitted(&s)).0;
        s = apply(&s, command(&s, Some(0))).0;
        assert_eq!(
            stage_id(&s),
            "a",
            "unbound approval is asked again after rework"
        );
        s = apply(&s, approve(&s, "r1")).0;
        s = apply(&s, approve(&s, "r1")).0;
        s = apply(&s, approve(&s, "r3")).0;
        let (s, actions) = apply(&s, command(&s, Some(0)));
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, PipelineAction::Merge { head, .. } if head == "H2"))
        );
        let cancel = PipelineEvent::Cancel {
            reason: "operator".into(),
        };
        assert_eq!(step(&s, cancel), Err(TransitionError::MergeInFlight));
        assert!(step(&s, merged(&s, "H1")).is_err());
        let (done, _) = apply(&s, merged(&s, "H2"));
        assert_eq!(done.status(), PipelineStatus::Done);
        for event in [
            PipelineEvent::Start,
            PipelineEvent::Cancel { reason: "x".into() },
            commit("H9", "P9"),
            main_advanced("H9", "P2", false),
            merged(&s, "H2"),
        ] {
            assert!(step(&done, event).is_err());
        }
    }

    /// Fresh verifier scenario: MainAdvanced (same or changed patch, with or
    /// without conflict) in every stage never moves forward, never acts in
    /// work, and the task can still finish with every check and approval
    /// covering the merged head.
    #[test]
    fn verifier_main_advanced_in_every_stage_keeps_the_gate() {
        for target in 0..7 {
            for same_patch in [true, false] {
                for conflict in [false, true] {
                    let mut s = apply(&state_for(mixed_workflow()), PipelineEvent::Start).0;
                    while s.stage_index < target {
                        let event = match s.current_stage().unwrap().stage.kind() {
                            StageKind::Work => work(&s, "H1", "P1"),
                            StageKind::Submit => PipelineEvent::Submitted {
                                stage_id: stage_id(&s),
                                attempt: s.attempt(),
                                change_id: Some("7".into()),
                            },
                            StageKind::Command => command(&s, Some(0)),
                            _ => approve(
                                &s,
                                if s.approval_reviewers.is_empty() {
                                    "r1"
                                } else {
                                    "r2"
                                },
                            ),
                        };
                        s = apply(&s, event).0;
                    }
                    let before = s.clone();
                    let patch = if same_patch { "P1" } else { "P9" };
                    let Ok((mut s, actions)) = step(&s, main_advanced("H9", patch, conflict))
                    else {
                        continue;
                    };
                    if before.merge_in_flight() {
                        assert_eq!(s.stage_index, before.stage_index, "left an in-flight merge");
                        assert!(actions.is_empty());
                        s = apply(&s, merge_failed(&before)).0;
                    }
                    assert!(s.stage_index <= before.stage_index, "moved forward");
                    if before.current_stage().unwrap().stage.kind() == StageKind::Work {
                        assert_eq!(s.stage_index, before.stage_index);
                        assert!(actions.is_empty());
                    }
                    let mut passed: Vec<(String, String)> = Vec::new();
                    let mut approved_b: Vec<String> = Vec::new();
                    for _ in 0..20 {
                        if s.status != PipelineStatus::Running {
                            break;
                        }
                        let head = s.current_head.clone();
                        let event = match s.current_stage().unwrap().stage.kind() {
                            StageKind::Work => work(&s, "H10", "P10"),
                            StageKind::Submit => submitted(&s),
                            StageKind::Command => {
                                passed.push((stage_id(&s), head.clone().unwrap()));
                                command(&s, Some(0))
                            }
                            StageKind::Approval => {
                                if stage_id(&s) == "b" {
                                    approved_b.push(head.clone().unwrap());
                                }
                                let reviewer = if s.approval_reviewers.is_empty() {
                                    "r1"
                                } else {
                                    "r2"
                                };
                                approve(&s, reviewer)
                            }
                            _ => {
                                let head = head.unwrap();
                                assert!(passed.contains(&("c1".into(), head.clone())));
                                assert!(passed.contains(&("c2".into(), head.clone())));
                                assert!(
                                    approved_b.contains(&head)
                                        || (same_patch && !conflict && target >= 5),
                                    "merge of {head} without approval b"
                                );
                                merged(&s, &head)
                            }
                        };
                        s = apply(&s, event).0;
                    }
                    assert_eq!(s.status, PipelineStatus::Done, "target {target}");
                }
            }
        }
    }

    /// Fresh verifier scenario: pick approval needs a winner, a conflicting
    /// winner is rejected, changes requested return to the plan with the
    /// fanout cleared, and a cancel timeout cancels.
    #[test]
    fn verifier_pick_winner_rules_rework_and_timeout() {
        let mut workflow = Workflow::builtin_epic();
        workflow.stages[1].stage = Stage::Fanout {
            source: FanoutSource::WorkOutput,
            join: FanoutJoin::Pick,
        };
        workflow.stages[2] = WorkflowStage {
            on_timeout: Some(TimeoutAction::Cancel),
            ..WorkflowStage::new(
                "review",
                Stage::Approval {
                    by: Approver::Human,
                    count: 2,
                    bind_head: false,
                },
            )
        };
        let mut s = apply(&state_for(workflow), PipelineEvent::Start).0;
        s = apply(
            &s,
            PipelineEvent::WorkCompleted {
                stage_id: s
                    .current_stage()
                    .map_or_else(String::new, |stage| stage.id.clone()),
                attempt: s.attempt(),
                product: WorkProduct::Plan {
                    items: vec!["a".into()],
                },
            },
        )
        .0;
        s = apply(
            &s,
            PipelineEvent::FanoutCompleted {
                attempt: s.attempt(),
                stage_id: "fanout".into(),
                child_task_ids: vec!["c1".into(), "c2".into()],
                selected_child: None,
            },
        )
        .0;
        assert!(step(&s, approve(&s, "r1")).is_err());
        let pick = |reviewer: &str, child: &str| PipelineEvent::ApprovalGranted {
            attempt: s.attempt(),
            stage_id: "review".into(),
            reviewer: reviewer.into(),
            head: None,
            selected_child: Some(child.into()),
        };
        let s1 = apply(&s, pick("r1", "c1")).0;
        assert!(step(&s1, pick("r2", "c2")).is_err());
        let (back, _) = apply(&s1, request_changes(&s1));
        assert_eq!(stage_id(&back), "plan");
        assert!(back.fanout_child_task_ids().is_empty() && back.selected_fanout_child().is_none());
        let (timed_out, _) = apply(
            &s1,
            PipelineEvent::StageTimedOut {
                attempt: s1.attempt(),
                stage_id: "review".into(),
            },
        );
        assert_eq!(timed_out.status(), PipelineStatus::Cancelled);
        assert!(
            step(
                &s1,
                PipelineEvent::StageTimedOut {
                    attempt: s1.attempt(),
                    stage_id: "plan".into()
                }
            )
            .is_err()
        );
    }

    /// Verifier r1 (important): a head change after the merge request went
    /// out must not pull the task out of the merge stage; the forge's result
    /// for the sent head decides. Cancel stays rejected.
    #[test]
    fn verifier_r1_head_change_while_merge_in_flight_waits_for_the_forge() {
        let state = run_to_merge(initial(), "H1", "P1");
        assert!(state.merge_in_flight());
        let (held, actions) = apply(&state, main_advanced("H2", "P1", false));
        assert!(actions.is_empty());
        assert_eq!(stage_id(&held), "merge");
        assert_eq!(held.current_head(), Some("H1"));
        let (held, _) = apply(&held, commit("H3", "P3"));
        assert_eq!(held.pending_head_changes().len(), 2);
        assert_eq!(
            step(&held, PipelineEvent::Cancel { reason: "x".into() }),
            Err(TransitionError::MergeInFlight)
        );
        let (done, actions) = apply(&held, merged(&held, "H1"));
        assert_eq!(done.status(), PipelineStatus::Done);
        assert!(done.pending_head_changes().is_empty());
        assert_eq!(
            actions,
            [PipelineAction::TaskDone {
                merge_commit: Some("M1".into())
            }]
        );
    }

    #[test]
    fn merge_failure_applies_pending_head_changes_or_rechecks() {
        let state = run_to_merge(initial(), "H1", "P1");
        // Stale failure for another head.
        assert_eq!(
            step(
                &state,
                PipelineEvent::MergeFailed {
                    stage_id: state
                        .current_stage()
                        .map_or_else(String::new, |stage| stage.id.clone()),
                    attempt: state.attempt(),
                    head: "H0".into(),
                    reason: "x".into()
                }
            ),
            Err(TransitionError::StaleResult)
        );
        // No pending change: repeat the checks for the same head; the
        // approval still covers it, so the merge is requested again after.
        let (retry, actions) = apply(&state, merge_failed(&state));
        assert_eq!(stage_id(&retry), "checks");
        assert!(actions.iter().any(
            |action| matches!(action, PipelineAction::RunCommand { head: Some(head), .. } if head == "H1")
        ));
        let (again, actions) = apply(&retry, command(&retry, Some(0)));
        assert!(actions.iter().any(|action| matches!(
            action,
            PipelineAction::Merge { stage_id, head, .. } if stage_id == "merge" && head == "H1"
        )));
        assert!(again.merge_in_flight());
        // Pending clean same-patch rebase: D14 keeps the approval, checks rerun.
        let (held, _) = apply(&state, main_advanced("H2", "P1", false));
        let (rebased, _) = apply(&held, merge_failed(&state));
        assert_eq!(stage_id(&rebased), "checks");
        assert_eq!(rebased.current_head(), Some("H2"));
        assert!(
            rebased
                .approvals()
                .iter()
                .any(|approval| approval.head.as_deref() == Some("H2"))
        );
        // Pending changed patch: back to the task holder.
        let (held, _) = apply(&state, main_advanced("H2", "P9", false));
        let (reworked, _) = apply(&held, merge_failed(&state));
        assert_eq!(stage_id(&reworked), "work");
    }

    #[test]
    fn cancel_timeout_on_the_merge_stage_is_rejected_while_the_merge_is_in_flight() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[4].on_timeout = Some(TimeoutAction::Cancel);
        let state = run_to_merge(state_for(workflow), "H1", "P1");
        assert_eq!(
            step(
                &state,
                PipelineEvent::StageTimedOut {
                    attempt: state.attempt(),
                    stage_id: "merge".into()
                }
            ),
            Err(TransitionError::MergeInFlight)
        );
    }

    /// Round-2 review N4: the merge gate needs every approval stage, not any.
    #[test]
    fn review_n4_merge_requires_every_approval_stage_for_the_current_head() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[3].id = "a".into();
        workflow.stages.insert(4, approval_stage("c", false));
        workflow.stages.insert(4, approval_stage("b", true));
        let mut state = state_for(workflow);
        state.status = PipelineStatus::Running;
        state.stage_index = 6;
        state.current_head = Some("H1".into());
        state.patch_id = Some("P1".into());
        state.passed_checks.push(PassedCheck {
            stage_id: "checks".into(),
            head: Some("H1".into()),
        });
        let record = |stage_id: &str, head: Option<&str>| ApprovalRecord {
            stage_id: stage_id.into(),
            head: head.map(Into::into),
            patch_id: None,
            reviewers: vec!["r".into()],
        };
        state.approvals.push(record("a", Some("H1")));
        assert_eq!(
            step(&state, merged(&state, "H1")),
            Err(TransitionError::MergeGateClosed)
        );
        state.approvals.push(record("b", Some("H0")));
        state.approvals.push(record("c", None));
        assert_eq!(
            step(&state, merged(&state, "H1")),
            Err(TransitionError::MergeGateClosed)
        );
        state.approvals.push(record("b", Some("H1")));
        assert_eq!(
            apply(&state, merged(&state, "H1")).0.status,
            PipelineStatus::Done
        );
    }

    /// Round-2 review N8: RunCommand carries the expanded command and the
    /// change id returned by submit.
    #[test]
    fn review_n8_run_command_carries_the_expanded_command_and_change_id() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[1].stage = Stage::Submit {
            forge: "github".into(),
        };
        workflow.stages[2].stage = Stage::Command {
            command: "gh pr checks {pr} --head {head}".into(),
        };
        let state = at_submit(workflow);
        let (_, actions) = apply(
            &state,
            PipelineEvent::Submitted {
                stage_id: state
                    .current_stage()
                    .map_or_else(String::new, |stage| stage.id.clone()),
                attempt: state.attempt(),
                change_id: Some("42".into()),
            },
        );
        assert!(actions.contains(&PipelineAction::RunCommand {
            stage_id: "checks".into(),
            attempt: 1,
            command: "gh pr checks '42' --head 'H1'".into(),
            head: Some("H1".into()),
            branch: Some("agend/T-1/demo".into()),
            change_id: Some("42".into()),
            timeout_ms: 300_000,
            timeout_action: TimeoutAction::Notify,
        }));
        let (failed, actions) = apply(&state, submitted(&state));
        assert_eq!(failed.status, PipelineStatus::Failed);
        assert!(actions.contains(&PipelineAction::TaskFailed {
            stage_id: "checks".into(),
            reason: "command placeholder `{pr}` has no value".into(),
        }));
    }

    /// Round-2 review N9: cancellation is its own status, from an operator
    /// event or a cancel timeout, and is terminal.
    #[test]
    fn review_n9_cancel_is_its_own_terminal_status() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[0].on_timeout = Some(TimeoutAction::Cancel);
        let state = apply(&state_for(workflow), PipelineEvent::Start).0;
        let (timed_out, actions) = apply(
            &state,
            PipelineEvent::StageTimedOut {
                attempt: state.attempt(),
                stage_id: "work".into(),
            },
        );
        assert_eq!(timed_out.status, PipelineStatus::Cancelled);
        assert!(actions.contains(&PipelineAction::TaskCancelled {
            stage_id: Some("work".into()),
            reason: "stage `work` timed out".into(),
        }));

        let cancel = PipelineEvent::Cancel {
            reason: "operator".into(),
        };
        let (cancelled, actions) = apply(&state, cancel.clone());
        assert_eq!(cancelled.status, PipelineStatus::Cancelled);
        assert_eq!(
            actions,
            [PipelineAction::TaskCancelled {
                stage_id: Some("work".into()),
                reason: "operator".into(),
            }]
        );
        assert_eq!(
            apply(&initial(), cancel.clone()).0.status,
            PipelineStatus::Cancelled
        );
        assert_eq!(step(&cancelled, cancel), Err(TransitionError::NotRunning));
        assert_eq!(
            step(&cancelled, work(&cancelled, "H1", "P1")),
            Err(TransitionError::NotRunning)
        );
    }

    #[test]
    fn timeout_notify_and_reassign_keep_the_pipeline_running() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[0].on_timeout = Some(TimeoutAction::Reassign);
        let state = apply(&state_for(workflow), PipelineEvent::Start).0;
        let (state, actions) = apply(
            &state,
            PipelineEvent::StageTimedOut {
                attempt: state.attempt(),
                stage_id: "work".into(),
            },
        );
        assert_eq!(state.status, PipelineStatus::Running);
        assert!(actions.contains(&PipelineAction::ReassignStage {
            stage_id: "work".into()
        }));
        // Verifier r5 low 3: reassign is a new attempt, re-requested; the old
        // holder's result and a replay of the timeout are stale.
        assert_eq!(state.attempt(), 2);
        assert!(actions.contains(&PipelineAction::AssignWork {
            stage_id: "work".into(),
            attempt: 2,
            role: "dev".into(),
        }));
        let old_holder = PipelineEvent::WorkCompleted {
            stage_id: "work".into(),
            attempt: 1,
            product: WorkProduct::Branch {
                branch: "b".into(),
                head: "H1".into(),
                patch_id: "P1".into(),
            },
        };
        assert_eq!(step(&state, old_holder), Err(TransitionError::StaleResult));
        let replayed_timeout = PipelineEvent::StageTimedOut {
            attempt: 1,
            stage_id: "work".into(),
        };
        assert_eq!(
            step(&state, replayed_timeout),
            Err(TransitionError::StaleResult)
        );
        let (_, actions) = apply(
            &apply(&initial(), PipelineEvent::Start).0,
            PipelineEvent::StageTimedOut {
                attempt: 1,
                stage_id: "work".into(),
            },
        );
        assert_eq!(
            actions,
            [PipelineAction::NotifyTimeout {
                stage_id: "work".into()
            }]
        );
    }

    /// Round-1 minor: MainAdvanced (changed patch) and a failed stage return
    /// to the same work stage — the most recent one before the current stage.
    #[test]
    fn review_rework_target_is_the_most_recent_work_stage_for_every_path() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[0].stage = Stage::Work {
            role: "planner".into(),
            instructions: String::new(),
            output: WorkOutput::Result,
        };
        workflow.stages[0].id = "plan".into();
        workflow.stages.insert(
            1,
            WorkflowStage::new(
                "work",
                Stage::Work {
                    role: "dev".into(),
                    instructions: String::new(),
                    output: WorkOutput::Branch,
                },
            ),
        );
        let mut state = apply(&state_for(workflow), PipelineEvent::Start).0;
        state = apply(
            &state,
            PipelineEvent::WorkCompleted {
                stage_id: state
                    .current_stage()
                    .map_or_else(String::new, |stage| stage.id.clone()),
                attempt: state.attempt(),
                product: WorkProduct::Result {
                    summary: "plan".into(),
                    output: None,
                },
            },
        )
        .0;
        state = apply(&state, work(&state, "H1", "P1")).0;
        state = apply(&state, submitted(&state)).0;
        state = apply(&state, command(&state, Some(0))).0;
        state = apply(&state, approve(&state, "reviewer-1")).0;
        assert!(matches!(state.current_stage().unwrap().stage, Stage::Merge));
        let (rebased, _) = through_merge(&state, main_advanced("H2", "P2", false));
        assert_eq!(stage_id(&rebased), "work");
        let (conflicted, _) = through_merge(&state, main_advanced("H2", "P1", true));
        assert_eq!(stage_id(&conflicted), "work");
        assert_eq!(conflicted.current_head.as_deref(), Some("H1"));
        let mut checks = state.clone();
        checks.stage_index = 3;
        let (failed, _) = apply(&checks, command(&checks, Some(1)));
        assert_eq!(stage_id(&failed), "work");
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
        let mut state = apply(&state_for(workflow), PipelineEvent::Start).0;
        state = apply(
            &state,
            PipelineEvent::WorkCompleted {
                stage_id: state
                    .current_stage()
                    .map_or_else(String::new, |stage| stage.id.clone()),
                attempt: state.attempt(),
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
        let (state, actions) = apply(
            &state,
            PipelineEvent::FanoutCompleted {
                attempt: state.attempt(),
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
        assert_eq!(choices, &["child-1", "child-2"]);
        let pick = |selected_child: Option<&str>| PipelineEvent::ApprovalGranted {
            attempt: state.attempt(),
            stage_id: "review".into(),
            reviewer: "operator".into(),
            head: None,
            selected_child: selected_child.map(Into::into),
        };
        assert_eq!(
            step(&state, pick(None)),
            Err(TransitionError::FanoutSelectionRequired)
        );
        let (state, actions) = apply(&state, pick(Some("child-2")));
        assert_eq!(state.status, PipelineStatus::Done);
        assert!(actions.contains(&PipelineAction::CancelFanoutSiblings {
            winner_task_id: "child-2".into(),
            sibling_task_ids: vec!["child-1".into()],
        }));

        let research = state_for(Workflow::builtin_research());
        let state = apply(&research, PipelineEvent::Start).0;
        let state = apply(
            &state,
            PipelineEvent::WorkCompleted {
                stage_id: state
                    .current_stage()
                    .map_or_else(String::new, |stage| stage.id.clone()),
                attempt: state.attempt(),
                product: WorkProduct::Result {
                    summary: "done".into(),
                    output: None,
                },
            },
        )
        .0;
        assert_eq!(stage_id(&state), "review");
        let (state, actions) = apply(&state, request_changes(&state));
        assert_eq!(stage_id(&state), "work");
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, PipelineAction::ReturnToWork { .. }))
        );
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
        let mut state = apply(&state_for(workflow), PipelineEvent::Start).0;
        state = apply(
            &state,
            PipelineEvent::WorkCompleted {
                stage_id: state
                    .current_stage()
                    .map_or_else(String::new, |stage| stage.id.clone()),
                attempt: state.attempt(),
                product: WorkProduct::Plan {
                    items: vec!["one".into(), "two".into()],
                },
            },
        )
        .0;
        let (_, actions) = apply(
            &state,
            PipelineEvent::FanoutCompleted {
                attempt: state.attempt(),
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

    /// Round-1 review I4: fanout `all` joins on the child set.
    #[test]
    fn review_i4_fanout_all_waits_for_child_set_without_requesting_a_winner() {
        let mut state = apply(&state_for(Workflow::builtin_epic()), PipelineEvent::Start).0;
        state = apply(
            &state,
            PipelineEvent::WorkCompleted {
                stage_id: state
                    .current_stage()
                    .map_or_else(String::new, |stage| stage.id.clone()),
                attempt: state.attempt(),
                product: WorkProduct::Plan {
                    items: vec!["one".into(), "two".into()],
                },
            },
        )
        .0;
        let (state, actions) = apply(
            &state,
            PipelineEvent::FanoutCompleted {
                attempt: state.attempt(),
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
    fn non_command_failure_uses_on_fail_target_or_fails_the_task() {
        let mut workflow = Workflow::builtin_code();
        workflow.stages[1].on_fail = Some("work".into());
        let state = at_submit(workflow);
        let (state, _) = apply(
            &state,
            PipelineEvent::StageFailed {
                attempt: state.attempt(),
                stage_id: "submit".into(),
                reason: "forge is unavailable".into(),
            },
        );
        assert_eq!(stage_id(&state), "work");

        let state = apply(&initial(), PipelineEvent::Start).0;
        let (state, actions) = apply(
            &state,
            PipelineEvent::StageFailed {
                attempt: state.attempt(),
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

    /// Round-1 review I5: saving a merge workflow without a command fails, so
    /// the runtime gate can always be satisfied by a saved workflow.
    #[test]
    fn review_i5_merge_workflow_requires_a_command_check() {
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

    /// Round-1 review minor: a corrupt stage index is an error, not a panic.
    #[test]
    fn review_invalid_stage_index_does_not_panic() {
        for index in [5, 99, usize::MAX] {
            let mut state = initial();
            state.status = PipelineStatus::Running;
            state.stage_index = index;
            assert_eq!(
                step(&state, submitted(&state)),
                Err(TransitionError::InvalidStageIndex)
            );
            assert!(step(&state, commit("H9", "P9")).is_err());
            assert!(step(&state, main_advanced("H9", "P9", false)).is_err());
            assert!(step(&state, merged(&state, "H9")).is_err());
        }
    }

    /// Tampered states — any stage index, status, head and records, which
    /// only code inside this crate can build — never make `step` panic, and
    /// `MergeCompleted` is only accepted when the records cover every command
    /// and approval stage for the current head.
    #[test]
    fn tampered_states_never_panic_and_never_merge_past_the_records() {
        struct Rng(u64);
        impl Rng {
            fn below(&mut self, n: usize) -> usize {
                self.0 ^= self.0 >> 12;
                self.0 ^= self.0 << 25;
                self.0 ^= self.0 >> 27;
                (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) % n as u64) as usize
            }
            fn chance(&mut self, percent: usize) -> bool {
                self.below(100) < percent
            }
            fn head(&mut self) -> String {
                alloc::format!("H{}", self.below(2))
            }
        }
        let mut rng = Rng(0x5eed_7a3e_0000_0001);
        let workflows = [
            Workflow::builtin_code(),
            Workflow::builtin_planned(),
            Workflow::builtin_research(),
            Workflow::builtin_epic(),
            mixed_workflow(),
        ];
        let mut attempts = 0;
        for workflow in workflows {
            let len = workflow.stages.len();
            let merge = workflow
                .stages
                .iter()
                .position(|stage| stage.stage.kind() == StageKind::Merge);
            for _ in 0..3_000 {
                let mut state = state_for(workflow.clone());
                state.stage_index = match (rng.below(10), merge) {
                    (0..4, Some(merge)) => merge,
                    (4, _) => usize::MAX,
                    (5, _) => len + rng.below(3),
                    _ => rng.below(len),
                };
                state.status = [
                    PipelineStatus::Pending,
                    PipelineStatus::Running,
                    PipelineStatus::Running,
                    PipelineStatus::Running,
                    PipelineStatus::Done,
                    PipelineStatus::Failed,
                    PipelineStatus::Cancelled,
                ][rng.below(7)];
                state.current_head = rng.chance(80).then(|| rng.head());
                state.patch_id = rng.chance(80).then(|| alloc::format!("P{}", rng.below(2)));
                for stage in &workflow.stages {
                    match stage.stage {
                        Stage::Command { .. } if rng.chance(60) => {
                            state.passed_checks.push(PassedCheck {
                                stage_id: stage.id.clone(),
                                head: rng.chance(90).then(|| rng.head()),
                            })
                        }
                        Stage::Approval { .. } if rng.chance(60) => {
                            state.approvals.push(ApprovalRecord {
                                stage_id: stage.id.clone(),
                                head: rng.chance(80).then(|| rng.head()),
                                patch_id: None,
                                reviewers: vec!["r1".into()],
                            })
                        }
                        _ => {}
                    }
                }
                for attempt in &mut state.attempts {
                    *attempt = rng.below(3) as u32;
                }
                let stage = if rng.chance(50) {
                    state
                        .current_stage()
                        .map_or_else(|| "none".into(), |stage| stage.id.clone())
                } else {
                    workflow.stages[rng.below(len)].id.clone()
                };
                let head = state.current_head.clone();
                // Mostly the current attempt, sometimes a stale one.
                let attempt = state.attempt() + u32::from(rng.chance(20));
                let events = [
                    PipelineEvent::Start,
                    PipelineEvent::WorkCompleted {
                        stage_id: stage.clone(),
                        attempt,
                        product: WorkProduct::Branch {
                            branch: "b".into(),
                            head: "H5".into(),
                            patch_id: "P5".into(),
                        },
                    },
                    PipelineEvent::Submitted {
                        stage_id: stage.clone(),
                        attempt,
                        change_id: None,
                    },
                    PipelineEvent::CommandFinished {
                        attempt,
                        stage_id: stage.clone(),
                        head: head.clone(),
                        exit_code: Some(rng.below(2) as i32),
                    },
                    PipelineEvent::ApprovalGranted {
                        attempt,
                        stage_id: stage.clone(),
                        reviewer: "r2".into(),
                        head: head.clone(),
                        selected_child: rng.chance(30).then(|| "c1".into()),
                    },
                    PipelineEvent::ChangesRequested {
                        attempt,
                        stage_id: stage.clone(),
                        reviewer: "r2".into(),
                        head: head.clone(),
                        reason: "x".into(),
                    },
                    commit("H6", "P6"),
                    main_advanced("H7", "P0", rng.chance(20)),
                    PipelineEvent::FanoutCompleted {
                        attempt,
                        stage_id: stage.clone(),
                        child_task_ids: vec!["c1".into(), "c2".into()],
                        selected_child: None,
                    },
                    PipelineEvent::StageFailed {
                        attempt,
                        stage_id: stage.clone(),
                        reason: "x".into(),
                    },
                    PipelineEvent::StageTimedOut {
                        stage_id: stage.clone(),
                        attempt,
                    },
                    PipelineEvent::Cancel { reason: "x".into() },
                    PipelineEvent::MergeCompleted {
                        stage_id: stage.clone(),
                        attempt,
                        head: head.clone().unwrap_or_default(),
                        merge_commit: "M".into(),
                    },
                    PipelineEvent::MergeFailed {
                        stage_id: stage.clone(),
                        attempt,
                        head: head.clone().unwrap_or_default(),
                        reason: "x".into(),
                    },
                ];
                for event in events {
                    attempts += 1;
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        step(&state, event.clone())
                    }))
                    .unwrap_or_else(|_| panic!("step panicked on {event:?} in {state:#?}"));
                    if let (PipelineEvent::MergeCompleted { head, .. }, Ok(_)) = (&event, &result) {
                        assert!(
                            records_cover_merge(&state, head),
                            "merge accepted past the records: {state:#?}"
                        );
                    }
                }
            }
        }
        assert_eq!(attempts, 5 * 3_000 * 14);
    }

    fn records_cover_merge(state: &PipelineState, head: &str) -> bool {
        let Some(stages) = state.workflow.stages.get(..state.stage_index) else {
            return false;
        };
        state.current_head.as_deref() == Some(head)
            && stages.iter().all(|stage| match stage.stage {
                Stage::Command { .. } => state
                    .passed_checks
                    .iter()
                    .any(|check| check.stage_id == stage.id && check.head.as_deref() == Some(head)),
                Stage::Approval { bind_head, .. } => state.approvals.iter().any(|approval| {
                    approval.stage_id == stage.id
                        && (!bind_head || approval.head.as_deref() == Some(head))
                }),
                _ => true,
            })
    }
}
