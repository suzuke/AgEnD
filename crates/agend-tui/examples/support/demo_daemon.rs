//! The demo fleet on the testkit fake daemon, for the demo and the tests:
//! the scripted demo's catalog as the fleet view (tasks, instances), its
//! terminal screens, and its events and asks. Screens then read it through
//! `ClientSource`, the product's path (gate 11 B P1).
//!
//! Shared by `examples/*.rs` and `tests/*.rs` with `#[path]`.

#![allow(dead_code)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agend_core::protocol::client::{
    AgentState as WireState, AttentionRequiredData, DaemonEvent, InstanceView, TaskView,
};
use agend_testkit::fake_daemon::FakeDaemon;
use agend_tui::source::scripted::{DemoStep, demo_catalog, demo_script, demo_terminals};
use agend_tui::source::{AgentInfo, AgentState, TaskInfo};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A catalog task as the daemon would list it (stage ids only; no repo).
pub fn task_view(task: &TaskInfo) -> TaskView {
    TaskView {
        task_id: task.id.clone(),
        title: task.title.clone(),
        team_id: task.team_id.clone(),
        status: if task.is_done() { "done" } else { "running" }.into(),
        assignee: task.holder.clone(),
        stages: task.stages.iter().map(|s| s.name.clone()).collect(),
        current_stage: task.current_stage().map(|i| task.stages[i].name.clone()),
    }
}

/// A catalog agent as the daemon would list it ("needs you" is not a
/// protocol state: it comes from the needs-you list).
pub fn instance_view(agent: &AgentInfo) -> InstanceView {
    InstanceView {
        instance_id: agent.id.clone(),
        team_id: agent.team_id.clone(),
        backend: agent.backend.as_str().into(),
        state: match agent.state {
            AgentState::Starting => WireState::Starting,
            AgentState::Working | AgentState::NeedsYou => WireState::Working,
            AgentState::Idle => WireState::Idle,
            AgentState::Stuck => WireState::Stuck,
            AgentState::Failed => WireState::Failed,
            AgentState::Unknown => WireState::Unknown,
        },
        working_directory: None,
    }
}

/// Seeds `daemon` with the demo. Needs-you items get increasing waiting
/// times, so the order matches the scripted demo's.
pub fn seed(daemon: &FakeDaemon) {
    let catalog = demo_catalog();
    for task in &catalog.tasks {
        daemon.set_task(task_view(task));
    }
    for agent in &catalog.agents {
        daemon.set_instance(instance_view(agent));
    }
    for (agent, screen) in demo_terminals() {
        daemon.set_screen(agent, &screen);
    }
    for step in demo_script() {
        match step {
            DemoStep::Event(DaemonEvent::AttentionRequired { data }) => {
                let task = data.task_id.clone().unwrap_or_default();
                daemon.add_attention(AttentionRequiredData {
                    attention_id: Some(format!("usage-limit:{task}")),
                    waiting_since_unix_ms: Some(now_ms()),
                    unblocks: Some(0),
                    if_ignored: catalog.if_ignored.get(&task).cloned(),
                    ..data
                });
            }
            DemoStep::Event(event) => {
                daemon.emit(event);
            }
            DemoStep::Ask(thread, recap) => {
                daemon.open_ask(thread, recap);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
