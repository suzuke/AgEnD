//! Backend recorder: drives a backend CLI through fixed scenarios over the
//! same transport its fake emulates, and records every message in both
//! directions as a JSONL transcript (`transcripts/<backend>/<scenario>.jsonl`).
//!
//! The same scenario code runs against the real CLI (`agend-record`, by hand,
//! under a write sandbox) and against the fake agent (`tests/conformance.rs`,
//! in CI). The conformance test compares the two by shape (`shape`), so a fake
//! cannot silently drift from the real CLI.
//!
//! Adding a backend: implement [`Backend`] in a new module and add it to
//! [`BACKENDS`] (see the README section on the recorder).
//!
//! Must NOT: run a real CLI outside a fresh `/private/tmp/agend-rec-*` project
//! directory, approve any command, or signal a process it did not spawn.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::fakes::lock;

pub mod claude;
pub mod codex;
pub mod opencode;
pub mod redact;
pub mod shape;

/// Transcript format version (the header's `format`).
pub const FORMAT: u64 = 1;

/// Where real recordings run: each scenario gets `mktemp -d <prefix>-<backend>-XXXX`.
pub const REAL_DIR_PREFIX: &str = "/private/tmp/agend-rec";

/// Every backend the recorder knows. Adding a backend = one entry here.
pub const BACKENDS: &[&dyn Backend] = &[&codex::Codex, &opencode::Opencode, &claude::Claude];

/// One recorded scenario. Prompts are tiny and harmless; approvals are
/// always denied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// One prompt, one reply.
    OneTurn,
    /// A long reply, interrupted with the CLI's own interrupt mechanism.
    Interrupt,
    /// The agent asks to run `echo agend-record`; the recorder denies it.
    Approval,
    /// A second message while a turn is running.
    Busy,
    /// Stop the CLI, start it again, continue the same session by id.
    Resume,
}

impl Scenario {
    pub const ALL: &[Scenario] = &[
        Scenario::OneTurn,
        Scenario::Interrupt,
        Scenario::Approval,
        Scenario::Busy,
        Scenario::Resume,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Scenario::OneTurn => "one_turn",
            Scenario::Interrupt => "interrupt",
            Scenario::Approval => "approval",
            Scenario::Busy => "busy",
            Scenario::Resume => "resume",
        }
    }

    pub fn parse(name: &str) -> Option<Scenario> {
        Scenario::ALL.iter().copied().find(|s| s.name() == name)
    }
}

/// Prompts shared by every backend. The approval prompt starts with `run: `,
/// the fakes' approval trigger (gate-02 A6).
pub mod prompts {
    pub const OK: &str = "Reply with exactly: OK";
    pub const LONG: &str =
        "Write the numbers from 1 to 400, separated by spaces, with no other text.";
    pub const APPROVAL_COMMAND: &str = "echo agend-record";
}

/// A backend the recorder can drive.
pub trait Backend: Sync {
    /// Short name: the command-line argument and the transcript directory.
    fn name(&self) -> &'static str;
    /// The real CLI program (looked up on `PATH`).
    fn program(&self) -> &'static str;
    /// The fake agent binary (`crate::fake_agent::*_BIN`).
    fn fake(&self) -> &'static str;
    /// Scenarios this backend supports (the others are not recorded).
    fn scenarios(&self) -> &'static [Scenario];
    /// Spawns `agent` with its working directory in `<dir>/project`, runs
    /// `scenario` over the backend's transport, records every message in
    /// `log`, and shuts the agent down.
    fn run(&self, scenario: Scenario, agent: &Agent, dir: &Path, log: &Log) -> Result<(), String>;
}

pub fn backend(name: &str) -> Option<&'static dyn Backend> {
    BACKENDS.iter().copied().find(|b| b.name() == name)
}

/// What to drive: the real CLI or its fake.
pub struct Agent {
    pub program: PathBuf,
    pub fake: bool,
    /// The `agend-record` binary (claude's hook and channel relays).
    pub relay: PathBuf,
    pub pace: Pace,
}

