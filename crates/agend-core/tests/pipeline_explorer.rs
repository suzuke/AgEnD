//! Seeded pseudo-random event-sequence explorer for `pipeline::state::step`.
//!
//! A property test without a property-testing dependency (core's dependency
//! and dev-dependency allowlist stays as it is): a fixed-seed xorshift
//! generator drives thousands of event sequences through the built-in
//! workflows and several custom ones. Most events are plausible for the
//! current stage (so tasks really reach merge), the rest are head changes,
//! stale or forged results, failures, timeouts and cancellations.
//!
//! After every step an independent oracle — built only from the events that
//! `step` accepted and the `ReturnToWork` actions it emitted, never from the
//! state's own records — checks (rework forgets every check and approval at
//! or after the work stage it returns to; only D14 carries approvals):
//!
//! 1. merge gate: `Merge` and `MergeCompleted` (and `Done` of a workflow
//!    without merge) only when every command stage before it passed for the
//!    current head and every approval stage has enough approvals covering the
//!    current head (bound) or at all (not bound); D14 lets a clean same-patch
//!    rebase carry an approval to the rebased head;
//! 2. no skipped stage: the stage index moves forward only on the current
//!    stage's own completion event, and only over approvals already
//!    satisfied; `Submitted` is required before anything after a submit stage;
//! 3. rework is never lost: a head change in a work stage leaves the task
//!    there with no action; requested changes and failed checks send the task
//!    back (to the most recent work stage unless `on_fail` says otherwise)
//!    instead of failing it; head changes never move a task forward;
//! 4. approvals and results only count for the head (and stage) they were
//!    given for;
//! 5. terminal states accept no event, and an in-flight merge cannot be
//!    cancelled;
//! 6. `step` never panics (tampered states are tested inside the crate,
//!    `pipeline::state::tests`, the only place a state can be forged).
//!
//! On failure the message names the workflow, seed and full event trace.
//! `AGEND_EXPLORER_SEQUENCES` overrides the per-workflow sequence count for
//! a deeper local run.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use agend_core::pipeline::stage::{FanoutJoin, StageKind};
use agend_core::pipeline::state::{
    PipelineAction, PipelineEvent, PipelineState, PipelineStatus, WorkProduct, step,
};
use agend_core::pipeline::workflow::{
    Approver, FanoutSource, Stage, TimeoutAction, WorkOutput, Workflow, WorkflowStage,
};

const SEED: u64 = 0x5eed_a9e1_d000_0001;
const SEQUENCES_PER_WORKFLOW: usize = 4_000;
const STEPS_PER_SEQUENCE: usize = 60;
const REVIEWERS: [&str; 3] = ["r1", "r2", "r3"];

