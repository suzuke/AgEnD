use agend_core::model::Backend;
use agend_core::pipeline::state::{
    PendingHeadChange, PipelineAction, PipelineEvent, PipelineState, WorkProduct, step,
};
use agend_core::pipeline::workflow::Workflow;
use agend_core::policy::assign::{
    AssignmentDecision, AssignmentRequest, Candidate, Purpose, RoleCapacity, choose,
};
use agend_core::policy::busy::{BusyLevel, effective_level};
use agend_core::policy::debounce::{AgentActivity, DebounceState};
use agend_core::protocol::client::ClientRequest;

fn main() {
    let ClientRequest::Hello { data: hello } = ClientRequest::hello() else {
        unreachable!("hello constructor returns hello");
    };
    let version = hello.supported[0];
    println!("client hello: supports {}.{}", version.major, version.minor);
    println!("busy levels for steer:");
    for backend in Backend::ALL {
        println!(
            "  {}: {:?}",
            backend.as_str(),
            effective_level(backend, BusyLevel::Steer)
        );
    }

    let mut debounce = DebounceState::new(AgentActivity::Busy);
    let immediate = debounce.observe(AgentActivity::Idle, 1_000);
    let early = debounce.advance(5_999);
    let stable = debounce.advance(6_000);
    let busy_now = debounce.observe(AgentActivity::Busy, 7_000);
    println!(
        "debounce: idle immediate={immediate}, before 5s={early}, at 5s={stable}; busy immediate={busy_now}, state={:?}",
        debounce.effective,
    );

    let request = AssignmentRequest {
        team_id: "team-a".into(),
        role: "dev".into(),
        allowed_backends: vec![Backend::Claude, Backend::Codex],
        available_backends: vec![Backend::Claude, Backend::Codex],
        role_capacity: Some(RoleCapacity {
            minimum_instances: 1,
            maximum_instances: 2,
            current_instances: 1,
        }),
        purpose: Purpose::NewTask,
    };
    let candidates = [Candidate {
        instance_id: "dev-1".into(),
        team_id: "team-a".into(),
        role: "dev".into(),
        backend: Backend::Codex,
        held_task: None,
        ephemeral: false,
        usage_available: true,
    }];
    let assignment = choose(&request, &candidates);
    let assigned = match assignment {
        AssignmentDecision::Assigned { instance_id } => instance_id,
        other => panic!("demo assignment unexpectedly returned {other:?}"),
    };
    let reviewer_request = AssignmentRequest {
        team_id: "team-a".into(),
        role: "reviewer".into(),
        allowed_backends: vec![Backend::Claude, Backend::Codex],
        available_backends: vec![Backend::Claude, Backend::Codex],
        role_capacity: Some(RoleCapacity {
            minimum_instances: 1,
            maximum_instances: 2,
            current_instances: 1,
        }),
        purpose: Purpose::Review {
            task_holder: assigned.clone(),
            task_holder_backend: Backend::Codex,
        },
    };
    let reviewer_candidates = [Candidate {
        instance_id: "review-1".into(),
        team_id: "team-a".into(),
        role: "reviewer".into(),
        backend: Backend::Claude,
        held_task: None,
        ephemeral: false,
        usage_available: true,
    }];
    let reviewer = match choose(&reviewer_request, &reviewer_candidates) {
        AssignmentDecision::Assigned { instance_id } => instance_id,
        other => panic!("demo reviewer assignment unexpectedly returned {other:?}"),
    };

    let workflow = Workflow::builtin_code();
    let roles = ["dev".to_string(), "reviewer".to_string()];
    let validated = workflow
        .clone()
        .validated(&roles)
        .expect("built-in code workflow is valid");
    let mut state = PipelineState::new("T-1", validated);
    println!(
        "task {} workflow={} v{}",
        state.task_id(),
        workflow.id,
        workflow.version
    );
    println!("  assignment -> {assigned}");
    println!("  reviewer   -> {reviewer}");

    state = transition(&state, "start", PipelineEvent::Start);
    state = transition(&state, "work", branch("H0", "P0"));
    state = transition(&state, "submit", submitted());
    state = transition(&state, "command failed", command_result(&state, Some(1)));
    state = transition(&state, "work retry", branch("H1", "P1"));
    state = transition(&state, "submit", submitted());
    state = transition(&state, "command passed", command_result(&state, Some(0)));
    state = transition(
        &state,
        "changes requested",
        request_changes(&state, &reviewer),
    );
    state = transition(
        &state,
        "rework, main advanced",
        PipelineEvent::MainAdvanced {
            rebased_head: "H1b".into(),
            patch_id: "P1".into(),
            conflict: false,
        },
    );
    state = transition(&state, "rework done", branch("H1c", "P1c"));
    state = transition(&state, "submit", submitted());
    state = transition(&state, "command passed", command_result(&state, Some(0)));
    state = transition(
        &state,
        "new commit",
        PipelineEvent::CommitCreated {
            head: "H2".into(),
            patch_id: "P2".into(),
        },
    );
    state = transition(&state, "command passed", command_result(&state, Some(0)));
    state = transition(
        &state,
        "changed rebase",
        PipelineEvent::MainAdvanced {
            rebased_head: "H3".into(),
            patch_id: "P3".into(),
            conflict: false,
        },
    );
    state = transition(&state, "work after changed patch", branch("H4", "P4"));
    state = transition(&state, "submit", submitted());
    state = transition(&state, "command passed", command_result(&state, Some(0)));
    state = transition(&state, "approval H4", approve(&state, &reviewer));
    state = transition(
        &state,
        "main advanced in flight",
        PipelineEvent::MainAdvanced {
            rebased_head: "H5".into(),
            patch_id: "P4".into(),
            conflict: false,
        },
    );
    state = transition(
        &state,
        "merge failed",
        PipelineEvent::MergeFailed {
            head: "H4".into(),
            reason: "main moved".into(),
        },
    );
    state = transition(&state, "checks rerun", command_result(&state, Some(0)));
    state = transition(
        &state,
        "merge completed",
        PipelineEvent::MergeCompleted {
            head: "H5".into(),
            merge_commit: "M1".into(),
        },
    );
    assert_eq!(state.merge_commit(), Some("M1"));
}

