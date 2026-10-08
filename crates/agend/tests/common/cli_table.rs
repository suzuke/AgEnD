//! The CLI-n table (gate 9 P10, `crates/agend/TESTING.md`): every row is one
//! `agend` command with its input and the expected exit code and output.
//! Rows the real daemon serves run twice, against the testkit fake daemon
//! (`FakeDaemon::start_at($AGEND_HOME/run/daemon.sock)`) and against a real
//! `agend daemon`; rows whose handler arrives in gate 10 run against the
//! fake only, and the real daemon's `not_supported` for them is its own
//! rows; rows that need no daemon run once. Shared by `tests/cli.rs` and
//! `examples/cli_demo.rs` (`#[path]`); needs `crate::lab` and `crate::cli`.
//!
//! Safety: see `cli_process.rs` (homes in the lab, only our own children).
#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::time::Duration;

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::client::{AgentState, InstanceView, ResultIdentity, V1_3, V1_8};
use agend_testkit::fake_daemon::FakeDaemon;

use crate::cli::{Cli, Run};
use crate::lab::{Daemon, Lab};

pub const A: &str = "g9-a";
pub const B: &str = "g9-b";

/// Who runs the command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Operator,
    /// `AGEND_INSTANCE=g9-a`
    A,
    /// `AGEND_INSTANCE=g9-b`
    B,
}

/// Which `AGEND_HOME` the command gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Home {
    Set,
    Unset,
    Relative,
    /// A home with v1's `fleet.yaml`.
    V1,
}

/// Where the row runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum On {
    /// No daemon is needed; once.
    Nothing,
    /// The fake and the real daemon, same expectation.
    Both,
    /// The fake only; the real handler arrives in the named gate.
    Fake(&'static str),
    /// The real daemon only (what it answers before gate 10).
    Real,
}

pub struct Expect {
    pub code: i32,
    pub stdout: &'static [&'static str],
    pub stderr: &'static [&'static str],
    /// `--json`: exactly one JSON value on stdout, nothing on stderr.
    pub json: bool,
}

/// Prepares the fake daemon before a row.
pub type Setup = fn(&FakeDaemon);

pub struct Row {
    pub id: &'static str,
    pub who: Who,
    pub home: Home,
    pub args: &'static [&'static str],
    pub on: On,
    pub expect: Expect,
    /// Prepares the fake daemon before the row (an assignment).
    pub setup: Option<Setup>,
}

const fn ok(stdout: &'static [&'static str]) -> Expect {
    Expect {
        code: 0,
        stdout,
        stderr: &[],
        json: false,
    }
}

const fn fails(code: i32, stderr: &'static [&'static str]) -> Expect {
    Expect {
        code,
        stdout: &[],
        stderr,
        json: false,
    }
}

const fn json(code: i32, stdout: &'static [&'static str]) -> Expect {
    Expect {
        code,
        stdout,
        stderr: &[],
        json: true,
    }
}

const fn row(
    id: &'static str,
    who: Who,
    args: &'static [&'static str],
    on: On,
    expect: Expect,
) -> Row {
    Row {
        id,
        who,
        home: Home::Set,
        args,
        on,
        expect,
        setup: None,
    }
}

fn assign(task: &str, stage: &str, attempt: u32) -> impl Fn(&FakeDaemon) {
    let (task, stage) = (task.to_owned(), stage.to_owned());
    move |fake| {
        fake.assign(
            &task,
            ResultIdentity {
                stage_id: stage.clone(),
                attempt,
            },
        )
    }
}

pub const GATE_10: &str = "gate 10";