/// xorshift64* — deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn roles() -> Vec<String> {
    ["dev", "reviewer", "researcher", "planner"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn work_stage(id: &str, output: WorkOutput) -> WorkflowStage {
    WorkflowStage::new(
        id,
        Stage::Work {
            role: "dev".into(),
            instructions: String::new(),
            output,
        },
    )
}

fn command_stage(id: &str, command: &str) -> WorkflowStage {
    WorkflowStage::new(
        id,
        Stage::Command {
            command: command.into(),
        },
    )
}

fn approval(id: &str, by: Approver, count: u8, bind_head: bool) -> WorkflowStage {
    WorkflowStage::new(
        id,
        Stage::Approval {
            by,
            count,
            bind_head,
        },
    )
}

fn submit_stage() -> WorkflowStage {
    WorkflowStage::new(
        "submit",
        Stage::Submit {
            forge: "local".into(),
        },
    )
}

fn repo_workflow(id: &str, stages: Vec<WorkflowStage>) -> Workflow {
    let mut workflow = Workflow::builtin_code();
    workflow.id = id.into();
    workflow.stages = stages;
    workflow
}

fn epic_with(join: FanoutJoin) -> Workflow {
    let mut workflow = Workflow::builtin_epic();
    workflow.stages[1].stage = Stage::Fanout {
        source: FanoutSource::WorkOutput,
        join,
    };
    workflow
}

fn reviewer() -> Approver {
    Approver::Role("reviewer".into())
}

/// Built-in workflows plus custom shapes that exercise multiple approvals,
/// multiple checks, checks before submit, unbound approvals before merge,
/// `on_fail`, placeholders and `allow_unreviewed`.
fn workflows() -> Vec<(&'static str, Workflow)> {
    let mut unreviewed = repo_workflow(
        "unreviewed",
        vec![
            work_stage("work", WorkOutput::Branch),
            submit_stage(),
            command_stage("checks", "cargo test"),
            WorkflowStage::new("merge", Stage::Merge),
        ],
    );
    unreviewed.allow_unreviewed = true;
    let mut design = approval("design", Approver::Human, 1, false);
    design.on_fail = Some("work".into());
    let mut reviewed_twice = approval("review", reviewer(), 2, true);
    reviewed_twice.on_timeout = Some(TimeoutAction::Cancel);
    let workflows = vec![
        ("code", Workflow::builtin_code()),
        ("research", Workflow::builtin_research()),
        ("planned", Workflow::builtin_planned()),
        ("epic", Workflow::builtin_epic()),
        ("epic-first", epic_with(FanoutJoin::First)),
        ("epic-pick", epic_with(FanoutJoin::Pick)),
        (
            "human-gate",
            repo_workflow(
                "human-gate",
                vec![
                    work_stage("work", WorkOutput::Branch),
                    submit_stage(),
                    command_stage("checks", "cargo test"),
                    approval("review", reviewer(), 1, true),
                    approval("human", Approver::Human, 1, true),
                    WorkflowStage::new("merge", Stage::Merge),
                ],
            ),
        ),
        (
            "review-between-checks",
            repo_workflow(
                "review-between-checks",
                vec![
                    work_stage("work", WorkOutput::Branch),
                    submit_stage(),
                    approval("review", reviewer(), 1, true),
                    command_stage("checks", "cargo test"),
                    command_stage("lint", "cargo clippy"),
                    approval("human", Approver::Human, 1, true),
                    WorkflowStage::new("merge", Stage::Merge),
                ],
            ),
        ),
        ("unreviewed", unreviewed),
        (
            "checks-before-submit",
            repo_workflow(
                "checks-before-submit",
                vec![
                    work_stage("work", WorkOutput::Branch),
                    command_stage("checks", "cargo test"),
                    submit_stage(),
                    reviewed_twice,
                    design,
                    WorkflowStage::new("merge", Stage::Merge),
                ],
            ),
        ),
        (
            "pr-placeholders",
            repo_workflow(
                "pr-placeholders",
                vec![
                    work_stage("work", WorkOutput::Branch),
                    submit_stage(),
                    command_stage("checks", "gh pr checks {pr} --head {head}"),
                    approval("review", reviewer(), 1, true),
                    WorkflowStage::new("merge", Stage::Merge),
                ],
            ),
        ),
    ];
    for (name, workflow) in &workflows {
        assert_eq!(workflow.validate(&roles()), Ok(()), "workflow {name}");
    }
    workflows
}

/// Event source: plausible completions, head changes and adversarial events.
struct Generator {
    counter: u64,
    heads: Vec<String>,
}

impl Generator {
    fn new() -> Self {
        Self {
            counter: 0,
            heads: Vec::new(),
        }
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}{}", self.counter)
    }

    fn fresh_head(&mut self) -> String {
        let head = self.fresh("H");
        self.heads.push(head.clone());
        head
    }

    fn any_head(&mut self, rng: &mut Rng) -> Option<String> {
        if self.heads.is_empty() || rng.chance(15) {
            None
        } else {
            Some(rng.pick(&self.heads).clone())
        }
    }

    fn any_stage_id(&self, rng: &mut Rng, state: &PipelineState) -> String {
        if rng.chance(10) {
            return "no-such-stage".into();
        }
        rng.pick(&state.workflow().stages).id.clone()
    }

    fn event(&mut self, rng: &mut Rng, state: &PipelineState, oracle: &Oracle) -> PipelineEvent {
        let roll = rng.below(100);
        if roll < 62
            && let Some(event) = self.plausible(rng, state)
        {
            return event;
        }
        if roll < 78 {
            return self.head_change(rng, oracle);
        }
        if roll < 98 {
            return self.adversarial(rng, state);
        }
        let stage_id = state
            .current_stage()
            .map_or_else(|| "none".into(), |stage| stage.id.clone());
        match rng.below(4) {
            0 => PipelineEvent::Cancel {
                reason: "operator".into(),
            },
            1 => PipelineEvent::StageFailed {
                stage_id,
                reason: "infrastructure".into(),
            },
            _ => PipelineEvent::StageTimedOut { stage_id },
        }
    }

    fn plausible(&mut self, rng: &mut Rng, state: &PipelineState) -> Option<PipelineEvent> {
        if state.status() == PipelineStatus::Pending {
            return Some(PipelineEvent::Start);
        }
        let stage = state.current_stage()?;
        let stage_id = stage.id.clone();
        let head = state.current_head().map(String::from);
        Some(match &stage.stage {
            Stage::Work { output, .. } => PipelineEvent::WorkCompleted {
                product: match output {
                    WorkOutput::Branch => {
                        let (head, patch_id) = match (state.current_head(), state.patch_id()) {
                            (Some(head), Some(patch)) if rng.chance(25) => {
                                (head.to_string(), patch.to_string())
                            }
                            _ => (self.fresh_head(), self.fresh("P")),
                        };
                        WorkProduct::Branch {
                            branch: "agend/T-1/explore".into(),
                            head,
                            patch_id,
                        }
                    }
                    WorkOutput::Result => WorkProduct::Result {
                        summary: "findings".into(),
                        output: None,
                    },
                    WorkOutput::Plan => WorkProduct::Plan {
                        items: vec!["a".into(), "b".into()],
                    },
                },
            },
            Stage::Submit { .. } => PipelineEvent::Submitted {
                change_id: rng.chance(80).then(|| "42".into()),
            },
            Stage::Command { .. } => PipelineEvent::CommandFinished {
                stage_id,
                head,
                exit_code: match rng.below(10) {
                    0 => Some(1),
                    1 => None,
                    _ => Some(0),
                },
            },
            Stage::Approval { .. } => {
                let reviewer = rng.pick(&REVIEWERS).to_string();
                if rng.chance(12) {
                    PipelineEvent::ChangesRequested {
                        stage_id,
                        reviewer,
                        head,
                        reason: "please change".into(),
                    }
                } else {
                    let selected_child = (!state.fanout_child_task_ids().is_empty()
                        && rng.chance(85))
                    .then(|| rng.pick(state.fanout_child_task_ids()).clone());
                    PipelineEvent::ApprovalGranted {
                        stage_id,
                        reviewer,
                        head,
                        selected_child,
                    }
                }
            }
            Stage::Fanout { join, .. } => PipelineEvent::FanoutCompleted {
                stage_id,
                child_task_ids: vec!["c1".into(), "c2".into(), "c3".into()],
                selected_child: (*join == FanoutJoin::First).then(|| "c2".into()),
            },
            Stage::Merge if rng.chance(25) => PipelineEvent::MergeFailed {
                head: head.unwrap_or_default(),
                reason: "forge refused".into(),
            },
            Stage::Merge => PipelineEvent::MergeCompleted {
                head: head.unwrap_or_default(),
                merge_commit: self.fresh("M"),
            },
        })
    }

    fn head_change(&mut self, rng: &mut Rng, oracle: &Oracle) -> PipelineEvent {
        if rng.chance(40) {
            return PipelineEvent::CommitCreated {
                head: self.fresh_head(),
                patch_id: self.fresh("P"),
            };
        }
        let same_patch = oracle
            .head
            .as_ref()
            .and_then(|head| oracle.patch_of.get(head))
            .filter(|_| rng.chance(65))
            .cloned();
        PipelineEvent::MainAdvanced {
            rebased_head: self.fresh_head(),
            patch_id: same_patch.unwrap_or_else(|| self.fresh("P")),
            conflict: rng.chance(15),
        }
    }

    fn adversarial(&mut self, rng: &mut Rng, state: &PipelineState) -> PipelineEvent {
        let stage_id = self.any_stage_id(rng, state);
        let reviewer = rng.pick(&REVIEWERS).to_string();
        match rng.below(13) {
            12 => PipelineEvent::MergeFailed {
                head: self.any_head(rng).unwrap_or_default(),
                reason: "stale".into(),
            },
            0 => PipelineEvent::Start,
            1 => PipelineEvent::Submitted { change_id: None },
            2 => PipelineEvent::CommandFinished {
                stage_id,
                head: self.any_head(rng),
                exit_code: Some(0),
            },
            3 => PipelineEvent::ApprovalGranted {
                stage_id,
                reviewer,
                head: self.any_head(rng),
                selected_child: None,
            },
            4 => PipelineEvent::ApprovalGranted {
                stage_id: state
                    .current_stage()
                    .map_or_else(String::new, |stage| stage.id.clone()),
                reviewer,
                head: state.current_head().map(String::from),
                selected_child: Some(if rng.chance(50) { "c9" } else { "c1" }.into()),
            },
            5 => PipelineEvent::ChangesRequested {
                stage_id,
                reviewer,
                head: self.any_head(rng),
                reason: "stale".into(),
            },
            6 => PipelineEvent::MergeCompleted {
                head: self.any_head(rng).unwrap_or_default(),
                merge_commit: self.fresh("M"),
            },
            7 => PipelineEvent::WorkCompleted {
                product: match rng.below(3) {
                    0 => WorkProduct::Result {
                        summary: "wrong kind".into(),
                        output: None,
                    },
                    1 => WorkProduct::Plan { items: Vec::new() },
                    _ => WorkProduct::Branch {
                        branch: "agend/T-1/explore".into(),
                        head: self.any_head(rng).unwrap_or_default(),
                        patch_id: "P-old".into(),
                    },
                },
            },
            8 => PipelineEvent::FanoutCompleted {
                stage_id,
                child_task_ids: match rng.below(3) {
                    0 => Vec::new(),
                    1 => vec!["c1".into(), "c1".into()],
                    _ => vec!["c1".into(), String::new()],
                },
                selected_child: None,
            },
            9 => PipelineEvent::StageFailed {
                stage_id,
                reason: "stale failure".into(),
            },
            10 => PipelineEvent::StageTimedOut { stage_id },
            _ => PipelineEvent::MainAdvanced {
                rebased_head: state.current_head().map(String::from).unwrap_or_default(),
                patch_id: state.patch_id().map(String::from).unwrap_or_default(),
                conflict: false,
            },
        }
    }
}

