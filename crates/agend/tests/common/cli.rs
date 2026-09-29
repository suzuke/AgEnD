//! Running the real `agend` binary as the CLI (gate 9): one command with a
//! given `AGEND_HOME` and `AGEND_INSTANCE`, its stdout, stderr and exit code.
//! Shared by `tests/cli.rs` and `examples/cli_demo.rs` (`#[path]`).
//!
//! Safety: the environment is cleared except `PATH`, `HOME`, `USER`,
//! `LANG`, `TMPDIR` and what the caller sets; stdin is `/dev/null`; a
//! command that overruns its limit is killed with `Child::kill` (our own
//! child).
#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Longest one command may run.
pub const RUN_WITHIN: Duration = Duration::from_secs(90);

/// One finished `agend` command.
#[derive(Debug, Clone)]
pub struct Run {
    /// `$ AGEND_INSTANCE=g9-a agend send g9-b "hi"` (how it was run).
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
    pub took: Duration,
}

impl Run {
    /// The command, its output and its exit code, for a report.
    pub fn shown(&self) -> Vec<String> {
        let mut out = vec![self.command.clone()];
        out.extend(self.stdout.lines().map(str::to_owned));
        out.extend(self.stderr.lines().map(|l| format!("stderr: {l}")));
        out.push(format!(
            "exit={} ({:.1} s)",
            self.code.map_or("?".into(), |c| c.to_string()),
            self.took.as_secs_f64()
        ));
        out
    }
}

fn quoted(arg: &str) -> String {
    if arg.is_empty() || arg.contains([' ', '"', '\'', '$', '*', '?']) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_owned()
    }
}

/// The `agend` binary and the home its commands use.
#[derive(Clone)]
pub struct Cli {
    pub bin: PathBuf,
    pub home: PathBuf,
}

impl Cli {
    pub fn new(bin: &Path, home: &Path) -> Cli {
        Cli {
            bin: bin.to_path_buf(),
            home: home.to_path_buf(),
        }
    }

    /// `agend <args>` with `AGEND_HOME` = this home.
    pub fn run(&self, caller: Option<&str>, args: &[&str]) -> Run {
        self.run_with(Some(&self.home.clone()), caller, args, &[])
    }

    /// With another `AGEND_HOME` (`None`: unset).
    pub fn run_home(&self, home: Option<&Path>, caller: Option<&str>, args: &[&str]) -> Run {
        self.run_with(home, caller, args, &[])
    }

    pub fn run_with(
        &self,
        home: Option<&Path>,
        caller: Option<&str>,
        args: &[&str],
        extra: &[(&str, &str)],
    ) -> Run {
        let child = self.spawn(home, caller, args, extra);
        let command = describe(caller, args);
        match child {
            Ok((child, started)) => finish(command, child, started, RUN_WITHIN),
            Err(e) => Run {
                command,
                stdout: String::new(),
                stderr: e,
                code: None,
                took: Duration::ZERO,
            },
        }
    }

    /// Starts `agend <args>` without waiting (the caller finishes it with
    /// [`finish`]).
    pub fn spawn(
        &self,
        home: Option<&Path>,
        caller: Option<&str>,
        args: &[&str],
        extra: &[(&str, &str)],
    ) -> Result<(Child, Instant), String> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in ["PATH", "HOME", "USER", "LANG", "TMPDIR"] {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        if let Some(home) = home {
            cmd.env("AGEND_HOME", home);
        }
        if let Some(caller) = caller {
            cmd.env("AGEND_INSTANCE", caller);
        }
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let started = Instant::now();
        cmd.spawn()
            .map(|child| (child, started))
            .map_err(|e| format!("cannot run {}: {e}", self.bin.display()))
    }
}

pub fn describe(caller: Option<&str>, args: &[&str]) -> String {
    let who = caller.map_or(String::new(), |c| format!("AGEND_INSTANCE={c} "));
    let args: Vec<String> = args.iter().map(|a| quoted(a)).collect();
    format!("$ {who}agend {}", args.join(" "))
}

/// Waits for `child` (at most `limit`; then `Child::kill`) and collects it.
pub fn finish(command: String, mut child: Child, started: Instant, limit: Duration) -> Run {
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let deadline = started + limit;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    Run {
        command,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
        code,
        took: started.elapsed(),
    }
}