impl Agent {
    /// Environment only the fakes get: where they persist state across a
    /// restart ([`crate::fake_agent::STATE_DIR_ENV`]), inside the scenario
    /// directory. The real CLIs keep the user's own state directories.
    pub fn fake_env(&self, project: &Path) -> Vec<(String, PathBuf)> {
        if !self.fake {
            return Vec::new();
        }
        let data = project.parent().unwrap_or(project).join("fake-state");
        vec![(crate::fake_agent::STATE_DIR_ENV.to_owned(), data)]
    }

    /// Extra arguments only the fakes take.
    pub fn fake_args(&self) -> Vec<String> {
        if self.fake {
            vec!["--turn-ms".into(), self.pace.fake_turn_ms.to_string()]
        } else {
            Vec::new()
        }
    }
}

/// Timings. Real models are slow; fakes are fast and deterministic.
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    /// How long a turn runs before the scenario acts on it (interrupt, busy).
    pub settle: Duration,
    /// Longest wait for any expected message.
    pub timeout: Duration,
    /// How long nothing must happen before the scenario ends.
    pub quiet: Duration,
    /// `--turn-ms` for the fakes.
    pub fake_turn_ms: u64,
}

pub const REAL_PACE: Pace = Pace {
    settle: Duration::from_millis(2000),
    timeout: Duration::from_secs(120),
    quiet: Duration::from_secs(5),
    fake_turn_ms: 0,
};

pub const FAKE_PACE: Pace = Pace {
    settle: Duration::from_millis(150),
    timeout: Duration::from_secs(15),
    quiet: Duration::from_millis(400),
    fake_turn_ms: 900,
};

/// Who sent a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The recorder (in production: the AgEnD driver).
    Client,
    /// The backend CLI (or its fake).
    Backend,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Client => "client",
            Side::Backend => "backend",
        }
    }
}

/// One message. `via` names the channel: `ws` (codex), `http` and `sse`
/// (opencode), `key`, `hook`, `mcp` and `screen` (claude).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub from: Side,
    pub via: String,
    pub msg: Value,
}

impl Entry {
    pub fn to_json(&self) -> Value {
        json!({"from": self.from.name(), "via": self.via, "msg": self.msg})
    }

    pub fn from_json(value: &Value) -> Result<Entry, String> {
        let from = match value["from"].as_str() {
            Some("client") => Side::Client,
            Some("backend") => Side::Backend,
            other => return Err(format!("bad `from`: {other:?}")),
        };
        let via = value["via"].as_str().ok_or("missing `via`")?.to_owned();
        Ok(Entry {
            from,
            via,
            msg: value["msg"].clone(),
        })
    }

    /// `msg[key]` as a string.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.msg[key].as_str()
    }
}

/// The ordered record of one scenario; shared by the threads that talk to
/// the backend. Waiters are woken on every push.
#[derive(Clone, Default)]
pub struct Log {
    inner: Arc<(Mutex<Vec<Entry>>, Condvar)>,
}

impl Log {
    pub fn push(&self, from: Side, via: &str, msg: Value) -> usize {
        let (entries, changed) = &*self.inner;
        let mut entries = lock(entries);
        entries.push(Entry {
            from,
            via: via.to_owned(),
            msg,
        });
        changed.notify_all();
        entries.len() - 1
    }