/// What the accepted events say, independent of the state's own records.
struct Oracle {
    head: Option<String>,
    patch_of: BTreeMap<String, String>,
    passed: BTreeSet<(String, Option<String>)>,
    /// (stage, reviewer, heads covered; `None` for an unbound approval).
    approvals: Vec<(String, String, Option<BTreeSet<String>>)>,
    submitted_since_work: bool,
    /// Head changes seen while the merge was in flight; they only count if
    /// the forge reports the merge failed.
    pending: Vec<PipelineEvent>,
}

impl Oracle {
    fn new() -> Self {
        Self {
            head: None,
            patch_of: BTreeMap::new(),
            passed: BTreeSet::new(),
            approvals: Vec::new(),
            submitted_since_work: false,
            pending: Vec::new(),
        }
    }

    fn set_head(&mut self, head: &str, patch_id: &str) {
        self.head = Some(head.into());
        self.patch_of.insert(head.into(), patch_id.into());
    }

    fn record(&mut self, before: &PipelineState, event: &PipelineEvent) {
        if is_head_change(event) && before.merge_in_flight() {
            self.pending.push(event.clone());
            return;
        }
        self.apply_event(before, event);
    }

    fn apply_event(&mut self, before: &PipelineState, event: &PipelineEvent) {
        match event {
            PipelineEvent::MergeFailed { .. } => {
                for pending in std::mem::take(&mut self.pending) {
                    self.apply_event(before, &pending);
                }
            }
            PipelineEvent::MergeCompleted { .. } => self.pending.clear(),
            PipelineEvent::WorkCompleted { product } => {
                if let WorkProduct::Branch { head, patch_id, .. } = product {
                    self.set_head(head, patch_id);
                }
                self.submitted_since_work = false;
            }
            PipelineEvent::Submitted { .. } => self.submitted_since_work = true,
            PipelineEvent::CommandFinished {
                stage_id,
                head,
                exit_code: Some(0),
            } => {
                self.passed.insert((stage_id.clone(), head.clone()));
            }
            PipelineEvent::ApprovalGranted {
                stage_id,
                reviewer,
                head,
                ..
            } => {
                let covered =
                    bound(before, stage_id).then(|| head.iter().cloned().collect::<BTreeSet<_>>());
                self.approvals
                    .push((stage_id.clone(), reviewer.clone(), covered));
            }
            PipelineEvent::CommitCreated { head, patch_id } => self.set_head(head, patch_id),
            PipelineEvent::MainAdvanced {
                rebased_head,
                patch_id,
                conflict: false,
            } => {
                let previous = self.head.clone();
                let same_patch = previous
                    .as_ref()
                    .and_then(|head| self.patch_of.get(head))
                    .is_some_and(|previous_patch| previous_patch == patch_id);
                if let (true, Some(previous)) = (same_patch, previous) {
                    for (_, _, covered) in &mut self.approvals {
                        if let Some(covered) = covered
                            && covered.contains(&previous)
                        {
                            covered.insert(rebased_head.clone());
                        }
                    }
                }
                self.set_head(rebased_head, patch_id);
            }
            _ => {}
        }
    }

