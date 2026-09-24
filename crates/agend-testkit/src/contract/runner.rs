//! `Runner` contract: exit codes, stdout and stderr come back separately and
//! unchanged; a command that outlives its timeout reports `timed_out` and no
//! exit code; commands run in the given working directory.
//!
//! The commands are POSIX `sh` snippets. A real runner executes them; a fake
//! fixture scripts [`COMMANDS`] (each entry says what the shell does).

use std::fmt::Debug;

use agend_core::traits::{CommandOutput, Runner};

use super::{Case, CaseResult, Report, ensure, ok, run_suite};
use crate::block_on;

/// Timeout for every case except the timeout case.
pub const TIMEOUT_MS: u64 = 10_000;
/// Timeout for [`TIMES_OUT`]; its command takes far longer.
pub const SHORT_TIMEOUT_MS: u64 = 200;
/// Prints the working directory; the expected stdout depends on the fixture.
pub const PWD: &str = "pwd";

/// A contract command and what `sh -c <command>` does.
#[derive(Debug, Clone, Copy)]
pub struct ContractCommand {
    pub command: &'static str,
    pub exit_code: i32,
    pub stdout: &'static [u8],
    pub stderr: &'static [u8],
    pub duration_ms: u64,
}

pub const SUCCEEDS: ContractCommand = ContractCommand {
    command: "printf ok",
    exit_code: 0,
    stdout: b"ok",
    stderr: b"",
    duration_ms: 0,
};

pub const FAILS: ContractCommand = ContractCommand {
    command: "exit 3",
    exit_code: 3,
    stdout: b"",
    stderr: b"",
    duration_ms: 0,
};

pub const SEPARATES_STREAMS: ContractCommand = ContractCommand {
    command: "printf out; printf err >&2",
    exit_code: 0,
    stdout: b"out",
    stderr: b"err",
    duration_ms: 0,
};

pub const TIMES_OUT: ContractCommand = ContractCommand {
    command: "sleep 30",
    exit_code: 0,
    stdout: b"",
    stderr: b"",
    duration_ms: 30_000,
};

pub const COMMANDS: [ContractCommand; 4] = [SUCCEEDS, FAILS, SEPARATES_STREAMS, TIMES_OUT];

pub trait RunnerFixture {
    type Runner: Runner<Error = Self::Error>;
    type Error: Send + Debug;

    fn runner(&self) -> &Self::Runner;

    /// An existing directory, canonical (symlinks resolved).
    fn working_directory(&self) -> &str;
}

pub fn cases<F: RunnerFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            name: "success_reports_exit_code_zero_and_stdout",
            check: |fx| expect(fx, &SUCCEEDS),
        },
        Case {
            name: "failure_reports_its_exit_code",
            check: |fx| expect(fx, &FAILS),
        },
        Case {
            name: "stdout_and_stderr_stay_separate",
            check: |fx| expect(fx, &SEPARATES_STREAMS),
        },
        Case {
            name: "timeout_reports_timed_out_without_exit_code",
            check: timeout_reports_timed_out_without_exit_code,
        },
        Case {
            name: "runs_in_the_working_directory",
            check: runs_in_the_working_directory,
        },
    ]
}

pub fn run<F: RunnerFixture>(implementation: &str, make: impl FnMut() -> F) -> Report {
    run_suite("Runner", implementation, &cases::<F>(), make)
}

fn run_command<F: RunnerFixture>(
    fx: &F,
    command: &str,
    timeout_ms: u64,
) -> Result<CommandOutput, String> {
    ok(
        "run",
        block_on(fx.runner().run(command, fx.working_directory(), timeout_ms)),
    )
}

fn expect<F: RunnerFixture>(fx: &F, command: &ContractCommand) -> CaseResult {
    let output = run_command(fx, command.command, TIMEOUT_MS)?;
    let expected = CommandOutput {
        exit_code: Some(command.exit_code),
        stdout: command.stdout.to_vec(),
        stderr: command.stderr.to_vec(),
        timed_out: false,
    };
    ensure(output == expected, || {
        format!(
            "`{}`: expected {expected:?}, got {output:?}",
            command.command
        )
    })
}

fn timeout_reports_timed_out_without_exit_code<F: RunnerFixture>(fx: &F) -> CaseResult {
    let output = run_command(fx, TIMES_OUT.command, SHORT_TIMEOUT_MS)?;
    ensure(output.timed_out && output.exit_code.is_none(), || {
        format!(
            "`{}` with a {SHORT_TIMEOUT_MS} ms timeout: expected timed_out and no exit code, got {output:?}",
            TIMES_OUT.command
        )
    })
}

fn runs_in_the_working_directory<F: RunnerFixture>(fx: &F) -> CaseResult {
    let output = run_command(fx, PWD, TIMEOUT_MS)?;
    let printed = String::from_utf8_lossy(&output.stdout);
    ensure(
        output.exit_code == Some(0) && printed.trim_end() == fx.working_directory(),
        || {
            format!(
                "`pwd`: expected {:?}, got {output:?}",
                fx.working_directory()
            )
        },
    )
}
