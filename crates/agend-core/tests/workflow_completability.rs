//! Save-time completability (verifier rounds r1 and r2 on gate 1).
//!
//! `Workflow::validate` must never accept a workflow the state machine cannot
//! finish. Besides the static rules it runs a completability witness (`step`
//! over the canonical success script, one rework per command/approval stage,
//! one new commit per non-work stage). This file holds:
//!
//! - the verifier's r2 counterexamples as named regression tests (each is now
//!   rejected at save time or completes);
//! - the verifier's random workflow generator: every accepted workflow must
//!   finish under an independent success driver, also after random
//!   disturbances (failures, requested changes, new commits, main advancing,
//!   merge failures, timeouts). 200,000 workflows by default; run the
//!   ignored test for 1,000,000 more.

use agend_core::pipeline::stage::FanoutJoin;
use agend_core::pipeline::state::{
    PipelineAction, PipelineEvent, PipelineState, PipelineStatus, TransitionError, WorkProduct,
    step,
};
use agend_core::pipeline::workflow::{
    Approver, FanoutSource, Stage, TimeoutAction, WorkOutput, Workflow, WorkflowError,
    WorkflowRequirement, WorkflowStage,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

fn roles() -> Vec<String> {
    ["dev", "reviewer", "planner"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn repo_workflow(stages: Vec<WorkflowStage>) -> Workflow {
    Workflow {
        id: "p".into(),
        version: 1,
        requires: vec![WorkflowRequirement::Repo],
        allow_unreviewed: false,
        stages,
    }
}

fn work(id: &str, output: WorkOutput) -> WorkflowStage {
    WorkflowStage::new(
        id,
        Stage::Work {
            role: "dev".into(),
            instructions: String::new(),
            output,
        },
    )
}

fn command(id: &str) -> WorkflowStage {
    WorkflowStage::new(
        id,
        Stage::Command {
            command: "true".into(),
        },
    )
}

fn approval(id: &str, bind_head: bool) -> WorkflowStage {
    WorkflowStage::new(
        id,
        Stage::Approval {
            by: Approver::Human,
            count: 1,
            bind_head,
        },
    )
}

fn pick_fanout(id: &str) -> WorkflowStage {
    WorkflowStage::new(
        id,
        Stage::Fanout {
            source: FanoutSource::Listed(vec!["a".into(), "b".into()]),
            join: FanoutJoin::Pick,
        },
    )
}

fn merge() -> WorkflowStage {
    WorkflowStage::new("m", Stage::Merge)
}

fn branch(state: &PipelineState, head: &str, patch: &str) -> PipelineEvent {
    PipelineEvent::WorkCompleted {
        stage_id: state
            .current_stage()
            .map_or_else(String::new, |stage| stage.id.clone()),
        attempt: state.attempt(),
        product: WorkProduct::Branch {
            branch: "b".into(),
            head: head.into(),
            patch_id: patch.into(),
        },
    }
}

fn ok(state: &PipelineState, event: PipelineEvent) -> (PipelineState, Vec<PipelineAction>) {
    step(state, event.clone()).unwrap_or_else(|error| panic!("{event:?}: {error:?}"))
}

fn command_result(state: &PipelineState, exit_code: i32) -> PipelineEvent {
    PipelineEvent::CommandFinished {
        attempt: state.attempt(),
        stage_id: state.current_stage().unwrap().id.clone(),
        head: state.current_head().map(String::from),
        exit_code: Some(exit_code),
    }
}

/// Verifier r2 #1: a pick fanout whose approval is not right after it (rework
/// between them used to clear the children, so the pick never succeeded).
#[test]
fn verifier_r2_pick_fanout_must_be_followed_by_its_approval() {
    let workflow = repo_workflow(vec![
        work("w1", WorkOutput::Branch),
        pick_fanout("fan"),
        work("w2", WorkOutput::Branch),
        command("c"),
        approval("pick", true),
        merge(),
    ]);
    let errors = workflow.validate(&roles()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        WorkflowError::PickWithoutApproval { stage_id } if stage_id == "fan"
    )));
}

/// Verifier r2 #2: a plan work between a pick fanout and the pick.
#[test]
fn verifier_r2_plan_work_between_pick_fanout_and_pick_is_rejected() {
    let mut workflow = repo_workflow(vec![
        work("w", WorkOutput::Result),
        pick_fanout("fan"),
        WorkflowStage::new(
            "plan",
            Stage::Work {
                role: "planner".into(),
                instructions: String::new(),
                output: WorkOutput::Plan,
            },
        ),
        approval("pick", false),
    ]);
    workflow.requires.clear();
    let errors = workflow.validate(&roles()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        WorkflowError::PickWithoutApproval { stage_id } if stage_id == "fan"
    )));
}

