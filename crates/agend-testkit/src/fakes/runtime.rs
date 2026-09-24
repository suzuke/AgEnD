use std::collections::BTreeMap;
use std::sync::Mutex;

use agend_core::traits::{HolderHandle, HolderLaunch, Runtime};

use super::{Failures, FakeError, lock};

const OPERATIONS: &[&str] = &["start_holder", "stop_holder", "recover_holders"];

/// First fake process id; each started holder gets the next one.
pub const FIRST_FAKE_PID: u32 = 40_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCall {
    Start(HolderLaunch),
    Stop { instance_id: String },
    Recover,
}

/// Starts no processes: keeps a table of "running" holders. Beyond the
/// contract: starting an instance that is already running and stopping one
/// that is not both fail, so a test notices a double start or stray stop.
#[derive(Debug)]
pub struct FakeRuntime {
    socket_dir: String,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    running: BTreeMap<String, HolderHandle>,
    next_pid: u32,
    calls: Vec<RuntimeCall>,
    failures: Failures,
}

impl FakeRuntime {
    pub fn new() -> Self {
        Self::with_socket_dir("/fake/agend/run/holders")
    }

    /// Holder sockets are reported as `<socket_dir>/<instance_id>.sock`.
    pub fn with_socket_dir(socket_dir: &str) -> Self {
        Self {
            socket_dir: socket_dir.trim_end_matches('/').to_owned(),
            state: Mutex::new(State {
                running: BTreeMap::new(),
                next_pid: FIRST_FAKE_PID,
                calls: Vec::new(),
                failures: Failures::new(OPERATIONS),
            }),
        }
    }

    /// Simulates a holder dying on its own (it disappears from recovery).
    pub fn crash(&self, instance_id: &str) -> Option<HolderHandle> {
        lock(&self.state).running.remove(instance_id)
    }

    /// Simulates a holder that outlived a daemon restart.
    pub fn adopt(&self, handle: HolderHandle) {
        lock(&self.state)
            .running
            .insert(handle.instance_id.clone(), handle);
    }

    pub fn running(&self) -> Vec<HolderHandle> {
        lock(&self.state).running.values().cloned().collect()
    }

    pub fn fail_next(&self, operation: &str, message: &str) {
        lock(&self.state).failures.push(operation, message);
    }

    pub fn calls(&self) -> Vec<RuntimeCall> {
        lock(&self.state).calls.clone()
    }
}

impl Default for FakeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime for FakeRuntime {
    type Error = FakeError;

    async fn start_holder(&self, launch: &HolderLaunch) -> Result<HolderHandle, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(RuntimeCall::Start(launch.clone()));
        if let Some(error) = state.failures.take("start_holder") {
            return Err(error);
        }
        if state.running.contains_key(&launch.instance_id) {
            return Err(FakeError::new(
                "start_holder",
                format!("holder already running for {}", launch.instance_id),
            ));
        }
        let handle = HolderHandle {
            instance_id: launch.instance_id.clone(),
            process_id: Some(state.next_pid),
            socket_path: format!("{}/{}.sock", self.socket_dir, launch.instance_id),
        };
        state.next_pid += 1;
        state
            .running
            .insert(launch.instance_id.clone(), handle.clone());
        Ok(handle)
    }

    async fn stop_holder(&self, instance_id: &str) -> Result<(), FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(RuntimeCall::Stop {
            instance_id: instance_id.to_owned(),
        });
        if let Some(error) = state.failures.take("stop_holder") {
            return Err(error);
        }
        match state.running.remove(instance_id) {
            Some(_) => Ok(()),
            None => Err(FakeError::new(
                "stop_holder",
                format!("no holder running for {instance_id}"),
            )),
        }
    }

    async fn recover_holders(&self) -> Result<Vec<HolderHandle>, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(RuntimeCall::Recover);
        if let Some(error) = state.failures.take("recover_holders") {
            return Err(error);
        }
        Ok(state.running.values().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;
    use agend_core::model::Backend;

    fn launch(id: &str) -> HolderLaunch {
        HolderLaunch {
            instance_id: id.into(),
            backend: Backend::Codex,
            executable: "codex".into(),
            args: vec![],
            working_directory: "/w".into(),
        }
    }

    #[test]
    fn crashed_holders_are_not_recovered_and_adopted_ones_are() {
        let runtime = FakeRuntime::new();
        let a = block_on(runtime.start_holder(&launch("a"))).unwrap();
        assert_eq!(a.process_id, Some(FIRST_FAKE_PID));
        assert_eq!(a.socket_path, "/fake/agend/run/holders/a.sock");
        block_on(runtime.start_holder(&launch("b"))).unwrap();
        runtime.crash("a");
        runtime.adopt(HolderHandle {
            instance_id: "c".into(),
            process_id: Some(7),
            socket_path: "/old/c.sock".into(),
        });
        let ids: Vec<String> = block_on(runtime.recover_holders())
            .unwrap()
            .into_iter()
            .map(|h| h.instance_id)
            .collect();
        assert_eq!(ids, vec!["b", "c"]);
    }

    #[test]
    fn double_start_and_stray_stop_fail() {
        let runtime = FakeRuntime::new();
        block_on(runtime.start_holder(&launch("a"))).unwrap();
        assert!(block_on(runtime.start_holder(&launch("a"))).is_err());
        assert!(block_on(runtime.stop_holder("zzz")).is_err());
        assert_eq!(runtime.calls().len(), 3);
    }
}