/// The table, in the order it runs (rows build on the ones before them).
pub fn rows() -> Vec<Row> {
    use On::*;
    use Who::*;
    let mut rows = vec![
        row(
            "CLI-1",
            Operator,
            &["--version"],
            Nothing,
            ok(&["agend 0.0.0"]),
        ),
        Row {
            home: Home::Unset,
            ..row(
                "CLI-2",
                Operator,
                &["status"],
                Nothing,
                fails(
                    2,
                    &[
                        "agend: AGEND_HOME is not set and HOME is not an absolute path; run: export AGEND_HOME=<absolute path>",
                    ],
                ),
            )
        },
        Row {
            home: Home::Relative,
            ..row(
                "CLI-3",
                Operator,
                &["status"],
                Nothing,
                fails(
                    2,
                    &["agend: AGEND_HOME must be an absolute path, got g9-relative"],
                ),
            )
        },
        Row {
            home: Home::V1,
            ..row(
                "CLI-4",
                Operator,
                &["status"],
                Nothing,
                fails(
                    1,
                    &[
                        "looks like an AgEnD v1 home (fleet.yaml); set AGEND_HOME to another directory",
                    ],
                ),
            )
        },
        row(
            "CLI-5",
            Operator,
            &["send", "g9-b"],
            Nothing,
            fails(
                2,
                &[
                    "agend: the following required arguments were not provided",
                    "example: agend send dev-2",
                ],
            ),
        ),
        row(
            "CLI-6",
            Operator,
            &["send", "g9-b", "--json"],
            Nothing,
            json(
                2,
                &[
                    r#"{"error":{"code":"usage","message":"the following required arguments were not provided: <MESSAGE>"}}"#,
                ],
            ),
        ),
        row(
            "CLI-7",
            Operator,
            &["--help"],
            Nothing,
            ok(&["Examples:\n  agend status"]),
        ),
        row(
            "CLI-8",
            Operator,
            &["status"],
            Both,
            ok(&[
                "daemon: pid ",
                "client protocol 1.3",
                "instances: 2 (g9-a ",
                "needs you: 0",
            ]),
        ),
        row(
            "CLI-9",
            A,
            &["status"],
            Both,
            ok(&["g9-a (claude): no task\nnext: agend inbox | agend send <name> \"<message>\""]),
        ),
        row(
            "CLI-10",
            A,
            &["instance", "add", "x", "claude"],
            Both,
            fails(
                1,
                &["agend: forbidden: only the operator can add instances; ask the operator\n"],
            ),
        ),
        row(
            "CLI-11",
            Operator,
            &["done", "t-1/work/1"],
            Both,
            fails(
                1,
                &[
                    "agend: forbidden: agend done is an agent command; it runs inside an agent, where AGEND_INSTANCE is set\n",
                ],
            ),
        ),
        row(
            "CLI-12",
            Operator,
            &["done", "t-1/work/1", "--json"],
            Both,
            json(
                1,
                &[
                    r#"{"error":{"code":"forbidden","message":"agend done is an agent command; it runs inside an agent, where AGEND_INSTANCE is set"}}"#,
                ],
            ),
        ),
        row(
            "CLI-13",
            A,
            &["send", "g9-b", "hi b"],
            Both,
            ok(&["accepted: message ", " to g9-b (queue)"]),
        ),
        row(
            "CLI-14",
            A,
            &["send", "g9-b", "urgent", "--level", "steer"],
            Both,
            ok(&["accepted: message ", " to g9-b (steer)"]),
        ),
        row(
            "CLI-15",
            B,
            &["inbox"],
            Both,
            ok(&[" from g9-a: hi b\n", " from g9-a: urgent\n"]),
        ),
        row(
            "CLI-16",
            B,
            &["inbox", "--after", "00000000-0000-4000-8000-000000000000"],
            Both,
            fails(
                1,
                &[
                    "agend: unknown_message: you have no message 00000000-0000-4000-8000-000000000000 (unknown, older than 30 days, or not yours); run agend inbox without --after",
                ],
            ),
        ),
        row(
            "CLI-17",
            A,
            &["send", "nobody", "hi"],
            Both,
            fails(1, &["agend: unknown_instance: no instance nobody"]),
        ),
        row(
            "CLI-18",
            Operator,
            &[
                "instance",
                "add",
                "g9-new",
                "claude",
                "--program",
                "/bin/sh",
                "--",
                "-c",
                "sleep 600",
            ],
            Both,
            ok(&[
                "added g9-new (claude, session ",
                "/workspace/g9-new); starting",
            ]),
        ),
        row(
            "CLI-19",
            Operator,
            &["instance", "list"],
            Both,
            ok(&[
                "NAME    BACKEND  STATE",
                "  DIR\n",
                "g9-new  claude   ",
                "/workspace/g9-new\n",
            ]),
        ),
        row(
            "CLI-20",
            Operator,
            &["instance", "add", "g9-new", "claude"],
            Both,
            fails(1, &["agend: instance_exists: name g9-new is already used"]),
        ),
        row(
            "CLI-21",
            Operator,
            &["instance", "add", "Bad_Name", "claude"],
            Both,
            fails(
                1,
                &[
                    "agend: invalid_request: invalid name \"Bad_Name\": use 1-24 characters from a-z 0-9 - (e.g. dev-1)",
                ],
            ),
        ),
        row(
            "CLI-22",
            Operator,
            &["instance", "remove", "g9-new"],
            Nothing,
            fails(
                2,
                &[
                    "agend: agend instance remove needs --yes when not on a terminal\nexample: agend instance remove g9-new --yes",
                ],
            ),
        ),
        row(
            "CLI-23",
            Operator,
            &["instance", "remove", "g9-new", "--yes"],
            Both,
            ok(&["removed g9-new; workspace kept at ", "/workspace/g9-new\n"]),
        ),
        row(
            "CLI-24",
            Operator,
            &["instance", "remove", "g9-new", "--yes"],
            Both,
            fails(
                1,
                &["agend: unknown_instance: no instance g9-new; see agend instance list"],
            ),
        ),
        row(
            "CLI-25",
            Operator,
            &["task", "cancel", "t-1"],
            Both,
            fails(1, &["agend: invalid_request: unknown task t-1"]),
        ),
        row(
            "CLI-26",
            Operator,
            &["task", "create", "--role", "dev", "login page"],
            Nothing,
            fails(
                2,
                &["agend: the operator's agend task create needs --team <team>"],
            ),
        ),
        row(
            "CLI-27",
            Operator,
            &[
                "task",
                "create",
                "--role",
                "dev",
                "login page",
                "--team",
                "web",
            ],
            Nothing,
            fails(1, &["cannot reach the AgEnD daemon"]),
        ),
        row(
            "CLI-28",
            A,
            &["done", "t-42"],
            Nothing,
            fails(
                2,
                &[
                    "agend: invalid ticket \"t-42\": use <task>/<stage>/<attempt> from agend status, e.g. t-42/review/2",
                ],
            ),
        ),
        row(
            "CLI-29",
            A,
            &["remind", "1d"],
            Nothing,
            fails(2, &["agend: invalid delay \"1d\": use 90s, 30m or 2h"]),
        ),
        row(
            "CLI-30",
            Operator,
            &["daemon", "restart", "--binary", "/usr/bin/false"],
            Both,
            fails(
                1,
                &[
                    "agend: preflight_failed: /usr/bin/false daemon preflight exited with status 1; the daemon keeps running agend 0.0.0",
                ],
            ),
        ),
        row(
            "CLI-31",
            Operator,
            &["daemon", "restart"],
            Both,
            ok(&[
                "preflight agend 0.0.0 (",
                "  db copy: ",
                "restarting the daemon (pid ",
                "the daemon is back: pid ",
                ", client protocol 1.3, instances=2 (",
            ]),
        ),
        row(
            "CLI-32",
            B,
            &["inbox"],
            Both,
            ok(&[" from g9-a: hi b\n", " from g9-a: urgent\n"]),
        ),
        row(
            "CLI-33",
            A,
            &["done", "t-1/work/1"],
            Real,
            fails(1, &["agend: invalid_request: unknown task t-1"]),
        ),
        row(
            "CLI-34",
            A,
            &["block", "waiting for the API key"],
            Real,
            fails(1, &["agend: invalid_request: unknown task"]),
        ),
        row(
            "CLI-35",
            A,
            &["ask", "which database?"],
            Real,
            ok(&["asked A-"]),
        ),
    ];
    let fake_only: Vec<(Row, Option<Setup>)> = vec![
        (
            row(
                "CLI-36",
                A,
                &["status"],
                Fake(GATE_10),
                ok(&["t-42 · review (attempt 2) · ticket t-42/review/2"]),
            ),
            Some(|f| assign("t-42", "review", 2)(f)),
        ),
        (
            row(
                "CLI-37",
                A,
                &["review", "approve", "t-42/review/2"],
                Fake(GATE_10),
                ok(&["accepted"]),
            ),
            None,
        ),
        (
            row(
                "CLI-38",
                A,
                &["review", "approve", "t-42/review/2"],
                Fake(GATE_10),
                fails(1, &["agend: stale_result: "]),
            ),
            None,
        ),
        (
            row(
                "CLI-39",
                A,
                &["done", "t-43/work/1"],
                Fake(GATE_10),
                ok(&["accepted"]),
            ),
            Some(|f| assign("t-43", "work", 1)(f)),
        ),
        (
            row(
                "CLI-40",
                A,
                &["result", "t-44/plan/1", "split in two"],
                Fake(GATE_10),
                ok(&["accepted"]),
            ),
            Some(|f| assign("t-44", "plan", 1)(f)),
        ),
        (
            row(
                "CLI-41",
                A,
                &["review", "changes", "t-45/review/1", "rename the flag"],
                Fake(GATE_10),
                ok(&["accepted"]),
            ),
            Some(|f| assign("t-45", "review", 1)(f)),
        ),
        (
            row(
                "CLI-42",
                A,
                &[
                    "ask",
                    "which database?",
                    "--option",
                    "sqlite",
                    "--option",
                    "postgres",
                ],
                Fake(GATE_10),
                ok(&["asked A-"]),
            ),
            None,
        ),
        (
            row(
                "CLI-43",
                A,
                &["block", "waiting for the API key"],
                Fake(GATE_10),
                ok(&["accepted"]),
            ),
            Some(|f| assign("t-46", "work", 1)(f)),
        ),
        (
            row("CLI-44", A, &["unblock"], Fake(GATE_10), ok(&["accepted"])),
            None,
        ),
        (
            row(
                "CLI-45",
                A,
                &["remind", "30m"],
                Fake(GATE_10),
                ok(&["accepted"]),
            ),
            None,
        ),
        (
            row(
                "CLI-46",
                A,
                &["task", "create", "--role", "dev", "login page"],
                Fake(GATE_10),
                ok(&["created T-"]),
            ),
            None,
        ),
    ];
    for (mut r, setup) in fake_only {
        r.setup = setup;
        rows.push(r);
    }
    rows
}