    /// Rework invalidates every check and approval at or after the work stage
    /// it returns to (only D14 carries approvals across a head change).
    fn forget_from(&mut self, workflow: &Workflow, work_stage_id: &str) {
        let position = |id: &str| workflow.stages.iter().position(|stage| stage.id == id);
        let Some(target) = position(work_stage_id) else {
            return;
        };
        self.passed
            .retain(|(stage_id, _)| position(stage_id).is_some_and(|index| index < target));
        self.approvals
            .retain(|(stage_id, _, _)| position(stage_id).is_some_and(|index| index < target));
    }

    /// Why the gate over `stages[..upto]` is shut for `head`, if it is.
    fn gate_blocker(&self, workflow: &Workflow, upto: usize, head: Option<&str>) -> Option<String> {
        for stage in &workflow.stages[..upto] {
            match &stage.stage {
                Stage::Command { .. } => {
                    if !self
                        .passed
                        .contains(&(stage.id.clone(), head.map(String::from)))
                    {
                        return Some(format!("command {} never passed on {head:?}", stage.id));
                    }
                }
                Stage::Approval { .. } if !self.approval_covered(stage, head) => {
                    return Some(format!("approval {} does not cover {head:?}", stage.id));
                }
                _ => {}
            }
        }
        None
    }

