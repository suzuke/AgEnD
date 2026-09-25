//! `Runner` contract (rules RUN-1..9 in CONTRACTS.md): exit codes come back
//! as they are; stdout and stderr come back separately and byte for byte,
//! however large (no pipe deadlock); a command that outlives its timeout
//! reports `timed_out` and no exit code, soon after the timeout (not when
//! the command would have finished), and the command is stopped together
//! with every process it started (it never gets to write [`LATE_MARKER`] or
//! [`CHILD_MARKER`]); commands run in the given working directory.
//!
//! The commands are POSIX `sh` snippets. A real runner executes them; a fake
//! fixture scripts [`COMMANDS`] (each entry says what the shell does).
//!
//! Not pinned: the exit code of a command killed by a signal; processes that
//! leave the command's process group on purpose (`setsid`); stdin.

use std::fmt::Debug;
use std::path::Path;
use std::time::{Duration, Instant};

use agend_core::traits::{CommandOutput, Runner};

use super::{Case, CaseResult, Report, ensure, ok, run_suite};
use crate::block_on;

/// Timeout for every case except the timeout cases.
pub const TIMEOUT_MS: u64 = 10_000;
/// Timeout for [`TIMES_OUT`] and [`STARTS_A_CHILD`]; they take far longer.
pub const SHORT_TIMEOUT_MS: u64 = 200;
/// A timed-out `run` must return within this many ms of starting: ten times
/// the timeout, but well before [`TIMES_OUT`] would finish on its own.
pub const TIMEOUT_REPORT_LIMIT_MS: u64 = 2_000;
/// File [`TIMES_OUT`] creates in the working directory if it is not stopped.
pub const LATE_MARKER: &str = "timed-out-command-finished";
/// File the child `sh` of [`STARTS_A_CHILD`] creates if it is not stopped.
pub const CHILD_MARKER: &str = "timed-out-child-finished";
/// How long after a timed-out command would have finished the case looks
/// for its marker.
pub const LATE_MARKER_GRACE_MS: u64 = 1_500;
/// Bytes [`LARGE_OUTPUT`] writes to stdout and again to stderr: far more
/// than a pipe buffer (64 KiB on Linux and macOS).
pub const LARGE_OUTPUT_BYTES: usize = 256 * 1024;
/// Timeout for [`LARGE_OUTPUT`]; it finishes in milliseconds when drained.
pub const LARGE_OUTPUT_TIMEOUT_MS: u64 = 5_000;
/// Prints the working directory; the expected stdout depends on the fixture.
pub const PWD: &str = "pwd";

/// A contract command and what `sh -c <command>` does.
#[derive(Debug, Clone, Copy)]
pub struct ContractCommand {
    pub command: &'static str,
    pub exit_code: i32,
    pub stdout: &'static [u8],
    pub stderr: &'static [u8],
    /// `stdout` and `stderr` are each written this many times.
    pub repeat: usize,
    pub duration_ms: u64,
}

impl ContractCommand {
    pub fn expected_stdout(&self) -> Vec<u8> {
        self.stdout.repeat(self.repeat)
    }

    pub fn expected_stderr(&self) -> Vec<u8> {
        self.stderr.repeat(self.repeat)
    }
}

pub const SUCCEEDS: ContractCommand = ContractCommand {
    command: "printf ok",
    exit_code: 0,
    stdout: b"ok",
    stderr: b"",
    repeat: 1,
    duration_ms: 0,
};

pub const FAILS: ContractCommand = ContractCommand {
    command: "exit 3",
    exit_code: 3,
    stdout: b"",
    stderr: b"",
    repeat: 1,
    duration_ms: 0,
};

pub const SEPARATES_STREAMS: ContractCommand = ContractCommand {
    command: "printf out; printf err >&2",
    exit_code: 0,
    stdout: b"out",
    stderr: b"err",
    repeat: 1,
    duration_ms: 0,
};