/// Verifier r2 #3: a work stage after the last branch work in a merge
/// workflow could take a new head that no check covers.
#[test]
fn verifier_r2_work_after_the_last_branch_work_is_rejected_with_merge() {
    let workflow = repo_workflow(vec![
        work("w", WorkOutput::Branch),
        command("c"),
        approval("r", true),
        work("notes", WorkOutput::Result),
        merge(),
    ]);
    let errors = workflow.validate(&roles()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        WorkflowError::WorkAfterBranchWork { stage_id, .. } if stage_id == "notes"
    )));
}

/// Rework to a work stage after a pick fanout keeps that fanout's children;
/// rework to a stage at or before the fanout forgets them.
#[test]
fn rework_after_a_fanout_keeps_its_children() {
    let workflow = repo_workflow(vec![
        work("plan", WorkOutput::Result),
        pick_fanout("fan"),
        approval("pick", false),
        work("impl", WorkOutput::Branch),
        command("c"),
        approval("review", true),
        merge(),
    ]);
    let mut state = PipelineState::new("T", workflow.validated(&roles()).unwrap());
    state = ok(&state, PipelineEvent::Start).0;
    state = ok(
        &state,
        PipelineEvent::WorkCompleted {
            stage_id: state
                .current_stage()
                .map_or_else(String::new, |stage| stage.id.clone()),
            attempt: state.attempt(),
            product: WorkProduct::Result {
                summary: "s".into(),
                output: None,
            },
        },
    )
    .0;
    state = ok(
        &state,
        PipelineEvent::FanoutCompleted {
            attempt: state.attempt(),
            stage_id: "fan".into(),
            child_task_ids: vec!["a".into(), "b".into()],
            selected_child: None,
        },
    )
    .0;
    state = ok(
        &state,
        PipelineEvent::ApprovalGranted {
            attempt: state.attempt(),
            stage_id: "pick".into(),
            reviewer: "r".into(),
            head: None,
            selected_child: Some("b".into()),
        },
    )
    .0;
    state = ok(&state, branch(&state, "H1", "P1")).0;
    let failing = command_result(&state, 1);
    let (reworked, _) = ok(&state, failing);
    assert_eq!(reworked.current_stage().unwrap().id, "impl");
    // The pick cancelled the sibling, so only the winner is still a child.
    assert_eq!(reworked.fanout_child_task_ids(), ["b"]);
    assert_eq!(reworked.selected_fanout_child(), Some("b"));
}

