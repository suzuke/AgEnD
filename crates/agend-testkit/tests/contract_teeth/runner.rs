//! Runner mutants: output-shape mutants wrap the scripted `FakeRunner`;
//! process-level mutants are the real `ShRunner` with one knob wrong.

use std::process::{Child, Command};
use std::sync::Mutex;

use agend_core::traits::{CommandOutput, Runner};
use agend_testkit::block_on;
use agend_testkit::contract::fakes::FakeRunnerFixture;
use agend_testkit::contract::runner::{self, RunnerFixture};
use agend_testkit::fakes::FakeError;

use super::Mutant;
use super::real_runner::{Drain, Kill, ShRunner};

type Run = fn(&M, &str, &str, u64) -> Result<CommandOutput, FakeError>;

/// The scripted fake runner behind a broken `run`.
pub struct M {
    fx: FakeRunnerFixture,
    /// Real `sh` processes a mutant left running; waited for (never
    /// signalled) on drop.
    children: Mutex<Vec<Child>>,
    run: Run,
}

impl M {
    fn new(run: Run) -> Self {
        Self {
            fx: FakeRunnerFixture::new(),
            children: Mutex::new(Vec::new()),
            run,
        }
    }

    fn real(&self, c: &str, wd: &str, timeout_ms: u64) -> Result<CommandOutput, FakeError> {
        block_on(self.fx.runner.run(c, wd, timeout_ms))
    }

    /// Runs `real` and then changes its output with `f`.
    fn map(
        &self,
        c: &str,
        wd: &str,
        timeout_ms: u64,
        f: fn(&mut CommandOutput),
    ) -> Result<CommandOutput, FakeError> {
        let mut output = self.real(c, wd, timeout_ms)?;
        f(&mut output);
        Ok(output)
    }
}

impl Drop for M {
    fn drop(&mut self) {
        for mut child in self.children.lock().unwrap().drain(..) {
            let _ = child.wait();
        }
    }
}

impl Runner for M {
    type Error = FakeError;
    async fn run(&self, c: &str, wd: &str, timeout_ms: u64) -> Result<CommandOutput, FakeError> {
        (self.run)(self, c, wd, timeout_ms)
    }
}

impl RunnerFixture for M {
    type Runner = Self;
    type Error = FakeError;
    fn runner(&self) -> &Self {
        self
    }
    fn working_directory(&self) -> &str {
        RunnerFixture::working_directory(&self.fx)
    }
}

pub fn mutants() -> Vec<Mutant> {
    vec![
        // RUN-1: every failure becomes exit code 1 (a success flag).
        Mutant {
            rule: "RUN-1",
            name: "NonZeroExitBecomesOne",
            run: |name| {
                runner::run(name, || {
                    M::new(|m, c, wd, t| {
                        m.map(c, wd, t, |o| {
                            if o.exit_code.is_some_and(|code| code != 0) {
                                o.exit_code = Some(1);
                            }
                        })
                    })
                })
            },
        },
        // RUN-2: stderr appended to stdout (2>&1).
        Mutant {
            rule: "RUN-2",
            name: "MergesStderrIntoStdout",
            run: |name| {
                runner::run(name, || {
                    M::new(|m, c, wd, t| {
                        m.map(c, wd, t, |o| {
                            let err = std::mem::take(&mut o.stderr);
                            o.stdout.extend(err);
                        })
                    })
                })
            },
        },
        // RUN-3 (verifier r2 RN1): stdout trimmed.
        Mutant {
            rule: "RUN-3",
            name: "TrimsStdout",
            run: |name| {
                runner::run(name, || {
                    M::new(|m, c, wd, t| {
                        m.map(c, wd, t, |o| o.stdout = o.stdout.trim_ascii().to_vec())
                    })
                })
            },
        },
        // RUN-3: output decoded as UTF-8, lossily.
        Mutant {
            rule: "RUN-3",
            name: "DecodesOutputLossily",
            run: |name| {
                runner::run(name, || {
                    M::new(|m, c, wd, t| {
                        m.map(c, wd, t, |o| {
                            o.stdout = String::from_utf8_lossy(&o.stdout).into_owned().into_bytes();
                        })
                    })
                })
            },
        },
        // RUN-4: output capped at one pipe buffer (64 KiB).
        Mutant {
            rule: "RUN-4",
            name: "CapsOutputAt64KiB",
            run: |name| {
                runner::run(name, || {
                    M::new(|m, c, wd, t| {
                        m.map(c, wd, t, |o| {
                            o.stdout.truncate(64 * 1024);
                            o.stderr.truncate(64 * 1024);
                        })
                    })
                })
            },
        },
        // RUN-4 (verifier r2 RN3): a real runner that reads the pipes only
        // after exit; a full pipe blocks the command until the timeout.
        Mutant {
            rule: "RUN-4",
            name: "RealDrainsAfterExit",
            run: |name| runner::run(name, || ShRunner::correct().drain(Drain::AfterExit)),
        },
        // RUN-5: a timeout reported as exit code 124 (what `timeout(1)` does).
        Mutant {
            rule: "RUN-5",
            name: "TimeoutExitCode124",
            run: |name| {
                runner::run(name, || {
                    M::new(|m, c, wd, t| {
                        m.map(c, wd, t, |o| {
                            if o.timed_out {
                                o.exit_code = Some(124);
                            }
                        })
                    })
                })
            },
        },
        // RUN-6: the timeout noticed only after the command would have ended.
        Mutant {
            rule: "RUN-6",
            name: "TimesOutLate",
            run: |name| {
                runner::run(name, || {
                    M::new(|m, c, wd, t| {
                        let output = m.real(c, wd, t)?;
                        if output.timed_out {
                            std::thread::sleep(std::time::Duration::from_millis(
                                runner::TIMES_OUT.duration_ms,
                            ));
                        }
                        Ok(output)
                    })
                })
            },
        },
        // RUN-7: reports the timeout on time but never stops the command (it
        // really runs it with `sh` and leaves it running).
        Mutant {
            rule: "RUN-7",
            name: "LeavesCommandRunning",
            run: |name| {
                runner::run(name, || {
                    M::new(|m, c, wd, t| {
                        let output = m.real(c, wd, t)?;
                        if output.timed_out {
                            let child = Command::new("sh")
                                .args(["-c", c])
                                .current_dir(wd)
                                .spawn()
                                .expect("spawn sh");
                            m.children.lock().unwrap().push(child);
                        }
                        Ok(output)
                    })
                })
            },
        },
        // RUN-8 (verifier r2 RN2): a real runner that kills only `sh`; the
        // processes it started keep running.
        Mutant {
            rule: "RUN-8",
            name: "RealKillsShOnly",
            run: |name| runner::run(name, || ShRunner::correct().kill(Kill::ShOnly)),
        },
        // RUN-9: a real runner that runs every command in a directory of
        // its own.
        Mutant {
            rule: "RUN-9",
            name: "RealIgnoresWorkingDirectory",
            run: |name| runner::run(name, || ShRunner::correct().in_its_own_directory()),
        },
    ]
}
