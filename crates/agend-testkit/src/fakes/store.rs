use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agend_core::pipeline::task::Task;
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{CasResult, Store, StoredEvent, VersionedTask};

use super::{Failures, FakeError, lock};

const OPERATIONS: &[&str] = &[
    "load_task",
    "create_task",
    "compare_and_swap_task",
    "load_workflow",
    "append_event",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreCall {
    LoadTask { task_id: String },
    CreateTask(Task),
    CompareAndSwapTask { task: Task, expected_version: u64 },
    LoadWorkflow { workflow_id: String, version: u64 },
    AppendEvent { task_id: String, event: StoredEvent },
}

/// In-memory store. Versions start at 1 and grow by 1 per successful
/// compare-and-swap. The data plays the part of the database file:
/// [`FakeStore::reopen`] is a new handle over the same data (a daemon
/// restart), while `calls()` and `fail_next` stay per handle. Beyond the
/// contract: appending an event to an unknown task fails.
#[derive(Debug)]
pub struct FakeStore {
    data: Arc<Mutex<Data>>,
    state: Mutex<State>,
}

/// What outlives a handle: everything a real store keeps on disk.
#[derive(Debug, Default)]
struct Data {
    tasks: BTreeMap<String, VersionedTask>,
    workflows: BTreeMap<(String, u64), Workflow>,
    events: BTreeMap<String, Vec<StoredEvent>>,
}

#[derive(Debug)]
struct State {
    calls: Vec<StoreCall>,
    failures: Failures,
}

impl FakeStore {
    pub const FIRST_VERSION: u64 = 1;

    pub fn new() -> Self {
        Self::over(Arc::default())
    }

    fn over(data: Arc<Mutex<Data>>) -> Self {
        Self {
            data,
            state: Mutex::new(State {
                calls: Vec::new(),
                failures: Failures::new(OPERATIONS),
            }),
        }
    }

    /// A new handle over the same data, as after a daemon restart. Its
    /// `calls()` start empty.
    pub fn reopen(&self) -> Self {
        Self::over(Arc::clone(&self.data))
    }

    /// Saves a workflow version (the trait has no write method for them).
    pub fn insert_workflow(&self, workflow: Workflow) {
        lock(&self.data)
            .workflows
            .insert((workflow.id.clone(), workflow.version), workflow);
    }

    pub fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        lock(&self.data)
            .events
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn fail_next(&self, operation: &str, message: &str) {
        lock(&self.state).failures.push(operation, message);
    }

    pub fn calls(&self) -> Vec<StoreCall> {
        lock(&self.state).calls.clone()
    }
}

impl Default for FakeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Store for FakeStore {
    type Error = FakeError;

    async fn load_task(&self, task_id: &str) -> Result<Option<VersionedTask>, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(StoreCall::LoadTask {
            task_id: task_id.to_owned(),
        });
        if let Some(error) = state.failures.take("load_task") {
            return Err(error);
        }
        Ok(lock(&self.data).tasks.get(task_id).cloned())
    }

    async fn create_task(&self, task: &Task) -> Result<(), FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(StoreCall::CreateTask(task.clone()));
        if let Some(error) = state.failures.take("create_task") {
            return Err(error);
        }
        let mut data = lock(&self.data);
        if data.tasks.contains_key(&task.id) {
            return Err(FakeError::new(
                "create_task",
                format!("task {} already exists", task.id),
            ));
        }
        data.tasks.insert(
            task.id.clone(),
            VersionedTask {
                version: Self::FIRST_VERSION,
                task: task.clone(),
            },
        );
        Ok(())
    }

    async fn compare_and_swap_task(
        &self,
        task: &Task,
        expected_version: u64,
    ) -> Result<CasResult, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(StoreCall::CompareAndSwapTask {
            task: task.clone(),
            expected_version,
        });
        if let Some(error) = state.failures.take("compare_and_swap_task") {
            return Err(error);
        }
        let mut data = lock(&self.data);
        let Some(current) = data.tasks.get_mut(&task.id) else {
            return Ok(CasResult::Conflict {
                current_version: None,
            });
        };
        if current.version != expected_version {
            return Ok(CasResult::Conflict {
                current_version: Some(current.version),
            });
        }
        current.version += 1;
        current.task = task.clone();
        Ok(CasResult::Written {
            new_version: current.version,
        })
    }

    async fn load_workflow(
        &self,
        workflow_id: &str,
        version: u64,
    ) -> Result<Option<Workflow>, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(StoreCall::LoadWorkflow {
            workflow_id: workflow_id.to_owned(),
            version,
        });
        if let Some(error) = state.failures.take("load_workflow") {
            return Err(error);
        }
        Ok(lock(&self.data)
            .workflows
            .get(&(workflow_id.to_owned(), version))
            .cloned())
    }

    async fn append_event(&self, task_id: &str, event: &StoredEvent) -> Result<(), FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(StoreCall::AppendEvent {
            task_id: task_id.to_owned(),
            event: event.clone(),
        });
        if let Some(error) = state.failures.take("append_event") {
            return Err(error);
        }
        let mut data = lock(&self.data);
        if !data.tasks.contains_key(task_id) {
            return Err(FakeError::new("append_event", format!("no task {task_id}")));
        }
        data.events
            .entry(task_id.to_owned())
            .or_default()
            .push(event.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;

    #[test]
    fn scripted_failure_leaves_the_task_unchanged() {
        let store = FakeStore::new();
        let task = Task::new("T-1", "fix", "general", "code", 1);
        block_on(store.create_task(&task)).unwrap();
        store.fail_next("compare_and_swap_task", "disk full");
        let mut changed = task.clone();
        changed.title = "fix it".into();
        let error = block_on(store.compare_and_swap_task(&changed, 1)).unwrap_err();
        assert_eq!(error.message, "disk full");
        let loaded = block_on(store.load_task("T-1")).unwrap().unwrap();
        assert_eq!((loaded.version, loaded.task), (1, task));
        assert!(block_on(store.append_event("T-9", &event())).is_err());
    }

    #[test]
    fn reopened_store_keeps_the_data_but_not_the_calls() {
        let store = FakeStore::new();
        let task = Task::new("T-1", "fix", "general", "code", 1);
        block_on(store.create_task(&task)).unwrap();
        block_on(store.append_event("T-1", &event())).unwrap();
        let reopened = store.reopen();
        drop(store);
        let loaded = block_on(reopened.load_task("T-1")).unwrap().unwrap();
        assert_eq!((loaded.version, loaded.task), (1, task));
        assert_eq!(reopened.events("T-1"), vec![event()]);
        assert_eq!(reopened.calls().len(), 1);
    }

    fn event() -> StoredEvent {
        StoredEvent {
            id: "e-1".into(),
            occurred_at_unix_ms: 1,
            kind: "created".into(),
            detail: String::new(),
        }
    }
}