/// What one run of a row gave.
pub struct Verdict {
    pub run: Run,
    pub problem: Option<String>,
}

pub fn check(expect: &Expect, run: &Run) -> Option<String> {
    check_version(expect, run, V1_3)
}

fn check_version(expect: &Expect, run: &Run, selected: ProtocolVersion) -> Option<String> {
    let mut problems = Vec::new();
    if run.code != Some(expect.code) {
        problems.push(format!("exit {:?}, expected {}", run.code, expect.code));
    }
    for needle in expect.stdout {
        // Status/restart must report the actual negotiated capability.
        let needle = needle.replace(
            "client protocol 1.3",
            &format!("client protocol {}.{}", selected.major, selected.minor),
        );
        if !run.stdout.contains(&needle) {
            problems.push(format!("stdout lacks {needle:?}"));
        }
    }
    for needle in expect.stderr {
        if !run.stderr.contains(needle) {
            problems.push(format!("stderr lacks {needle:?}"));
        }
    }
    if expect.json {
        let lines: Vec<&str> = run.stdout.lines().collect();
        let one = lines.len() == 1 && serde_json::from_str::<serde_json::Value>(lines[0]).is_ok();
        if !one || !run.stderr.is_empty() {
            problems.push("--json must print exactly one JSON value and nothing on stderr".into());
        }
    } else if expect.code == 0 && !run.stderr.is_empty() {
        problems.push("a success printed on stderr".into());
    } else if expect.code != 0 && !run.stdout.is_empty() {
        problems.push("a failure printed on stdout".into());
    }
    (!problems.is_empty()).then(|| format!("{}\n{}", problems.join("; "), run.shown().join("\n")))
}