/// Verifier r2 minor: several head changes pending when the merge fails are
/// all applied, but only one command runs, for the latest head.
#[test]
fn verifier_r2_merge_failure_with_several_pending_changes_runs_checks_once() {
    let workflow = repo_workflow(vec![
        work("w", WorkOutput::Branch),
        WorkflowStage::new(
            "s",
            Stage::Submit {
                forge: "local".into(),
            },
        ),
        command("c"),
        approval("r", true),
        merge(),
    ]);
    let mut state = PipelineState::new("T", workflow.validated(&roles()).unwrap());
    state = ok(&state, PipelineEvent::Start).0;
    state = ok(&state, branch(&state, "H1", "P1")).0;
    state = ok(
        &state,
        PipelineEvent::Submitted {
            stage_id: state
                .current_stage()
                .map_or_else(String::new, |stage| stage.id.clone()),
            attempt: state.attempt(),
            change_id: None,
        },
    )
    .0;
    let passing = command_result(&state, 0);
    state = ok(&state, passing).0;
    state = ok(
        &state,
        PipelineEvent::ApprovalGranted {
            attempt: state.attempt(),
            stage_id: "r".into(),
            reviewer: "x".into(),
            head: Some("H1".into()),
            selected_child: None,
        },
    )
    .0;
    assert!(state.merge_in_flight());
    let conflicted = ok(
        &state,
        PipelineEvent::MainAdvanced {
            rebased_head: "H1".into(),
            patch_id: "P1".into(),
            conflict: true,
        },
    )
    .0;
    state = ok(
        &state,
        PipelineEvent::CommitCreated {
            head: "H2".into(),
            patch_id: "P2".into(),
        },
    )
    .0;
    state = ok(
        &state,
        PipelineEvent::MainAdvanced {
            rebased_head: "H3".into(),
            patch_id: "P2".into(),
            conflict: false,
        },
    )
    .0;
    assert_eq!(
        step(&state, PipelineEvent::Cancel { reason: "x".into() }),
        Err(TransitionError::MergeInFlight)
    );
    let stale_failure = PipelineEvent::MergeFailed {
        stage_id: state
            .current_stage()
            .map_or_else(String::new, |stage| stage.id.clone()),
        attempt: state.attempt(),
        head: "H2".into(),
        reason: "r".into(),
    };
    assert_eq!(
        step(&state, stale_failure),
        Err(TransitionError::StaleResult)
    );
    let failed = PipelineEvent::MergeFailed {
        stage_id: state
            .current_stage()
            .map_or_else(String::new, |stage| stage.id.clone()),
        attempt: state.attempt(),
        head: "H1".into(),
        reason: "r".into(),
    };
    let (retry, actions) = ok(&state, failed.clone());
    let commands: Vec<_> = actions
        .iter()
        .filter_map(|action| match action {
            PipelineAction::RunCommand { head, .. } => Some(head.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(commands, [Some("H3".into())]);
    assert_eq!(retry.current_head(), Some("H3"));
    // The sent head merged after all: done, pending changes dropped.
    let (done, _) = ok(
        &state,
        PipelineEvent::MergeCompleted {
            stage_id: state
                .current_stage()
                .map_or_else(String::new, |stage| stage.id.clone()),
            attempt: state.attempt(),
            head: "H1".into(),
            merge_commit: "M".into(),
        },
    );
    assert_eq!(done.status(), PipelineStatus::Done);
    assert!(step(&done, failed).is_err());
    // A pending conflict sends the task back to its holder on failure.
    let (back, _) = ok(
        &conflicted,
        PipelineEvent::MergeFailed {
            stage_id: conflicted
                .current_stage()
                .map_or_else(String::new, |stage| stage.id.clone()),
            attempt: conflicted.attempt(),
            head: "H1".into(),
            reason: "r".into(),
        },
    );
    assert_eq!(back.current_stage().unwrap().id, "w");
}

/// Verifier r2 minor: main advancing before any branch exists is ignored.
#[test]
fn verifier_r2_main_advanced_before_any_branch_is_a_no_op() {
    let state = PipelineState::new("T", Workflow::builtin_code().validated(&roles()).unwrap());
    let (state, _) = ok(&state, PipelineEvent::Start);
    let (after, actions) = ok(
        &state,
        PipelineEvent::MainAdvanced {
            rebased_head: "H9".into(),
            patch_id: "P9".into(),
            conflict: false,
        },
    );
    assert!(actions.is_empty());
    assert_eq!(after, state);
}

/// The completability witness names the scenario and stage that got stuck.
#[test]
fn completability_witness_rejects_a_workflow_that_cannot_finish() {
    let mut workflow = Workflow::builtin_code();
    workflow.stages[2].stage = Stage::Command {
        command: "gh pr checks {pr}".into(),
    };
    // The local forge returns no change id, so `{pr}` never expands.
    let errors = workflow.validate(&roles()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        WorkflowError::NotCompletable { scenario, stage_id, .. }
            if scenario == "success script" && stage_id == "checks"
    )));
    for workflow in [
        Workflow::builtin_code(),
        Workflow::builtin_research(),
        Workflow::builtin_epic(),
        Workflow::builtin_planned(),
    ] {
        assert_eq!(
            workflow.validate(&["dev", "reviewer", "researcher", "planner"].map(String::from)),
            Ok(()),
            "{}",
            workflow.id
        );
    }
}

/// Verifier r3 (medium): without merge, a work stage after the last branch
/// work could take a new head and the task would finish with checks and
/// approvals covering an older one. Such workflows are rejected, like merge
/// workflows.
#[test]
fn verifier_r3_no_work_after_the_last_branch_work_when_anything_is_head_bound() {
    let workflow = repo_workflow(vec![
        work("w", WorkOutput::Branch),
        command("c"),
        approval("a", true),
        work("notes", WorkOutput::Result),
    ]);
    let errors = workflow.validate(&roles()).unwrap_err();
    assert!(errors.iter().any(|error| matches!(
        error,
        WorkflowError::WorkAfterBranchWork { stage_id, .. } if stage_id == "notes"
    )));
    // Without head-bound stages a later work stage stays allowed.
    let mut plain = repo_workflow(vec![
        work("w", WorkOutput::Branch),
        approval("a", false),
        work("notes", WorkOutput::Result),
    ]);
    plain.requires.clear();
    assert_eq!(plain.validate(&roles()), Ok(()));
}

/// Verifier r3 (medium): a pick fanout that runs again after a head change
/// makes new children, so the pick must be made again among them; the old
/// pick no longer counts, and the re-sent fanout carries the new head.
#[test]
fn verifier_r3_pick_fanout_rerun_requires_a_new_pick() {
    let workflow = repo_workflow(vec![
        work("w", WorkOutput::Branch),
        command("c"),
        pick_fanout("f"),
        approval("pick", false),
        WorkflowStage::new(
            "rev",
            Stage::Approval {
                by: Approver::Role("reviewer".into()),
                count: 1,
                bind_head: true,
            },
        ),
        merge(),
    ]);
    let mut state = PipelineState::new("T", workflow.validated(&roles()).unwrap());
    state = ok(&state, PipelineEvent::Start).0;
    state = ok(&state, branch(&state, "H1", "P1")).0;
    let passing = command_result(&state, 0);
    state = ok(&state, passing).0;
    state = ok(
        &state,
        PipelineEvent::FanoutCompleted {
            attempt: state.attempt(),
            stage_id: "f".into(),
            child_task_ids: vec!["a".into(), "b".into()],
            selected_child: None,
        },
    )
    .0;
    state = ok(
        &state,
        PipelineEvent::ApprovalGranted {
            attempt: state.attempt(),
            stage_id: "pick".into(),
            reviewer: "human".into(),
            head: None,
            selected_child: Some("a".into()),
        },
    )
    .0;
    state = ok(
        &state,
        PipelineEvent::CommitCreated {
            head: "H2".into(),
            patch_id: "P2".into(),
        },
    )
    .0;
    assert_eq!(state.current_stage().unwrap().id, "c");
    let passing = command_result(&state, 0);
    let (state, actions) = ok(&state, passing);
    assert!(actions.iter().any(|action| matches!(
        action,
        PipelineAction::Fanout {
            head: Some(head),
            work_product: Some(WorkProduct::Branch { head: product_head, .. }),
            ..
        } if head == "H2" && product_head == "H2"
    )));
    assert!(state.fanout_child_task_ids().is_empty());
    assert!(state.selected_fanout_child().is_none());
    assert!(
        state
            .approvals()
            .iter()
            .all(|approval| approval.stage_id != "pick")
    );
    let (state, _) = ok(
        &state,
        PipelineEvent::FanoutCompleted {
            attempt: state.attempt(),
            stage_id: "f".into(),
            child_task_ids: vec!["a2".into(), "b2".into()],
            selected_child: None,
        },
    );
    assert_eq!(
        state.current_stage().unwrap().id,
        "pick",
        "the pick is asked again"
    );
    let stale_pick = PipelineEvent::ApprovalGranted {
        attempt: state.attempt(),
        stage_id: "pick".into(),
        reviewer: "human".into(),
        head: None,
        selected_child: Some("a".into()),
    };
    assert_eq!(
        step(&state, stale_pick),
        Err(TransitionError::InvalidFanoutSelection)
    );
    let (state, _) = ok(
        &state,
        PipelineEvent::ApprovalGranted {
            attempt: state.attempt(),
            stage_id: "pick".into(),
            reviewer: "human".into(),
            head: None,
            selected_child: Some("b2".into()),
        },
    );
    let (state, _) = ok(
        &state,
        PipelineEvent::ApprovalGranted {
            attempt: state.attempt(),
            stage_id: "rev".into(),
            reviewer: "r".into(),
            head: Some("H2".into()),
            selected_child: None,
        },
    );
    let (done, _) = ok(
        &state,
        PipelineEvent::MergeCompleted {
            stage_id: state
                .current_stage()
                .map_or_else(String::new, |stage| stage.id.clone()),
            attempt: state.attempt(),
            head: "H2".into(),
            merge_commit: "M".into(),
        },
    );
    assert_eq!(done.status(), PipelineStatus::Done);
    assert_eq!(done.selected_fanout_child(), Some("b2"));
}

/// Verifier r3 probe: the witness rejects a known-bad shape (two fanouts with
/// head-bound approvals and a plan between them).
#[test]
fn verifier_r3_witness_rejects_a_known_bad_shape() {
    let mut workflow = repo_workflow(vec![
        work("w", WorkOutput::Branch),
        pick_fanout("f1"),
        approval("a1", true),
        WorkflowStage::new(
            "p2",
            Stage::Work {
                role: "planner".into(),
                instructions: String::new(),
                output: WorkOutput::Plan,
            },
        ),
        WorkflowStage::new(
            "f2",
            Stage::Fanout {
                source: FanoutSource::WorkOutput,
                join: FanoutJoin::All,
            },
        ),
        WorkflowStage::new(
            "a3",
            Stage::Approval {
                by: Approver::Role("reviewer".into()),
                count: 1,
                bind_head: true,
            },
        ),
    ]);
    workflow.id = "bad".into();
    assert!(workflow.validate(&roles()).is_err());
}

// ---- random workflow generator (ported from the verifier) ----

struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

fn random_workflow(rng: &mut SplitMix) -> Workflow {
    let count = 1 + rng.below(7);
    let mut stages = Vec::new();
    for index in 0..count {
        let stage = match rng.below(10) {
            0..=2 => Stage::Work {
                role: ["dev", "planner", "ghost"][rng.below(3)].into(),
                instructions: String::new(),
                output: [WorkOutput::Branch, WorkOutput::Result, WorkOutput::Plan][rng.below(3)],
            },
            3 => Stage::Submit {
                forge: ["local", "github"][rng.below(2)].into(),
            },
            4 => Stage::Command {
                command: ["true", "gh pr checks {pr}", "x {head} {branch}"][rng.below(3)].into(),
            },
            5 | 6 => Stage::Approval {
                by: if rng.chance(50) {
                    Approver::Human
                } else {
                    Approver::Role("reviewer".into())
                },
                count: 1 + rng.below(2) as u8,
                bind_head: rng.chance(60),
            },
            7 | 8 => Stage::Merge,
            _ => Stage::Fanout {
                source: if rng.chance(50) {
                    FanoutSource::WorkOutput
                } else {
                    FanoutSource::Listed(vec!["a".into(), "b".into()])
                },
                join: [FanoutJoin::All, FanoutJoin::First, FanoutJoin::Pick][rng.below(3)],
            },
        };
        let mut stage = WorkflowStage::new(format!("s{index}"), stage);
        if rng.chance(15) && index > 0 {
            stage.on_fail = Some(format!("s{}", rng.below(index)));
        }
        if rng.chance(10) {
            stage.on_timeout = Some(
                [
                    TimeoutAction::Notify,
                    TimeoutAction::Reassign,
                    TimeoutAction::Cancel,
                ][rng.below(3)],
            );
        }
        stages.push(stage);
    }
    Workflow {
        id: "g".into(),
        version: 1,
        requires: if rng.chance(80) {
            vec![WorkflowRequirement::Repo]
        } else {
            Vec::new()
        },
        allow_unreviewed: rng.chance(30),
        stages,
    }
}

static HEADS: AtomicU64 = AtomicU64::new(0);

fn fresh_head() -> String {
    format!("G{}", HEADS.fetch_add(1, Ordering::Relaxed))
}

/// Success events for the current stage; `variant` varies the pick choice.
fn success(state: &PipelineState, variant: usize) -> Vec<PipelineEvent> {
    if state.status() == PipelineStatus::Pending {
        return vec![PipelineEvent::Start];
    }
    let Some(stage) = state.current_stage() else {
        return Vec::new();
    };
    let stage_id = stage.id.clone();
    let head = state.current_head().map(String::from);
    match &stage.stage {
        Stage::Work { output, .. } => vec![PipelineEvent::WorkCompleted {
            stage_id: state
                .current_stage()
                .map_or_else(String::new, |stage| stage.id.clone()),
            attempt: state.attempt(),
            product: match output {
                WorkOutput::Branch => {
                    let head = fresh_head();
                    WorkProduct::Branch {
                        branch: "br".into(),
                        patch_id: format!("P{head}"),
                        head,
                    }
                }
                WorkOutput::Result => WorkProduct::Result {
                    summary: "s".into(),
                    output: None,
                },
                WorkOutput::Plan => WorkProduct::Plan {
                    items: vec!["i1".into(), "i2".into()],
                },
            },
        }],
        Stage::Submit { forge } => vec![PipelineEvent::Submitted {
            stage_id: state
                .current_stage()
                .map_or_else(String::new, |stage| stage.id.clone()),
            attempt: state.attempt(),
            change_id: (forge != "local").then(|| "9".into()),
        }],
        Stage::Command { .. } => vec![PipelineEvent::CommandFinished {
            attempt: state.attempt(),
            stage_id,
            head,
            exit_code: Some(0),
        }],
        Stage::Approval {
            count, bind_head, ..
        } => {
            let children = state.fanout_child_task_ids().to_vec();
            let selected = if children.is_empty() || variant == 2 {
                None
            } else {
                Some(children[variant % children.len()].clone())
            };
            // Only reviewers not yet counted in this attempt approve.
            (0..usize::from(*count) * 2)
                .map(|reviewer| format!("r{reviewer}"))
                .filter(|reviewer| !state.approval_reviewers().contains(reviewer))
                .take(usize::from(*count).saturating_sub(state.approval_reviewers().len()))
                .map(|reviewer| PipelineEvent::ApprovalGranted {
                    attempt: state.attempt(),
                    stage_id: stage_id.clone(),
                    reviewer,
                    head: if *bind_head { head.clone() } else { None },
                    selected_child: selected.clone(),
                })
                .collect()
        }
        Stage::Fanout { source, join } => {
            let children: Vec<String> = match source {
                FanoutSource::Listed(children) => children.clone(),
                FanoutSource::WorkOutput => vec!["k1".into(), "k2".into()],
            };
            vec![PipelineEvent::FanoutCompleted {
                attempt: state.attempt(),
                stage_id,
                selected_child: (*join == FanoutJoin::First).then(|| children[0].clone()),
                child_task_ids: children,
            }]
        }
        Stage::Merge => vec![PipelineEvent::MergeCompleted {
            stage_id: state
                .current_stage()
                .map_or_else(String::new, |stage| stage.id.clone()),
            attempt: state.attempt(),
            head: head.unwrap_or_default(),
            merge_commit: "M".into(),
        }],
    }
}

/// Drive with success events (trying the pick variants) until terminal.
fn drive(mut state: PipelineState) -> Result<PipelineState, String> {
    for _ in 0..100 {
        if state.status().is_terminal() {
            return Ok(state);
        }
        let mut last_error = String::new();
        let mut advanced = false;
        for variant in 0..3 {
            let mut trial = state.clone();
            let mut accepted = true;
            for event in success(&state, variant) {
                match step(&trial, event.clone()) {
                    Ok((next, _)) => trial = next,
                    Err(error) => {
                        last_error = format!("{event:?} -> {error:?}");
                        accepted = false;
                        break;
                    }
                }
            }
            if accepted {
                state = trial;
                advanced = true;
                break;
            }
        }
        if !advanced {
            return Err(format!(
                "stuck at stage {:?}: {last_error}",
                state.current_stage().map(|stage| stage.id.clone())
            ));
        }
    }
    Err("no progress in 100 steps".into())
}

fn disturbance(rng: &mut SplitMix, state: &PipelineState) -> PipelineEvent {
    let stage_id = state
        .current_stage()
        .map(|stage| stage.id.clone())
        .unwrap_or_default();
    let head = state.current_head().map(String::from);
    match rng.below(9) {
        0 => PipelineEvent::CommandFinished {
            attempt: state.attempt(),
            stage_id,
            head,
            exit_code: Some(1),
        },
        1 => PipelineEvent::ChangesRequested {
            attempt: state.attempt(),
            stage_id,
            reviewer: "r0".into(),
            head,
            reason: "x".into(),
        },
        2 => {
            let head = fresh_head();
            PipelineEvent::CommitCreated {
                patch_id: format!("P{head}"),
                head,
            }
        }
        3 => PipelineEvent::MainAdvanced {
            rebased_head: fresh_head(),
            patch_id: if rng.chance(50) {
                state.patch_id().unwrap_or("z").into()
            } else {
                "other".into()
            },
            conflict: rng.chance(30),
        },
        4 => PipelineEvent::MergeFailed {
            stage_id: state
                .current_stage()
                .map_or_else(String::new, |stage| stage.id.clone()),
            attempt: state.attempt(),
            head: head.unwrap_or_default(),
            reason: "x".into(),
        },
        5 => PipelineEvent::StageTimedOut {
            attempt: state.attempt(),
            stage_id,
        },
        6 => PipelineEvent::StageFailed {
            attempt: state.attempt(),
            stage_id,
            reason: "x".into(),
        },
        _ => {
            let variant = rng.below(3);
            success(state, variant)
                .into_iter()
                .next()
                .unwrap_or(PipelineEvent::Start)
        }
    }
}

fn check_random_workflows(seed: u64, count: usize) {
    let mut rng = SplitMix(seed);
    let mut accepted = 0;
    let mut failures: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for _ in 0..count {
        let workflow = random_workflow(&mut rng);
        let Ok(validated) = workflow.clone().validated(&roles()) else {
            continue;
        };
        accepted += 1;
        let start = PipelineState::new("T", validated);
        if let Err(error) = drive(start.clone()) {
            failures
                .entry(format!("success script: {error}"))
                .or_insert((0, format!("{workflow:?}")))
                .0 += 1;
            continue;
        }
        for _ in 0..5 {
            let mut state = start.clone();
            let mut trace = Vec::new();
            for _ in 0..25 {
                if state.status().is_terminal() {
                    break;
                }
                let event = disturbance(&mut rng, &state);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    step(&state, event.clone())
                }));
                let Ok(result) = result else {
                    failures
                        .entry("panic".into())
                        .or_insert((0, format!("{workflow:?} {trace:?} {event:?}")))
                        .0 += 1;
                    break;
                };
                let Ok((next, actions)) = result else {
                    continue;
                };
                trace.push(event);
                for action in &actions {
                    if let PipelineAction::Merge { head, .. } = action
                        && next.current_head() != Some(head.as_str())
                    {
                        failures
                            .entry("merge action for another head".into())
                            .or_insert((0, format!("{workflow:?} {trace:?}")))
                            .0 += 1;
                    }
                }
                state = next;
                if state.status() == PipelineStatus::Running
                    && let Err(error) = drive(state.clone())
                {
                    failures
                        .entry(format!("after disturbance: {error}"))
                        .or_insert((0, format!("{workflow:?}\ntrace {trace:?}")))
                        .0 += 1;
                    break;
                }
            }
        }
    }
    eprintln!("random workflows (seed {seed:#x}): generated {count}, accepted {accepted}");
    for (kind, (times, example)) in &failures {
        eprintln!(
            "FAIL x{times}: {kind}\n  e.g. {}",
            &example[..example.len().min(1500)]
        );
    }
    assert!(
        accepted > count / 50,
        "too few accepted workflows: {accepted}"
    );
    assert!(failures.is_empty(), "{} failure kinds", failures.len());
}

/// Every workflow `validate` accepts finishes under an independent success
/// driver, also after random disturbances.
#[test]
fn random_accepted_workflows_always_finish() {
    check_random_workflows(0x5EED_2026_0925_0002, 200_000);
}

/// Deeper run: `cargo test -p agend-core --test workflow_completability -- --ignored`.
#[test]
#[ignore = "deep run: 1,000,000 more random workflows"]
fn random_accepted_workflows_always_finish_deep() {
    check_random_workflows(0x00C0_FFEE_2026_0925, 1_000_000);
}