/// Leading and trailing whitespace, blank lines and a byte that is not
/// UTF-8: a runner that trims or decodes output loses them.
pub const KEEPS_BYTES: ContractCommand = ContractCommand {
    command: "printf ' \\377 out \\n\\n'; printf ' err \\n' >&2",
    exit_code: 0,
    stdout: b" \xff out \n\n",
    stderr: b" err \n",
    repeat: 1,
    duration_ms: 0,
};

/// [`LARGE_OUTPUT_BYTES`] of `x` on stdout, then as many `y` on stderr.
pub const LARGE_OUTPUT: ContractCommand = ContractCommand {
    command: "dd if=/dev/zero bs=1024 count=256 2>/dev/null | tr '\\000' x; \
              dd if=/dev/zero bs=1024 count=256 2>/dev/null | tr '\\000' y >&2",
    exit_code: 0,
    stdout: b"x",
    stderr: b"y",
    repeat: LARGE_OUTPUT_BYTES,
    duration_ms: 0,
};

/// Sleeps, then writes [`LATE_MARKER`]; `sh` exits 0 after about 3 s.
pub const TIMES_OUT: ContractCommand = ContractCommand {
    command: "sleep 3; touch timed-out-command-finished",
    exit_code: 0,
    stdout: b"",
    stderr: b"",
    repeat: 1,
    duration_ms: 3_000,
};

/// Starts a child `sh` that sleeps, then writes [`CHILD_MARKER`] (`; true`
/// keeps the outer shell from exec'ing it). Stopping only the outer `sh`
/// leaves the child running.
pub const STARTS_A_CHILD: ContractCommand = ContractCommand {
    command: "sh -c 'sleep 3; touch timed-out-child-finished'; true",
    exit_code: 0,
    stdout: b"",
    stderr: b"",
    repeat: 1,
    duration_ms: 3_000,
};

pub const COMMANDS: [ContractCommand; 7] = [
    SUCCEEDS,
    FAILS,
    SEPARATES_STREAMS,
    KEEPS_BYTES,
    LARGE_OUTPUT,
    TIMES_OUT,
    STARTS_A_CHILD,
];

pub trait RunnerFixture {
    type Runner: Runner<Error = Self::Error>;
    type Error: Send + Debug;

    fn runner(&self) -> &Self::Runner;

    /// An existing, fresh directory, canonical (symlinks resolved).
    fn working_directory(&self) -> &str;
}