fn caller(who: Who) -> Option<&'static str> {
    match who {
        Who::Operator => None,
        Who::A => Some(A),
        Who::B => Some(B),
    }
}

fn run_row(cli: &Cli, row: &Row, selected: ProtocolVersion) -> Verdict {
    let mut run = match row.home {
        Home::Set => cli.run(caller(row.who), row.args),
        Home::Unset => cli.run_home(None, caller(row.who), row.args),
        Home::Relative => cli.run_home(Some(Path::new("g9-relative")), caller(row.who), row.args),
        Home::V1 => {
            let v1 = cli.home.join("v1");
            let _ = fs::create_dir_all(&v1);
            let _ = fs::write(v1.join("fleet.yaml"), "instances: {}\n");
            cli.run_home(Some(&v1), caller(row.who), row.args)
        }
    };
    let home = match row.home {
        Home::Set => "",
        Home::Unset => "(AGEND_HOME unset) ",
        Home::Relative => "AGEND_HOME=g9-relative ",
        Home::V1 => "AGEND_HOME=<a home with fleet.yaml> ",
    };
    run.command = run.command.replacen("$ ", &format!("$ {home}"), 1);
    let problem = check_version(&row.expect, &run, selected);
    Verdict { run, problem }
}

/// Every row against its targets; `(row id, target, verdict)` in order.
pub fn run_table(
    lab: &Lab,
    bin: &Path,
) -> Result<Vec<(&'static str, &'static str, Verdict)>, String> {
    let rows = rows();
    let mut out = Vec::new();
    // No daemon.
    let none = Cli::new(bin, &lab.home(90));
    for r in rows.iter().filter(|r| r.on == On::Nothing) {
        out.push((r.id, "once", run_row(&none, r, V1_3)));
    }
    // The fake daemon at $AGEND_HOME/run/daemon.sock.
    let home = lab.home(91);
    fs::create_dir_all(home.join("run")).map_err(|e| e.to_string())?;
    let fake = FakeDaemon::start_at(&home.join("run/daemon.sock")).map_err(|e| e.to_string())?;
    for id in [A, B] {
        fake.set_instance(InstanceView {
            instance_id: id.into(),
            team_id: "general".into(),
            backend: "claude".into(),
            state: AgentState::Unknown,
            working_directory: Some(home.join("workspace").join(id).display().to_string()),
        });
    }
    let cli = Cli::new(bin, &home);
    for r in rows
        .iter()
        .filter(|r| matches!(r.on, On::Both | On::Fake(_)))
    {
        if let Some(setup) = r.setup {
            setup(&fake);
        }
        out.push((r.id, "fake", run_row(&cli, r, V1_3)));
    }
    drop(fake);
    // The real daemon.
    let home = lab.home(92);
    let mut daemon = Daemon::start(lab, &home, &[])?;
    daemon.ready()?;
    let cli = Cli::new(bin, &home);
    for id in [A, B] {
        let added = cli.run(
            None,
            &[
                "instance",
                "add",
                id,
                "claude",
                "--program",
                "/bin/sh",
                "--",
                "-c",
                "sleep 600",
            ],
        );
        if added.code != Some(0) {
            return Err(format!("setup: {}", added.shown().join("\n")));
        }
    }
    for r in rows.iter().filter(|r| matches!(r.on, On::Both | On::Real)) {
        out.push((r.id, "real", run_row(&cli, r, V1_8)));
    }
    daemon.interrupt()?;
    for (_, target, verdict) in &mut out {
        if *target == "real"
            && let Some(problem) = &mut verdict.problem
        {
            problem.push_str("\ndaemon log:\n");
            problem.push_str(&daemon.log.join("\n"));
        }
    }
    Ok(out)
}