fn branch(head: &str, patch_id: &str) -> PipelineEvent {
    PipelineEvent::WorkCompleted {
        product: WorkProduct::Branch {
            branch: "agend/T-1/demo".into(),
            head: head.into(),
            patch_id: patch_id.into(),
        },
    }
}

fn submitted() -> PipelineEvent {
    PipelineEvent::Submitted { change_id: None }
}

fn stage_id(state: &PipelineState) -> String {
    state.current_stage().expect("current stage").id.clone()
}

fn command_result(state: &PipelineState, exit_code: Option<i32>) -> PipelineEvent {
    PipelineEvent::CommandFinished {
        stage_id: stage_id(state),
        head: state.current_head().map(String::from),
        exit_code,
    }
}

fn approve(state: &PipelineState, reviewer: &str) -> PipelineEvent {
    PipelineEvent::ApprovalGranted {
        stage_id: stage_id(state),
        reviewer: reviewer.into(),
        head: state.current_head().map(String::from),
        selected_child: None,
    }
}

fn request_changes(state: &PipelineState, reviewer: &str) -> PipelineEvent {
    PipelineEvent::ChangesRequested {
        stage_id: stage_id(state),
        reviewer: reviewer.into(),
        head: state.current_head().map(String::from),
        reason: "rename the flag".into(),
    }
}

fn transition(state: &PipelineState, label: &str, event: PipelineEvent) -> PipelineState {
    let (next, actions) = step(state, event).unwrap_or_else(|error| panic!("{label}: {error}"));
    let summaries = actions
        .iter()
        .filter_map(action_summary)
        .collect::<Vec<_>>();
    let outcome = if summaries.is_empty() {
        let pending = next
            .pending_head_changes()
            .iter()
            .map(|change| match change {
                PendingHeadChange::CommitCreated { head, .. }
                | PendingHeadChange::MainAdvanced {
                    rebased_head: head, ..
                } => head.as_str(),
            })
            .collect::<Vec<_>>();
        let pending = if pending.is_empty() {
            String::new()
        } else {
            format!(", {} pending until the merge result", pending.join(", "))
        };
        format!(
            "stay in {} at {}{pending}",
            stage_id(&next),
            next.current_head().unwrap_or("<no head>")
        )
    } else {
        summaries.join(", ")
    };
    println!("  {label:<25} -> {outcome}");
    next
}

fn action_summary(action: &PipelineAction) -> Option<String> {
    match action {
        PipelineAction::ScheduleTimeout { .. } => None,
        PipelineAction::AssignWork { stage_id, role } => {
            Some(format!("assign role {role} ({stage_id})"))
        }
        PipelineAction::Submit { forge, .. } => Some(format!("submit via {forge}")),
        PipelineAction::RunCommand {
            stage_id,
            command,
            head,
            ..
        } => Some(format!(
            "run {stage_id} ({command}) at {}",
            head.as_deref().unwrap_or("<no head>")
        )),
        PipelineAction::RequestApproval {
            stage_id,
            bind_head,
            ..
        } => Some(format!(
            "request {stage_id} approval (head-bound={bind_head})"
        )),
        PipelineAction::Merge { head, .. } => Some(format!("merge at {head}")),
        PipelineAction::ReturnToWork {
            stage_id, reason, ..
        } => Some(format!("return {stage_id} to its task holder ({reason})")),
        PipelineAction::TaskDone { merge_commit } => Some(format!("task done at {merge_commit:?}")),
        other => Some(format!("{other:?}")),
    }
}
