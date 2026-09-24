//! Event identity (verifier round r4 on gate 1).
//!
//! Every result event carries the identity of the stage attempt that asked
//! for it — `stage_id`, `attempt` and, for a head-bound stage, `head` — and
//! `step` rejects any other result with `StaleResult` without changing
//! anything. While the merge is in flight only its result or a head change is
//! accepted. A pick is fixed only when the approval count is met. These are
//! the verifier's r4 counterexamples as named regression tests.

use agend_core::pipeline::stage::FanoutJoin;
use agend_core::pipeline::state::{
    PipelineAction, PipelineEvent, PipelineState, PipelineStatus, TransitionError, WorkProduct,
    step,
};
use agend_core::pipeline::workflow::{
    Approver, FanoutSource, Stage, WorkOutput, Workflow, WorkflowRequirement, WorkflowStage,
};

fn roles() -> Vec<String> {
    ["dev", "reviewer", "planner", "qa"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn stage(id: &str, stage: Stage) -> WorkflowStage {
    WorkflowStage::new(id, stage)
}

fn work(role: &str, output: WorkOutput) -> Stage {
    Stage::Work {
        role: role.into(),
        instructions: String::new(),
        output,
    }
}

fn command() -> Stage {
    Stage::Command {
        command: "make check".into(),
    }
}

fn approval(count: u8, bind_head: bool) -> Stage {
    Stage::Approval {
        by: Approver::Role("reviewer".into()),
        count,
        bind_head,
    }
}

fn workflow(stages: Vec<WorkflowStage>) -> Workflow {
    Workflow {
        id: "p".into(),
        version: 1,
        requires: vec![WorkflowRequirement::Repo],
        allow_unreviewed: false,
        stages,
    }
}

fn start(workflow: Workflow) -> PipelineState {
    let state = PipelineState::new("T", workflow.validated(&roles()).expect("valid"));
    ok(&state, PipelineEvent::Start).0
}

fn ok(state: &PipelineState, event: PipelineEvent) -> (PipelineState, Vec<PipelineAction>) {
    step(state, event.clone()).unwrap_or_else(|error| panic!("{event:?}: {error:?}"))
}

fn current(state: &PipelineState) -> (String, u32) {
    (state.current_stage().unwrap().id.clone(), state.attempt())
}

fn branch(state: &PipelineState, head: &str) -> PipelineEvent {
    let (stage_id, attempt) = current(state);
    PipelineEvent::WorkCompleted {
        stage_id,
        attempt,
        product: WorkProduct::Branch {
            branch: "b".into(),
            head: head.into(),
            patch_id: format!("P-{head}"),
        },
    }
}

fn submitted(state: &PipelineState) -> PipelineEvent {
    let (stage_id, attempt) = current(state);
    PipelineEvent::Submitted {
        stage_id,
        attempt,
        change_id: None,
    }
}

fn passed(state: &PipelineState) -> PipelineEvent {
    let (stage_id, attempt) = current(state);
    PipelineEvent::CommandFinished {
        stage_id,
        attempt,
        head: state.current_head().map(String::from),
        exit_code: Some(0),
    }
}

fn approve(state: &PipelineState, reviewer: &str, pick: Option<&str>) -> PipelineEvent {
    let (stage_id, attempt) = current(state);
    PipelineEvent::ApprovalGranted {
        stage_id,
        attempt,
        reviewer: reviewer.into(),
        head: state.current_head().map(String::from),
        selected_child: pick.map(String::from),
    }
}

fn fanout_done(state: &PipelineState, children: &[&str]) -> PipelineEvent {
    let (stage_id, attempt) = current(state);
    PipelineEvent::FanoutCompleted {
        stage_id,
        attempt,
        child_task_ids: children.iter().map(|child| (*child).into()).collect(),
        selected_child: None,
    }
}

fn commit(head: &str) -> PipelineEvent {
    PipelineEvent::CommitCreated {
        head: head.into(),
        patch_id: format!("P-{head}"),
    }
}

/// r4 important 1: StageFailed while the merge is in flight must not move the
/// task out of merge; the forge's completion of the sent merge still counts.
#[test]
fn verifier_r4_stage_failed_during_an_in_flight_merge_is_rejected() {
    let mut code = Workflow::builtin_code();
    code.stages.last_mut().unwrap().on_fail = Some("work".into());
    let mut state = start(code);
    state = ok(&state, branch(&state, "H1")).0;
    state = ok(&state, submitted(&state)).0;
    state = ok(&state, passed(&state)).0;
    state = ok(&state, approve(&state, "r", None)).0;
    assert!(state.merge_in_flight());
    let (stage_id, attempt) = current(&state);
    for event in [
        PipelineEvent::StageFailed {
            stage_id: stage_id.clone(),
            attempt,
            reason: "x".into(),
        },
        PipelineEvent::StageTimedOut {
            stage_id: stage_id.clone(),
            attempt,
        },
        PipelineEvent::Cancel { reason: "x".into() },
    ] {
        assert_eq!(
            step(&state, event.clone()),
            Err(TransitionError::MergeInFlight),
            "{event:?}"
        );
    }
    let (done, _) = ok(
        &state,
        PipelineEvent::MergeCompleted {
            stage_id,
            attempt,
            head: "H1".into(),
            merge_commit: "M1".into(),
        },
    );
    assert_eq!(done.status(), PipelineStatus::Done);
}

/// r4 important 2: a pick reviewer's choice below the approval count must
/// not survive a head change: the reviewers at the new head have to pick.
#[test]
fn verifier_r4_partial_pick_does_not_survive_a_head_change() {
    let mut state = start(workflow(vec![
        stage("work", work("dev", WorkOutput::Branch)),
        stage(
            "fan",
            Stage::Fanout {
                source: FanoutSource::Listed(vec!["a".into(), "b".into()]),
                join: FanoutJoin::Pick,
            },
        ),
        stage("pick", approval(2, true)),
        stage("checks", command()),
    ]));
    state = ok(&state, branch(&state, "H1")).0;
    state = ok(&state, fanout_done(&state, &["a", "b"])).0;
    state = ok(&state, approve(&state, "r1", Some("a"))).0;
    assert_eq!(state.pending_pick(), Some("a"));
    assert_eq!(
        state.selected_fanout_child(),
        None,
        "not fixed below the count"
    );
    state = ok(&state, commit("H2")).0;
    assert_eq!(state.pending_pick(), None);
    assert!(state.approval_reviewers().is_empty());
    // r1's choice is gone: an approval at H2 without a choice is refused.
    assert_eq!(
        step(&state, approve(&state, "r2", None)),
        Err(TransitionError::FanoutSelectionRequired)
    );
    // Starting over at H2: both reviewers choose b.
    let mut state = start(workflow(vec![
        stage("work", work("dev", WorkOutput::Branch)),
        stage(
            "fan",
            Stage::Fanout {
                source: FanoutSource::Listed(vec!["a".into(), "b".into()]),
                join: FanoutJoin::Pick,
            },
        ),
        stage("pick", approval(2, true)),
        stage("checks", command()),
    ]));
    state = ok(&state, branch(&state, "H1")).0;
    state = ok(&state, fanout_done(&state, &["a", "b"])).0;
    state = ok(&state, approve(&state, "r1", Some("a"))).0;
    state = ok(&state, commit("H2")).0;
    state = ok(&state, approve(&state, "r2", Some("b"))).0;
    let (state, actions) = ok(&state, approve(&state, "r3", Some("b")));
    assert_eq!(state.selected_fanout_child(), Some("b"));
    assert!(actions.contains(&PipelineAction::CancelFanoutSiblings {
        winner_task_id: "b".into(),
        sibling_task_ids: vec!["a".into()],
    }));
}

/// r4 medium 3: the completion of an earlier fanout run is stale.
#[test]
fn verifier_r4_completion_of_an_earlier_fanout_run_is_stale() {
    let mut state = start(workflow(vec![
        stage("plan", work("planner", WorkOutput::Plan)),
        stage("work", work("dev", WorkOutput::Branch)),
        stage("checks", command()),
        stage(
            "fan",
            Stage::Fanout {
                source: FanoutSource::WorkOutput,
                join: FanoutJoin::All,
            },
        ),
        stage("review", approval(1, true)),
    ]));
    let (stage_id, attempt) = current(&state);
    state = ok(
        &state,
        PipelineEvent::WorkCompleted {
            stage_id,
            attempt,
            product: WorkProduct::Plan {
                items: vec!["x".into()],
            },
        },
    )
    .0;
    state = ok(&state, branch(&state, "H1")).0;
    state = ok(&state, passed(&state)).0;
    let first_run = fanout_done(&state, &["child-of-H1-run"]);
    assert_eq!(state.attempt(), 1);
    state = ok(&state, commit("H2")).0;
    state = ok(&state, passed(&state)).0;
    assert_eq!(current(&state), ("fan".into(), 2));
    assert_eq!(step(&state, first_run), Err(TransitionError::StaleResult));
    let (state, _) = ok(&state, fanout_done(&state, &["child-of-H2-run"]));
    assert_eq!(state.fanout_child_task_ids(), ["child-of-H2-run"]);
}

/// r4 medium 4: a duplicate completion for one work stage must not complete
/// the next work stage.
#[test]
fn verifier_r4_duplicate_work_completion_does_not_complete_the_next_work_stage() {
    let mut state = start(workflow(vec![
        stage("work1", work("dev", WorkOutput::Branch)),
        stage("work2", work("qa", WorkOutput::Branch)),
        stage("checks", command()),
        stage("review", approval(1, true)),
    ]));
    let first = branch(&state, "H1");
    state = ok(&state, first.clone()).0;
    assert_eq!(current(&state), ("work2".into(), 1));
    assert_eq!(step(&state, first), Err(TransitionError::StaleResult));
    let submitted_twice = submitted(&state);
    assert!(
        step(&state, submitted_twice).is_err(),
        "no submit stage here"
    );
}

/// r4 low: a workflow without branch work has no head to change; choices
/// never list cancelled children.
#[test]
fn verifier_r4_commit_without_branch_work_and_choices_without_cancelled_children() {
    let research = PipelineState::new(
        "T",
        Workflow::builtin_research()
            .validated(&["researcher".into(), "reviewer".into()])
            .unwrap(),
    );
    let research = ok(&research, PipelineEvent::Start).0;
    assert_eq!(
        step(&research, commit("H1")),
        Err(TransitionError::WrongStage)
    );

    // Pick b, then a new commit sends the task back to the head-bound pick
    // (the fanout is before the recheck point): only b is still offered.
    let mut state = start(workflow(vec![
        stage("work", work("dev", WorkOutput::Branch)),
        stage(
            "fan",
            Stage::Fanout {
                source: FanoutSource::Listed(vec!["a".into(), "b".into()]),
                join: FanoutJoin::Pick,
            },
        ),
        stage("pick", approval(1, true)),
        stage("checks", command()),
    ]));
    state = ok(&state, branch(&state, "H1")).0;
    state = ok(&state, fanout_done(&state, &["a", "b"])).0;
    state = ok(&state, approve(&state, "r1", Some("b"))).0;
    let (state, actions) = ok(&state, commit("H2"));
    assert_eq!(current(&state), ("pick".into(), 2));
    assert!(actions.iter().any(|action| matches!(
        action,
        PipelineAction::RequestApproval { choices, .. } if choices == &["b".to_string()]
    )));
    assert_eq!(
        state.selected_fanout_child(),
        None,
        "the pick is made again"
    );
}

/// Actions carry the identity the result must echo.
#[test]
fn actions_carry_the_identity_results_must_echo() {
    let state = PipelineState::new(
        "T",
        Workflow::builtin_code()
            .validated(&["dev".into(), "reviewer".into()])
            .unwrap(),
    );
    let (state, actions) = ok(&state, PipelineEvent::Start);
    assert!(actions.contains(&PipelineAction::AssignWork {
        stage_id: "work".into(),
        attempt: 1,
        role: "dev".into(),
    }));
    let (state, actions) = ok(&state, branch(&state, "H1"));
    assert!(actions.contains(&PipelineAction::Submit {
        stage_id: "submit".into(),
        attempt: 1,
        forge: "local".into(),
    }));
    let (state, _) = ok(&state, submitted(&state));
    let failing = PipelineEvent::CommandFinished {
        stage_id: "checks".into(),
        attempt: 1,
        head: Some("H1".into()),
        exit_code: Some(1),
    };
    let (state, actions) = ok(&state, failing);
    assert!(actions.iter().any(|action| matches!(
        action,
        PipelineAction::ReturnToWork { stage_id, attempt: 2, .. } if stage_id == "work"
    )));
    // The first attempt's completion is now stale; the second one's counts.
    let stale = PipelineEvent::WorkCompleted {
        stage_id: "work".into(),
        attempt: 1,
        product: WorkProduct::Branch {
            branch: "b".into(),
            head: "H2".into(),
            patch_id: "P2".into(),
        },
    };
    assert_eq!(step(&state, stale), Err(TransitionError::StaleResult));
    assert!(step(&state, branch(&state, "H2")).is_ok());
}
