use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use agend_core::traits::{CommandOutput, Runner};

use super::{Failures, FakeError, lock};

const OPERATIONS: &[&str] = &["run"];

/// Exit code of an unscripted command, like `sh` for "command not found".
pub const NOT_SCRIPTED_EXIT_CODE: i32 = 127;

/// What a scripted command does. `duration_ms` is compared with the call's
/// `timeout_ms`: a longer command times out (no exit code, no output), which
/// is the shape the `Runner` contract requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptedCommand {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration_ms: u64,
}

impl ScriptedCommand {
    pub fn exits(exit_code: i32) -> Self {
        Self {
            exit_code,
            stdout: Vec::new(),
            stderr: Vec::new(),
            duration_ms: 0,
        }
    }

    pub fn stdout(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdout = bytes.into();
        self
    }

    pub fn stderr(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stderr = bytes.into();
        self
    }

    pub fn takes_ms(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerCall {
    pub command: String,
    pub working_directory: String,
    pub timeout_ms: u64,
}

/// Runs nothing: answers each command string from a script. The responses
/// for one command are used in order and the last one repeats. An
/// unscripted command exits with 127 and says so on stderr.
#[derive(Debug)]
pub struct FakeRunner {
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    script: BTreeMap<String, VecDeque<ScriptedCommand>>,
    calls: Vec<RunnerCall>,
    failures: Failures,
}

impl FakeRunner {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                script: BTreeMap::new(),
                calls: Vec::new(),
                failures: Failures::new(OPERATIONS),
            }),
        }
    }

    /// Appends a response for `command`.
    pub fn on(&self, command: &str, response: ScriptedCommand) {
        lock(&self.state)
            .script
            .entry(command.to_owned())
            .or_default()
            .push_back(response);
    }

    pub fn fail_next(&self, operation: &str, message: &str) {
        lock(&self.state).failures.push(operation, message);
    }

    pub fn calls(&self) -> Vec<RunnerCall> {
        lock(&self.state).calls.clone()
    }
}

impl Default for FakeRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl Runner for FakeRunner {
    type Error = FakeError;

    async fn run(
        &self,
        command: &str,
        working_directory: &str,
        timeout_ms: u64,
    ) -> Result<CommandOutput, FakeError> {
        let mut state = lock(&self.state);
        state.calls.push(RunnerCall {
            command: command.to_owned(),
            working_directory: working_directory.to_owned(),
            timeout_ms,
        });
        if let Some(error) = state.failures.take("run") {
            return Err(error);
        }
        let response = match state.script.get_mut(command) {
            Some(queue) if queue.len() > 1 => queue.pop_front(),
            Some(queue) => queue.front().cloned(),
            None => None,
        };
        let Some(response) = response else {
            return Ok(CommandOutput {
                exit_code: Some(NOT_SCRIPTED_EXIT_CODE),
                stdout: Vec::new(),
                stderr: format!("fake-runner: command not scripted: {command}\n").into_bytes(),
                timed_out: false,
            });
        };
        if response.duration_ms > timeout_ms {
            return Ok(CommandOutput {
                exit_code: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
                timed_out: true,
            });
        }
        Ok(CommandOutput {
            exit_code: Some(response.exit_code),
            stdout: response.stdout,
            stderr: response.stderr,
            timed_out: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_on;

    #[test]
    fn responses_are_used_in_order_and_the_last_repeats() {
        let runner = FakeRunner::new();
        runner.on("cargo test", ScriptedCommand::exits(101).stderr("1 failed"));
        runner.on("cargo test", ScriptedCommand::exits(0).stdout("ok"));
        let codes: Vec<Option<i32>> = (0..3)
            .map(|_| {
                block_on(runner.run("cargo test", "/w", 1_000))
                    .unwrap()
                    .exit_code
            })
            .collect();
        assert_eq!(codes, vec![Some(101), Some(0), Some(0)]);
        assert_eq!(runner.calls()[0].working_directory, "/w");
    }

    #[test]
    fn slow_commands_time_out_and_unscripted_ones_exit_127() {
        let runner = FakeRunner::new();
        runner.on("sleep 5", ScriptedCommand::exits(0).takes_ms(5_000));
        let slow = block_on(runner.run("sleep 5", "/w", 100)).unwrap();
        assert!(slow.timed_out);
        assert_eq!(slow.exit_code, None);
        let unknown = block_on(runner.run("make", "/w", 100)).unwrap();
        assert_eq!(unknown.exit_code, Some(NOT_SCRIPTED_EXIT_CODE));
        assert!(String::from_utf8_lossy(&unknown.stderr).contains("not scripted: make"));
    }
}
