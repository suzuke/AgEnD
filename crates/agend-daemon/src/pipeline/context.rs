//! Stable, readable work products for reviewers and the next writer.
use agend_core::pipeline::state::{PipelineState, WorkProduct};

pub(super) fn work_product(state: &PipelineState) -> String {
    match state.work_product() {
        Some(WorkProduct::Result { summary, output }) => {
            let mut text = format!("Result:\n{summary}");
            if let Some(output) = output {
                text.push_str(&format!("\nOutput:\n{output}"));
            }
            text
        }
        Some(WorkProduct::Plan { items }) => format!("Plan:\n{}", items.join("\n")),
        Some(WorkProduct::Branch { branch, head, .. }) => {
            format!(
                "Branch: {branch}\nHead: {}",
                state.current_head().unwrap_or(head)
            )
        }
        None => String::new(),
    }
}