pub fn cases<F: RunnerFixture>() -> Vec<Case<F>> {
    vec![
        Case {
            rule: "RUN-1",
            name: "exit_codes_are_reported",
            check: |fx| {
                expect(fx, &SUCCEEDS, TIMEOUT_MS)?;
                expect(fx, &FAILS, TIMEOUT_MS)
            },
        },
        Case {
            rule: "RUN-2",
            name: "stdout_and_stderr_stay_separate",
            check: |fx| expect(fx, &SEPARATES_STREAMS, TIMEOUT_MS),
        },
        Case {
            rule: "RUN-3",
            name: "output_is_kept_byte_for_byte",
            check: |fx| expect(fx, &KEEPS_BYTES, TIMEOUT_MS),
        },
        Case {
            rule: "RUN-4",
            name: "large_output_comes_back_whole",
            check: |fx| expect(fx, &LARGE_OUTPUT, LARGE_OUTPUT_TIMEOUT_MS),
        },
        Case {
            rule: "RUN-5",
            name: "timeout_reports_timed_out_without_exit_code",
            check: timeout_reports_timed_out_without_exit_code,
        },
        Case {
            rule: "RUN-6",
            name: "timeout_is_reported_promptly",
            check: timeout_is_reported_promptly,
        },
        Case {
            rule: "RUN-7",
            name: "timed_out_command_is_stopped",
            check: |fx| stopped_after_timeout(fx, &TIMES_OUT, LATE_MARKER),
        },
        Case {
            rule: "RUN-8",
            name: "timed_out_command_children_are_stopped",
            check: |fx| stopped_after_timeout(fx, &STARTS_A_CHILD, CHILD_MARKER),
        },
        Case {
            rule: "RUN-9",
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

/// Shows short output in full and long output by length and first bytes.
fn describe(output: &CommandOutput) -> String {
    let bytes = |b: &[u8]| {
        if b.len() <= 64 {
            format!("{:?}", String::from_utf8_lossy(b))
        } else {
            format!(
                "{} bytes starting {:?}",
                b.len(),
                String::from_utf8_lossy(&b[..16])
            )
        }
    };
    format!(
        "exit_code {:?}, timed_out {}, stdout {}, stderr {}",
        output.exit_code,
        output.timed_out,
        bytes(&output.stdout),
        bytes(&output.stderr)
    )
}

fn expect<F: RunnerFixture>(fx: &F, command: &ContractCommand, timeout_ms: u64) -> CaseResult {
    let output = run_command(fx, command.command, timeout_ms)?;
    let expected = CommandOutput {
        exit_code: Some(command.exit_code),
        stdout: command.expected_stdout(),
        stderr: command.expected_stderr(),
        timed_out: false,
    };
    ensure(output == expected, || {
        format!(
            "`{}`: expected {}, got {}",
            command.command,
            describe(&expected),
            describe(&output)
        )
    })
}

fn timeout_reports_timed_out_without_exit_code<F: RunnerFixture>(fx: &F) -> CaseResult {
    let output = run_command(fx, TIMES_OUT.command, SHORT_TIMEOUT_MS)?;
    ensure(output.timed_out && output.exit_code.is_none(), || {
        format!(
            "`{}` with a {SHORT_TIMEOUT_MS} ms timeout: expected timed_out and no exit code, got {}",
            TIMES_OUT.command,
            describe(&output)
        )
    })
}

fn timeout_is_reported_promptly<F: RunnerFixture>(fx: &F) -> CaseResult {
    let started = Instant::now();
    run_command(fx, TIMES_OUT.command, SHORT_TIMEOUT_MS)?;
    let elapsed = started.elapsed();
    ensure(
        elapsed < Duration::from_millis(TIMEOUT_REPORT_LIMIT_MS),
        || {
            format!(
                "`{}` with a {SHORT_TIMEOUT_MS} ms timeout: reported after {} ms, limit {TIMEOUT_REPORT_LIMIT_MS} ms",
                TIMES_OUT.command,
                elapsed.as_millis()
            )
        },
    )
}

/// Runs `command` into a timeout, waits until it would have finished, and
/// checks (read-only) that it never wrote `marker`.
fn stopped_after_timeout<F: RunnerFixture>(
    fx: &F,
    command: &ContractCommand,
    marker: &str,
) -> CaseResult {
    let started = Instant::now();
    let output = run_command(fx, command.command, SHORT_TIMEOUT_MS)?;
    ensure(output.timed_out, || {
        format!(
            "`{}` with a {SHORT_TIMEOUT_MS} ms timeout did not time out: {}",
            command.command,
            describe(&output)
        )
    })?;
    let finished = Duration::from_millis(command.duration_ms + LATE_MARKER_GRACE_MS);
    std::thread::sleep(finished.saturating_sub(started.elapsed()));
    let marker = Path::new(fx.working_directory()).join(marker);
    ensure(!marker.exists(), || {
        format!(
            "`{}` kept running after it timed out: {} exists",
            command.command,
            marker.display()
        )
    })
}

fn runs_in_the_working_directory<F: RunnerFixture>(fx: &F) -> CaseResult {
    let output = run_command(fx, PWD, TIMEOUT_MS)?;
    let expected = format!("{}\n", fx.working_directory());
    ensure(
        output.exit_code == Some(0) && output.stdout == expected.as_bytes(),
        || format!("`pwd`: expected {expected:?}, got {}", describe(&output)),
    )
}