/// The demo's lines: each row's command, output and exit code once, then
/// `fake ok` / `real ok` / `ok` per target that ran it.
pub fn lines(results: &[(&'static str, &'static str, Verdict)]) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut failed = 0;
    let rows = rows();
    for r in &rows {
        let runs: Vec<&(&str, &str, Verdict)> =
            results.iter().filter(|(id, _, _)| *id == r.id).collect();
        // The real daemon's output when it ran the row.
        let shown = runs
            .iter()
            .find(|(_, target, _)| *target == "real")
            .or(runs.first());
        let Some((_, _, first)) = shown else {
            continue;
        };
        lines.push(format!("{}  {}", r.id, first.run.command));
        for line in first.run.stdout.lines() {
            lines.push(format!("        {line}"));
        }
        for line in first.run.stderr.lines() {
            lines.push(format!("        stderr: {line}"));
        }
        let verdicts: Vec<String> = runs
            .iter()
            .map(|(_, target, v)| match &v.problem {
                None if *target == "once" => "ok".to_owned(),
                None => format!("{target} ok"),
                Some(p) => {
                    failed += 1;
                    format!("{target} FAIL: {p}")
                }
            })
            .collect();
        let note = match r.on {
            On::Fake(gate) => format!(" (fake only; the real daemon's handler arrives in {gate})"),
            _ => String::new(),
        };
        lines.push(format!(
            "        exit {} · {}{note}",
            first.run.code.map_or("?".into(), |c| c.to_string()),
            verdicts.join(" · ")
        ));
    }
    (lines, failed)
}

/// Longest a row may run (the restart row waits for the daemon).
pub const ROW_WITHIN: Duration = Duration::from_secs(90);