    fn approval_covered(&self, stage: &WorkflowStage, head: Option<&str>) -> bool {
        let Stage::Approval {
            count, bind_head, ..
        } = &stage.stage
        else {
            return true;
        };
        let reviewers: BTreeSet<&str> = self
            .approvals
            .iter()
            .filter(|(stage_id, _, covered)| {
                *stage_id == stage.id
                    && (!*bind_head
                        || head.is_some_and(|head| {
                            covered
                                .as_ref()
                                .is_some_and(|covered| covered.contains(head))
                        }))
            })
            .map(|(_, reviewer, _)| reviewer.as_str())
            .collect();
        reviewers.len() >= usize::from(*count)
    }
}

fn bound(state: &PipelineState, stage_id: &str) -> bool {
    state.workflow().stages.iter().any(|stage| {
        stage.id == stage_id
            && matches!(
                stage.stage,
                Stage::Approval {
                    bind_head: true,
                    ..
                }
            )
    })
}

fn kind_at(workflow: &Workflow, index: usize) -> Option<StageKind> {
    workflow.stages.get(index).map(|stage| stage.stage.kind())
}

fn merge_index(workflow: &Workflow) -> Option<usize> {
    workflow
        .stages
        .iter()
        .position(|stage| stage.stage.kind() == StageKind::Merge)
}

