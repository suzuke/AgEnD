//! A small real `Runner` (`sh -c`) whose knobs can each be set wrong. With
//! every knob right it meets the whole Runner contract, which shows the
//! process-level rules (RUN-4, RUN-8, RUN-9) can be met by a real runner.
//!
//! Signal safety: the child is spawned into a new process group of its own
//! (`process_group(0)`, so its pgid is its pid), and on timeout only that
//! group is signalled, before the child is reaped (the pid cannot have been
//! reused). Nothing else is ever signalled.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agend_core::traits::{CommandOutput, Runner};
use agend_testkit::contract::fakes::FakeRunnerFixture;
use agend_testkit::contract::runner::RunnerFixture;
use agend_testkit::fakes::FakeError;
use agend_testkit::tempdir::TempDir;

/// What a timeout stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kill {
    /// The child's whole process group: `sh` and everything it started.
    Group,
    /// Only `sh` (`Child::kill`); its children keep running.
    ShOnly,
}

/// When stdout and stderr are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drain {
    /// On reader threads while the command runs.
    WhileRunning,
    /// After the command exits (deadlocks once a pipe buffer fills).
    AfterExit,
}

pub struct ShRunner {
    /// Provides the fresh, canonical working directory.
    dir: FakeRunnerFixture,
    kill: Kill,
    drain: Drain,
    /// Runs every command here instead of the requested directory.
    wrong_directory: Option<TempDir>,
}

impl ShRunner {
    pub fn correct() -> Self {
        Self {
            dir: FakeRunnerFixture::new(),
            kill: Kill::Group,
            drain: Drain::WhileRunning,
            wrong_directory: None,
        }
    }

    pub fn kill(self, kill: Kill) -> Self {
        Self { kill, ..self }
    }

    pub fn drain(self, drain: Drain) -> Self {
        Self { drain, ..self }
    }

    pub fn in_its_own_directory(self) -> Self {
        let dir = TempDir::new("runner-wrong-dir").expect("create temp dir");
        Self {
            wrong_directory: Some(dir),
            ..self
        }
    }
}

fn error(message: impl std::fmt::Display) -> FakeError {
    FakeError {
        operation: "run",
        message: message.to_string(),
    }
}

fn reader(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}

/// Sends SIGKILL to the process group led by `child`, which this runner
/// spawned with `process_group(0)` and has not reaped yet.
fn kill_group(child: &Child) -> Result<(), FakeError> {
    let pgid = child.id();
    assert!(pgid > 1, "refusing to signal process group {pgid}");
    let status = Command::new("kill")
        .args(["-s", "KILL", "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(error)?;
    if status.success() {
        Ok(())
    } else {
        Err(error(format!("kill -KILL -{pgid}: {status}")))
    }
}

impl Runner for ShRunner {
    type Error = FakeError;

    async fn run(
        &self,
        command: &str,
        working_directory: &str,
        timeout_ms: u64,
    ) -> Result<CommandOutput, FakeError> {
        let directory = match &self.wrong_directory {
            Some(dir) => dir.path().to_path_buf(),
            None => PathBuf::from(working_directory),
        };
        let mut child = Command::new("sh")
            .args(["-c", command])
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(error)?;
        let mut readers = match self.drain {
            Drain::WhileRunning => Some((
                reader(child.stdout.take().expect("piped stdout")),
                reader(child.stderr.take().expect("piped stderr")),
            )),
            Drain::AfterExit => None,
        };
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if let Some(status) = child.try_wait().map_err(error)? {
                let (stdout, stderr) = match readers.take() {
                    Some((out, err)) => (out.join().unwrap(), err.join().unwrap()),
                    None => {
                        let (mut out, mut err) = (Vec::new(), Vec::new());
                        child
                            .stdout
                            .take()
                            .unwrap()
                            .read_to_end(&mut out)
                            .map_err(error)?;
                        child
                            .stderr
                            .take()
                            .unwrap()
                            .read_to_end(&mut err)
                            .map_err(error)?;
                        (out, err)
                    }
                };
                return Ok(CommandOutput {
                    exit_code: status.code(),
                    stdout,
                    stderr,
                    timed_out: false,
                });
            }
            if Instant::now() >= deadline {
                match self.kill {
                    Kill::Group => kill_group(&child)?,
                    Kill::ShOnly => child.kill().map_err(error)?,
                }
                child.wait().map_err(error)?;
                // After a group kill every writer is gone and the readers
                // see EOF. After killing only `sh`, an orphan may still hold
                // the pipes: leave its readers behind.
                if self.kill == Kill::Group
                    && let Some((out, err)) = readers.take()
                {
                    let _ = (out.join(), err.join());
                }
                return Ok(CommandOutput {
                    exit_code: None,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    timed_out: true,
                });
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl RunnerFixture for ShRunner {
    type Runner = Self;
    type Error = FakeError;
    fn runner(&self) -> &Self {
        self
    }
    fn working_directory(&self) -> &str {
        RunnerFixture::working_directory(&self.dir)
    }
}

#[test]
fn a_real_sh_runner_meets_the_runner_contract() {
    agend_testkit::contract::runner::run("sh", ShRunner::correct).assert_passed();
}
