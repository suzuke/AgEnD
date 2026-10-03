//! Persisted pipeline JSON: golden, old fixture and hostile restore inputs.
use agend_core::pipeline::state::{
    PipelineEvent, PipelineSnapshot, PipelineState, outstanding_actions, step,
};
use agend_core::pipeline::workflow::Workflow;
use serde_json::{Value, json};
use std::path::PathBuf;
fn workflow() -> agend_core::pipeline::workflow::ValidatedWorkflow {
    Workflow::builtin_research()
        .validated(&["researcher".into(), "reviewer".into()])
        .unwrap()
}
fn initial() -> PipelineState {
    PipelineState::new("t-1", workflow())
}
fn restore(value: Value) -> Result<PipelineState, String> {
    let snapshot: PipelineSnapshot = serde_json::from_value(value).map_err(|e| e.to_string())?;
    PipelineState::restore(snapshot, workflow())
}
#[test]
fn persisted_snapshot_is_golden_and_contains_no_workflow() {
    let state = initial();
    let text = serde_json::to_string_pretty(&state.snapshot()).unwrap() + "\n";
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/pipeline-pending.json");
    if std::env::var_os("AGEND_BLESS_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    assert!(!text.contains("workflow"));
    assert_eq!(
        restore(serde_json::from_str(&text).unwrap()).unwrap(),
        state
    );
}
#[test]
fn old_snapshot_without_rework_reason_still_restores() {
    let text = include_str!("fixtures/pipeline-v1.json");
    let restored = restore(serde_json::from_str(text).unwrap()).unwrap();
    let (expected, _) = step(&initial(), PipelineEvent::Start).unwrap();
    assert_eq!(restored, expected);
    assert_eq!(
        outstanding_actions(&restored),
        outstanding_actions(&expected)
    );
}
#[test]
fn tampered_snapshots_fail_without_panics_or_dispatch() {
    let (running, _) = step(&initial(), PipelineEvent::Start).unwrap();
    let original = serde_json::to_value(running.snapshot()).unwrap();
    for (key, value) in [
        ("stage_index", json!(999)),
        ("attempts", json!([])),
        ("attempts", json!([4294967295u32, 0])),
        ("attempts", json!([0, 0])),
        ("task_id", json!("../outside")),
        ("passed_checks", json!([{"stage_id":"missing","head":null}])),
        ("approval_reviewers", json!(["forged"])),
        ("current_head", json!("unpaired")),
        ("notified_timeout", json!([999, 1])),
        (
            "pending_head_changes",
            json!([{"CommitCreated":{"head":"h","patch_id":"p"}}]),
        ),
    ] {
        let mut value_to_restore = original.clone();
        value_to_restore[key] = value;
        assert!(
            restore(value_to_restore).is_err(),
            "accepted hostile field {key}"
        );
    }
    let mut pending = serde_json::to_value(initial().snapshot()).unwrap();
    pending["branch"] = json!("agend/t-1/forged");
    assert!(restore(pending).is_err());
}