fn completes(kind: StageKind, event: &PipelineEvent) -> bool {
    matches!(
        (kind, event),
        (StageKind::Work, PipelineEvent::WorkCompleted { .. })
            | (StageKind::Submit, PipelineEvent::Submitted { .. })
            | (
                StageKind::Command,
                PipelineEvent::CommandFinished {
                    exit_code: Some(0),
                    ..
                }
            )
            | (StageKind::Approval, PipelineEvent::ApprovalGranted { .. })
            | (StageKind::Fanout, PipelineEvent::FanoutCompleted { .. })
    )
}

fn is_head_change(event: &PipelineEvent) -> bool {
    matches!(
        event,
        PipelineEvent::CommitCreated { .. } | PipelineEvent::MainAdvanced { .. }
    )
}

/// Check every invariant for one accepted step. `oracle` already includes
/// `event`.
fn check_step(
    before: &PipelineState,
    event: &PipelineEvent,
    after: &PipelineState,
    actions: &[PipelineAction],
    oracle: &Oracle,
) -> Result<(), String> {
    let workflow = &after.workflow();
    let (from, to) = (before.stage_index(), after.stage_index());
    let from_kind = kind_at(workflow, from);

    // 5. Terminal states are terminal.
    if before.status().is_terminal() {
        return Err(format!("{:?} pipeline accepted an event", before.status()));
    }
    // Head tracking matches the accepted events.
    if after.current_head() != oracle.head.as_deref() {
        return Err(format!(
            "state head {:?} differs from observed head {:?}",
            after.current_head(),
            oracle.head
        ));
    }

    // 4. Results and reviews only for the current stage and head.
    match event {
        PipelineEvent::CommandFinished { stage_id, head, .. }
            if before.current_stage().map(|s| &s.id) != Some(stage_id)
                || head.as_deref() != before.current_head() =>
        {
            return Err("command result for another stage or head was accepted".into());
        }
        PipelineEvent::ApprovalGranted { stage_id, head, .. }
        | PipelineEvent::ChangesRequested { stage_id, head, .. } => {
            if before.current_stage().map(|s| &s.id) != Some(stage_id) {
                return Err("review for another stage was accepted".into());
            }
            if bound(before, stage_id)
                && (head.as_deref() != before.current_head() || before.current_head().is_none())
            {
                return Err("head-bound review for another head was accepted".into());
            }
        }
        PipelineEvent::MergeCompleted { head, .. }
            if before.current_head() != Some(head.as_str()) =>
        {
            return Err("merge of a head other than the current one was accepted".into());
        }
        _ => {}
    }

    // An in-flight merge cannot be cancelled.
    if matches!(event, PipelineEvent::Cancel { .. }) && from_kind == Some(StageKind::Merge) {
        return Err("cancel accepted while the merge was in flight".into());
    }

    // 1. Merge gate, from the oracle.
    for action in actions {
        if let PipelineAction::Merge { head, .. } = action {
            let merge = merge_index(workflow).ok_or("merge action without merge stage")?;
            if Some(head.as_str()) != after.current_head() {
                return Err("merge action for a head other than the current one".into());
            }
            if let Some(blocker) = oracle.gate_blocker(workflow, merge, Some(head)) {
                return Err(format!("merge action while gate shut: {blocker}"));
            }
        }
    }
    if after.status() == PipelineStatus::Done {
        let upto = match merge_index(workflow) {
            Some(merge) => {
                if !matches!(event, PipelineEvent::MergeCompleted { .. }) {
                    return Err("merge workflow finished without MergeCompleted".into());
                }
                merge
            }
            None => workflow.stages.len(),
        };
        if let Some(blocker) = oracle.gate_blocker(workflow, upto, after.current_head()) {
            return Err(format!("task done while gate shut: {blocker}"));
        }
        if merge_index(workflow).is_some() && !oracle.submitted_since_work {
            return Err("task merged without Submitted".into());
        }
    }

    // 3. Head changes never move a task forward; in work they change nothing.
    if is_head_change(event) {
        if to > from {
            return Err(format!("head change moved the task forward {from} -> {to}"));
        }
        if from_kind == Some(StageKind::Work)
            && (to != from || !actions.is_empty() || after.status() != before.status())
        {
            return Err(format!(
                "head change during work left the work stage or acted: {actions:?}"
            ));
        }
        if from_kind == Some(StageKind::Submit) && to == from && !actions.is_empty() {
            return Err(format!("head change in submit emitted {actions:?}"));
        }
        if before.merge_in_flight() && (to != from || !actions.is_empty()) {
            return Err(format!(
                "head change pulled the task out of an in-flight merge: {actions:?}"
            ));
        }
    }
    // Requested changes and failed checks are rework, not task failure (D18).
    let rework_event = matches!(event, PipelineEvent::ChangesRequested { .. })
        || matches!(
            event,
            PipelineEvent::CommandFinished { exit_code, .. } if *exit_code != Some(0)
        );
    let has_target = before
        .current_stage()
        .is_some_and(|stage| stage.on_fail.is_some())
        || workflow.stages[..from]
            .iter()
            .any(|stage| stage.stage.kind() == StageKind::Work);
    if rework_event && has_target && (after.status() != PipelineStatus::Running || to >= from) {
        return Err(format!(
            "{event:?} did not send the task back: status {:?}, stage {from} -> {to}",
            after.status()
        ));
    }
    for action in actions {
        if let PipelineAction::ReturnToWork { stage_id, .. } = action {
            if after.current_stage().map(|stage| &stage.id) != Some(stage_id)
                || kind_at(workflow, to) != Some(StageKind::Work)
            {
                return Err(format!("ReturnToWork to {stage_id} did not land on work"));
            }
            let on_fail = before
                .current_stage()
                .and_then(|stage| stage.on_fail.clone());
            let previous_work = workflow.stages[..from]
                .iter()
                .rposition(|stage| stage.stage.kind() == StageKind::Work);
            if on_fail.as_ref() != Some(stage_id) && previous_work != Some(to) {
                return Err(format!(
                    "rework went to {stage_id}, not the most recent work stage"
                ));
            }
        }
    }

    // 2. No skipped stage.
    if after.status() == PipelineStatus::Running || after.status() == PipelineStatus::Done {
        if matches!(event, PipelineEvent::Start) {
            if to != 0 {
                return Err("start did not enter the first stage".into());
            }
        } else if to > from {
            let kind = from_kind.ok_or("moved from an invalid stage")?;
            if !completes(kind, event) {
                return Err(format!("{event:?} moved past a {kind:?} stage"));
            }
            for skipped in from + 1..to.min(workflow.stages.len()) {
                let stage = &workflow.stages[skipped];
                if stage.stage.kind() != StageKind::Approval
                    || !oracle.approval_covered(stage, after.current_head())
                {
                    return Err(format!(
                        "skipped stage {} without it being satisfied",
                        stage.id
                    ));
                }
            }
        } else if to < from
            && !is_head_change(event)
            && !matches!(
                event,
                PipelineEvent::ChangesRequested { .. }
                    | PipelineEvent::StageFailed { .. }
                    | PipelineEvent::CommandFinished { .. }
                    | PipelineEvent::MergeFailed { .. }
            )
        {
            return Err(format!("{event:?} moved the task backwards"));
        }
        for (submit, stage) in workflow.stages.iter().enumerate() {
            let past_submit = stage.stage.kind() == StageKind::Submit
                && to > submit
                && !workflow.stages[submit + 1..to.min(workflow.stages.len())]
                    .iter()
                    .any(|stage| stage.stage.kind() == StageKind::Work);
            if past_submit && !oracle.submitted_since_work {
                return Err(format!("reached stage {to} past submit without Submitted"));
            }
        }
    }

    // Status changes are explained by their action.
    if after.status() != before.status() {
        let explained = match after.status() {
            PipelineStatus::Failed => actions
                .iter()
                .any(|action| matches!(action, PipelineAction::TaskFailed { .. })),
            PipelineStatus::Cancelled => {
                matches!(
                    event,
                    PipelineEvent::Cancel { .. } | PipelineEvent::StageTimedOut { .. }
                ) && actions
                    .iter()
                    .any(|action| matches!(action, PipelineAction::TaskCancelled { .. }))
            }
            PipelineStatus::Done => actions
                .iter()
                .any(|action| matches!(action, PipelineAction::TaskDone { .. })),
            PipelineStatus::Running => matches!(event, PipelineEvent::Start),
            PipelineStatus::Pending => false,
        };
        if !explained {
            return Err(format!(
                "status {:?} -> {:?} without a matching action",
                before.status(),
                after.status()
            ));
        }
    }
    Ok(())
}