    pub fn len(&self) -> usize {
        lock(&self.inner.0).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn entries(&self) -> Vec<Entry> {
        lock(&self.inner.0).clone()
    }

    /// Waits for the first entry at index `>= start` that matches; returns
    /// its index and a copy.
    pub fn wait(
        &self,
        start: usize,
        timeout: Duration,
        what: &str,
        matches: impl Fn(&Entry) -> bool,
    ) -> Result<(usize, Entry), String> {
        let deadline = Instant::now() + timeout;
        let (entries, changed) = &*self.inner;
        let mut guard = lock(entries);
        loop {
            if let Some(i) = (start..guard.len()).find(|&i| matches(&guard[i])) {
                return Ok((i, guard[i].clone()));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(format!("timed out after {timeout:?} waiting for {what}"));
            }
            guard = changed
                .wait_timeout(guard, deadline - now)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    /// Waits until no entry matching `matches` arrives for `quiet`.
    pub fn wait_quiet(&self, quiet: Duration, limit: Duration, matches: impl Fn(&Entry) -> bool) {
        let deadline = Instant::now() + limit;
        let mut seen = self.len();
        while Instant::now() < deadline {
            let start = seen;
            match self.wait(start, quiet, "quiet", &matches) {
                Ok((i, _)) => seen = i + 1,
                Err(_) => return,
            }
        }
    }
}

/// `<program> --version`, first line.
pub fn cli_version(program: &Path) -> Result<String, String> {
    let out = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{} --version: {e}", program.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().next().unwrap_or_default().trim().to_owned())
}

/// Today as `YYYY-MM-DD` (UTC), from the system clock.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Creates `<dir>/project` (the agent's working directory).
pub fn make_project(dir: &Path) -> Result<PathBuf, String> {
    let project = dir.join("project");
    std::fs::create_dir(&project).map_err(|e| format!("mkdir {}: {e}", project.display()))?;
    Ok(project)
}

/// A child process the recorder spawned. Dropping it stops it: close stdin,
/// then SIGTERM, then SIGKILL, only ever to this child's own pid.
pub struct Spawned {
    pub child: Child,
    pub name: String,
}

impl Spawned {
    pub fn stop(&mut self) {
        drop(self.child.stdin.take());
        if wait_exit(&mut self.child, Duration::from_millis(1500)) {
            return;
        }
        let pid = self.child.id();
        if pid > 1 {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
        }
        if !wait_exit(&mut self.child, Duration::from_secs(5)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl Drop for Spawned {
    fn drop(&mut self) {
        self.stop();
    }
}

fn wait_exit(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Reads `stdout` lines on a thread until one contains `marker`; returns the
/// line. Later lines are drained so the child never blocks on a full pipe.
pub fn wait_for_line(
    child: &mut Child,
    stdout: impl std::io::Read + Send + 'static,
    marker: &'static str,
    timeout: Duration,
) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut found = false;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if !found && line.contains(marker) {
                found = true;
                let _ = tx.send(line);
            }
        }
    });
    match rx.recv_timeout(timeout) {
        Ok(line) => Ok(line),
        Err(_) => match child.try_wait() {
            Ok(Some(status)) => {
                let stderr_tail = child
                    .stderr
                    .take()
                    .map(|s| {
                        BufReader::new(s)
                            .lines()
                            .map_while(Result::ok)
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                Err(format!(
                    "no line containing {marker:?}: process exited ({status}) before it printed one, stderr: {stderr_tail:?}"
                ))
            }
            _ => Err(format!("no line containing {marker:?} within {timeout:?}")),
        },
    }
}

/// The header line of a transcript.
pub fn header(backend: &dyn Backend, scenario: Scenario, version: &str, run: Value) -> Value {
    json!({
        "type": "header",
        "format": FORMAT,
        "backend": backend.name(),
        "cli": backend.program(),
        "version": version,
        "scenario": scenario.name(),
        "recorded": today(),
        "run": run,
    })
}

/// Writes a redacted transcript: the header, then one entry per line.
pub fn write_transcript(path: &Path, header: &Value, entries: &[Entry]) -> Result<(), String> {
    let mut text = String::new();
    text.push_str(&header.to_string());
    text.push('\n');
    for entry in entries {
        text.push_str(&entry.to_json().to_string());
        text.push('\n');
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut file =
        std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("write {}: {e}", path.display()))
}

/// Reads a transcript: header and entries.
pub fn read_transcript(path: &Path) -> Result<(Value, Vec<Entry>), String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut lines = text.lines();
    let header: Value = serde_json::from_str(lines.next().ok_or("empty transcript")?)
        .map_err(|e| format!("{}: header: {e}", path.display()))?;
    let entries = lines
        .enumerate()
        .map(|(i, line)| {
            let value: Value = serde_json::from_str(line)
                .map_err(|e| format!("{}:{}: {e}", path.display(), i + 2))?;
            Entry::from_json(&value).map_err(|e| format!("{}:{}: {e}", path.display(), i + 2))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((header, entries))
}

/// Records `scenarios` of the REAL `backend` CLI into `<out>/<backend>/`.
/// Each scenario runs in a fresh `mktemp -d /private/tmp/agend-rec-<backend>-XXXX`
/// that is removed afterwards. A failed scenario is still written (redacted)
/// as `<scenario>.failed.jsonl` so it can be inspected; it is never copied
/// into the repo.
pub fn record_real(
    backend: &dyn Backend,
    scenarios: &[Scenario],
    out: &Path,
    relay: &Path,
) -> Result<(), String> {
    let program = PathBuf::from(backend.program());
    let version = cli_version(&program)?;
    eprintln!("agend-record: {} {version}", backend.program());
    let agent = Agent {
        program,
        fake: false,
        relay: relay.to_path_buf(),
        pace: REAL_PACE,
    };
    let mut failures = Vec::new();
    for &scenario in scenarios {
        let dir = mktemp_dir(backend.name())?;
        eprintln!(
            "agend-record: {} {} in {}",
            backend.name(),
            scenario.name(),
            dir.display()
        );
        let log = Log::default();
        let result = backend.run(scenario, &agent, &dir, &log);
        let redactor = redact::Redactor::new(&dir);
        let header = redactor.value(&header(
            backend,
            scenario,
            &version,
            json!({"program": backend.program()}),
        ));
        let entries = redactor.entries(&log.entries());
        let name = match &result {
            Ok(()) => format!("{}.jsonl", scenario.name()),
            Err(_) => format!("{}.failed.jsonl", scenario.name()),
        };
        let path = out.join(backend.name()).join(name);
        let findings = redact::scan(&header, &entries);
        if findings.is_empty() {
            write_transcript(&path, &header, &entries)?;
            eprintln!(
                "agend-record: wrote {} ({} messages)",
                path.display(),
                entries.len()
            );
        } else {
            failures.push(format!(
                "{}: secret scan found {findings:?}; not written",
                scenario.name()
            ));
        }
        if let Err(e) = result {
            failures.push(format!("{}: {e}", scenario.name()));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// `mktemp -d /private/tmp/agend-rec-<backend>-XXXX`, checked to be where
/// it must be.
pub(crate) fn mktemp_dir(backend: &str) -> Result<PathBuf, String> {
    let template = format!("{REAL_DIR_PREFIX}-{backend}-XXXX");
    let out = Command::new("mktemp")
        .args(["-d", &template])
        .output()
        .map_err(|e| format!("mktemp: {e}"))?;
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let prefix = format!("{REAL_DIR_PREFIX}-{backend}-");
    if !out.status.success() || !path.is_absolute() || !path.starts_with("/private/tmp") {
        return Err(format!("mktemp gave {}", path.display()));
    }
    if !path.to_string_lossy().starts_with(&prefix) || !path.is_dir() {
        return Err(format!("mktemp gave unexpected {}", path.display()));
    }
    Ok(path)
}

/// Re-applies the current redaction rules to a written transcript (after a
/// rule is added) and runs the secret scan; rewrites the file in place.
pub fn redact_file(path: &Path) -> Result<(), String> {
    let (header, entries) = read_transcript(path)?;
    let redactor = redact::Redactor::new(Path::new("/nonexistent/agend-rec"));
    let header = redactor.value(&header);
    let entries = redactor.entries(&entries);
    let findings = redact::scan(&header, &entries);
    if !findings.is_empty() {
        return Err(format!(
            "{}: secret scan found {findings:?}",
            path.display()
        ));
    }
    write_transcript(path, &header, &entries)
}

/// Runs `scenario` against the FAKE agent in a fresh temp dir; returns the
/// redacted entries (for the conformance check).
pub fn run_fake(backend: &dyn Backend, scenario: Scenario) -> Result<Vec<Entry>, String> {
    let program = crate::fake_agent::locate(backend.fake())?;
    let relay = crate::fake_agent::locate(RELAY_BIN)?;
    let agent = Agent {
        program,
        fake: true,
        relay,
        pace: FAKE_PACE,
    };
    let dir = crate::tempdir::TempDir::new(&format!("rec-{}", backend.name()))
        .map_err(|e| format!("temp dir: {e}"))?;
    let root = std::fs::canonicalize(dir.path()).map_err(|e| e.to_string())?;
    let log = Log::default();
    backend.run(scenario, &agent, &root, &log)?;
    Ok(redact::Redactor::new(&root).entries(&log.entries()))
}

/// The recorder binary (also the claude hook and channel relays).
pub const RELAY_BIN: &str = "agend-record";
