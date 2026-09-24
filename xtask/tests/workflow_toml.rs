//! Golden TOML tests for the persisted workflow format (D19 stores workflows
//! as TOML; D32 lets core derive serde for the workflow definition types).
//! Like `protocol_compat.rs` for the wire protocol, these lock the exact text
//! a saved workflow has: a change to the serde shape of `Workflow`,
//! `WorkflowStage`, `Stage` or their field types fails here. Each golden file
//! must also parse back into the same value and pass `Workflow::validate`.
//!
//! To regenerate after an intended format change (then review the diff):
//! `AGEND_BLESS_GOLDEN=1 cargo test -p xtask --test workflow_toml`.

use agend_core::pipeline::stage::FanoutJoin;
use agend_core::pipeline::workflow::{
    Approver, FanoutSource, Stage, TimeoutAction, WorkOutput, Workflow, WorkflowRequirement,
    WorkflowStage,
};
use std::path::PathBuf;

fn roles() -> Vec<String> {
    ["dev", "reviewer", "researcher", "planner"]
        .into_iter()
        .map(String::from)
        .collect()
}

/// A custom workflow that uses every optional field and every stage kind's
/// parameters that the built-ins leave out.
fn custom() -> Workflow {
    let mut checks = WorkflowStage::new(
        "checks",
        Stage::Command {
            command: "gh pr checks {pr} --watch".into(),
        },
    );
    checks.timeout_ms = Some(900_000);
    checks.on_timeout = Some(TimeoutAction::Reassign);
    checks.on_fail = Some("work".into());
    let mut human = WorkflowStage::new(
        "human",
        Stage::Approval {
            by: Approver::Human,
            count: 2,
            bind_head: true,
        },
    );
    human.on_timeout = Some(TimeoutAction::Cancel);
    Workflow {
        id: "custom".into(),
        version: 3,
        requires: vec![WorkflowRequirement::Repo],
        allow_unreviewed: false,
        stages: vec![
            WorkflowStage::new(
                "split",
                Stage::Work {
                    role: "planner".into(),
                    instructions: "Split the work.".into(),
                    output: WorkOutput::Plan,
                },
            ),
            WorkflowStage::new(
                "fanout",
                Stage::Fanout {
                    source: FanoutSource::Listed(vec!["api".into(), "ui".into()]),
                    join: FanoutJoin::Pick,
                },
            ),
            WorkflowStage::new(
                "pick",
                Stage::Approval {
                    by: Approver::Role("reviewer".into()),
                    count: 1,
                    bind_head: false,
                },
            ),
            WorkflowStage::new(
                "work",
                Stage::Work {
                    role: "dev".into(),
                    instructions: "Implement the picked design.".into(),
                    output: WorkOutput::Branch,
                },
            ),
            WorkflowStage::new(
                "submit",
                Stage::Submit {
                    forge: "github".into(),
                },
            ),
            checks,
            human,
            WorkflowStage::new("merge", Stage::Merge),
        ],
    }
}

fn check_golden(name: &str, workflow: &Workflow) {
    assert_eq!(workflow.validate(&roles()), Ok(()), "{name} must be valid");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("workflow-{name}.toml"));
    let encoded = toml::to_string(workflow).unwrap();
    if std::env::var_os("AGEND_BLESS_GOLDEN").is_some() {
        std::fs::write(&path, &encoded).unwrap();
    }
    let golden = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    assert_eq!(encoded, golden, "{name}: persisted TOML changed");
    let decoded: Workflow = toml::from_str(&golden).unwrap();
    assert_eq!(
        &decoded, workflow,
        "{name}: golden TOML does not round-trip"
    );
}

#[test]
fn built_in_workflows_have_a_stable_toml_format() {
    check_golden("code", &Workflow::builtin_code());
    check_golden("research", &Workflow::builtin_research());
    check_golden("epic", &Workflow::builtin_epic());
    check_golden("planned", &Workflow::builtin_planned());
}

#[test]
fn custom_workflow_with_every_optional_field_has_a_stable_toml_format() {
    check_golden("custom", &custom());
}