#[derive(Default)]
struct Stats {
    steps: usize,
    accepted: usize,
    merged: usize,
    done: usize,
    reworks: usize,
    head_changes_in_work: usize,
    cancelled: usize,
    failed: usize,
}

fn sequences() -> usize {
    std::env::var("AGEND_EXPLORER_SEQUENCES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(SEQUENCES_PER_WORKFLOW)
}

fn run_sequence(
    name: &str,
    workflow: &Workflow,
    seed: u64,
    stats: &mut Stats,
) -> Result<(), String> {
    let mut rng = Rng(seed);
    let mut generator = Generator::new();
    let mut oracle = Oracle::new();
    let mut state = PipelineState::new("T-1", workflow.clone().validated(&roles()).unwrap());
    let mut trace: Vec<String> = Vec::new();
    for _ in 0..STEPS_PER_SEQUENCE {
        let event = generator.event(&mut rng, &state, &oracle);
        trace.push(format!("{event:?}"));
        stats.steps += 1;
        let result = catch_unwind(AssertUnwindSafe(|| step(&state, event.clone())))
            .map_err(|_| format!("{name} seed {seed:#x}: step panicked; trace {trace:#?}"))?;
        let Ok((next, actions)) = result else {
            continue;
        };
        stats.accepted += 1;
        oracle.record(&state, &event);
        for action in &actions {
            if let PipelineAction::ReturnToWork { stage_id, .. } = action {
                oracle.forget_from(workflow, stage_id);
            }
        }
        check_step(&state, &event, &next, &actions, &oracle)
            .map_err(|error| format!("{name} seed {seed:#x}: {error}; trace {trace:#?}"))?;
        if is_head_change(&event) && kind_at(workflow, state.stage_index()) == Some(StageKind::Work)
        {
            stats.head_changes_in_work += 1;
        }
        stats.reworks += actions
            .iter()
            .filter(|action| matches!(action, PipelineAction::ReturnToWork { .. }))
            .count();
        state = next;
        match state.status() {
            PipelineStatus::Done => {
                stats.done += 1;
                stats.merged += usize::from(state.merge_commit().is_some());
            }
            PipelineStatus::Cancelled => stats.cancelled += 1,
            PipelineStatus::Failed => stats.failed += 1,
            _ => {}
        }
        if state.status().is_terminal() {
            // A few more events against the terminal state: all must fail.
            for _ in 0..3 {
                let event = generator.event(&mut rng, &state, &oracle);
                if step(&state, event.clone()).is_ok() {
                    return Err(format!(
                        "{name} seed {seed:#x}: terminal {:?} accepted {event:?}; trace {trace:#?}",
                        state.status()
                    ));
                }
            }
            break;
        }
    }
    Ok(())
}

#[test]
fn random_event_sequences_keep_every_pipeline_invariant() {
    let sequences = sequences();
    let mut seeds = Rng(SEED);
    let mut total = 0;
    for (name, workflow) in workflows() {
        let mut stats = Stats::default();
        for _ in 0..sequences {
            let seed = seeds.next() | 1;
            if let Err(failure) = run_sequence(name, &workflow, seed, &mut stats) {
                panic!("{failure}");
            }
        }
        eprintln!(
            "explorer {name}: {sequences} sequences, {} steps ({} accepted), {} done ({} merged), {} reworks, {} head changes during work, {} cancelled, {} failed",
            stats.steps,
            stats.accepted,
            stats.done,
            stats.merged,
            stats.reworks,
            stats.head_changes_in_work,
            stats.cancelled,
            stats.failed
        );
        // The exploration must actually reach the interesting states.
        assert!(stats.done > 0, "{name}: no sequence finished");
        assert!(stats.reworks > 0, "{name}: no sequence reworked");
        if merge_index(&workflow).is_some() {
            assert!(stats.merged > 0, "{name}: no sequence merged");
            assert!(
                stats.head_changes_in_work > 0,
                "{name}: no head change during work"
            );
        }
        total += sequences;
    }
    eprintln!("explorer total: {total} sequences");
}
