//! The gate 9 CLI against a real `agend daemon` and the testkit fake
//! daemon: resends after a restart (P5), `agend daemon restart` and its
//! preflight (P7), the inherited holders after an `exec`, and the milestone
//! (two fake codex agents messaging each other across a restart, P1, P5).
//! Shared by `tests/cli.rs` and `examples/cli_demo.rs` (`#[path]`); needs
//! `crate::lab` (gate 6), `crate::codex` (gate 7) and `crate::cli`.
//!
//! Safety: homes under `/tmp/g9-<pid>-<n>` (the lab's); every daemon is a
//! child of this process (SIGINT / `Child::kill`); holders are stopped with
//! `agend instance remove` or the lab's `Shutdown`; the `agend` commands are
//! our children (`Child::kill` only after their limit); the fake codex runs
//! only in our own holders. Nothing here signals another process.
//!
//! Must NOT: run the real codex, claude or opencode.
#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use agend_core::model::DeliveryState;
use agend_core::protocol::client::{
    AgentState, ClientRequest, ClientResponse, CommandResult, DAEMON_SOCKET, InstanceView,
    OperatorCommand, OperatorData,
};
use agend_daemon::driver::codex::launch;
use agend_daemon::runtime::files;
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;
use agend_testkit::contract::client::proxy::{Direction, Options, Proxy, Transform};
use agend_testkit::fake_agent::codex::Probe;
use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};
use serde_json::{Value, json};

use crate::cli::{Cli, Run, describe, finish};
use crate::lab::{Daemon, Lab};

fn ensure(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what()) }
}

fn expect_run(run: &Run, code: i32, needles: &[&str]) -> Result<(), String> {
    let all = format!("{}{}", run.stdout, run.stderr);
    ensure(
        run.code == Some(code) && needles.iter().all(|n| all.contains(n)),
        || {
            format!(
                "expected exit {code} with {needles:?}:\n{}",
                run.shown().join("\n")
            )
        },
    )
}

/// Waits until `check` holds (every 50 ms, at most `within`).
fn wait_for(within: Duration, what: &str, mut check: impl FnMut() -> bool) -> Result<(), String> {
    let deadline = Instant::now() + within;
    while !check() {
        if Instant::now() >= deadline {
            return Err(format!("{what}: not within {within:?}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// A real daemon on `home`, ready.
pub fn start(lab: &Lab, home: &Path) -> Result<Daemon, String> {
    let mut daemon = Daemon::start(lab, home, &[])?;
    daemon.ready()?;
    Ok(daemon)
}

/// `agend instance add <id> claude --program /bin/sh -- -c "sleep 600"`
/// and the pid of its holder once it runs.
pub fn add_sleeper(cli: &Cli, id: &str) -> Result<u32, String> {
    let run = cli.run(
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
    expect_run(&run, 0, &["added "])?;
    let mut pid = None;
    wait_for(Duration::from_secs(10), &format!("holder of {id}"), || {
        pid = files::running(&cli.home, id).ok().flatten();
        pid.is_some()
    })?;
    Ok(pid.expect("waited"))
}

/// The daemon's `hello` reply: `(pid, boot id)`.
pub fn hello(home: &Path) -> Result<(Option<u32>, Option<u64>), String> {
    let mut c = ProbeClient::connect(&home.join(DAEMON_SOCKET)).map_err(|e| e.to_string())?;
    match c
        .request(&ClientRequest::hello())
        .map_err(|e| e.to_string())?
    {
        ClientResponse::Hello { data } => Ok((data.daemon_pid, data.boot_id)),
        other => Err(format!("hello answered {other:?}")),
    }
}

/// `/tmp/agend-pf-*` directories now.
pub fn preflight_homes() -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir("/tmp")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("agend-pf-"))
        .collect();
    found.sort();
    found
}

/// A script in the lab (0755).
pub fn script(lab: &Lab, name: &str, body: &str) -> Result<PathBuf, String> {
    let path = lab.root.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).map_err(|e| e.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Whether `pid` is a zombie or gone: `ps -o stat= -p <pid>`.
fn state_of(pid: u32) -> String {
    std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// `== restart` (P7): the same pid and the same holder after `agend daemon
/// restart`; no preflight home is left.
pub fn restart(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(1);
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    let holder = add_sleeper(&cli, "g9-r")?;
    let (_, boot) = hello(&home)?;
    let run = cli.run(None, &["daemon", "restart"]);
    let back = format!("the daemon is back: pid {}, ", daemon.pid);
    expect_run(
        &run,
        0,
        &["  holder: hello ok, spawn ok, shutdown ok", &back],
    )?;
    let recovered = daemon.expect("recovered=1")?;
    let (pid, boot_after) = hello(&home)?;
    ensure(pid == Some(daemon.pid) && boot_after != boot, || {
        format!(
            "after the restart: pid {pid:?} boot {boot_after:?}; before: pid {} boot {boot:?}",
            daemon.pid
        )
    })?;
    let now = files::running(&home, "g9-r").map_err(|e| e.to_string())?;
    ensure(now == Some(holder), || {
        format!("holder of g9-r was {holder}, now {now:?}")
    })?;
    // Its own preflight home is gone (other tests may run theirs now).
    let line = daemon.expect("preflight home /tmp/agend-pf-")?;
    let pf = line.rsplit(' ').next().unwrap_or_default().to_owned();
    ensure(!Path::new(&pf).exists(), || format!("{pf} is left"))?;
    let status = cli.run(None, &["status"]);
    expect_run(&status, 0, &["instances: 1 (g9-r "])?;
    let mut out = run.shown();
    out.push(format!(
        "same daemon pid {}, new boot id; holder of g9-r still pid {holder}; daemon log: {}",
        daemon.pid,
        crate::lab::untimed(&recovered)
    ));
    out.push(format!("its preflight home {pf} is gone"));
    daemon.interrupt()?;
    Ok(out)
}

/// `== preflight` (P7): a binary that fails the preflight leaves the daemon
/// and `agend.db` exactly as they were.
pub fn preflight_failures(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(2);
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    add_sleeper(&cli, "g9-p")?;
    // Let the start settle: nothing writes to agend.db after this.
    std::thread::sleep(Duration::from_millis(500));
    let db = home.join("agend.db");
    let before = fs::read(&db).map_err(|e| e.to_string())?;
    let (pid, boot) = hello(&home)?;
    let broken = script(
        lab,
        "broken-agend",
        "echo 'db copy: broken on purpose' >&2\nexit 3",
    )?;
    let broken = broken.display().to_string();
    let cases: [(&str, String); 4] = [
        (
            "/usr/bin/false",
            "exited with status 1; the daemon keeps running agend 0.0.0".into(),
        ),
        (
            &broken,
            "exited with status 3: db copy: broken on purpose".into(),
        ),
        (
            "/usr/bin/true",
            "exited 0 without reporting its steps (is it agend?)".into(),
        ),
        ("/nonexistent/agend", "cannot run /nonexistent/agend".into()),
    ];
    let mut out = Vec::new();
    for (binary, said) in &cases {
        let run = cli.run(None, &["daemon", "restart", "--binary", binary]);
        expect_run(&run, 1, &["agend: preflight_failed: ", binary, said])?;
        out.extend(run.shown());
    }
    let after = fs::read(&db).map_err(|e| e.to_string())?;
    ensure(after == before, || "agend.db changed".into())?;
    ensure(hello(&home)? == (pid, boot), || {
        "the daemon was replaced".into()
    })?;
    ensure(!daemon.log.iter().any(|l| l.contains(" exec ")), || {
        "the daemon tried to exec".into()
    })?;
    out.push(format!(
        "agend.db byte-identical ({} bytes); same pid {} and boot id; no exec",
        before.len(),
        daemon.pid
    ));
    daemon.interrupt()?;
    Ok(out)
}

/// `== one restart at a time` (P7).
pub fn one_restart_at_a_time(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(3);
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    let slow = script(lab, "slow-agend", "sleep 3\nexit 1")?;
    let slow = slow.display().to_string();
    let args = ["daemon", "restart", "--binary", slow.as_str()];
    let (first, started) = cli.spawn(Some(&home), None, &args, &[])?;
    daemon.expect(&format!("restart requested: preflight of {slow}"))?;
    let second = cli.run(None, &args);
    expect_run(
        &second,
        1,
        &["agend: invalid_request: a restart is already in progress"],
    )?;
    let first = finish(
        describe(None, &args),
        first,
        started,
        Duration::from_secs(30),
    );
    expect_run(
        &first,
        1,
        &["agend: preflight_failed: ", "exited with status 1"],
    )?;
    let third = cli.run(None, &["daemon", "restart"]);
    expect_run(&third, 0, &["the daemon is back"])?;
    let mut out = first.shown();
    out.extend(second.shown());
    out.push("after the first failed: agend daemon restart → the daemon is back".into());
    daemon.interrupt()?;
    Ok(out)
}

/// The fake daemon behind a proxy at `$AGEND_HOME/run/daemon.sock`.
struct Proxied {
    fake: FakeDaemon,
    proxy: Option<Proxy>,
    home: PathBuf,
}

impl Proxied {
    fn new(home: &Path) -> Result<Proxied, String> {
        fs::create_dir_all(home.join("run")).map_err(|e| e.to_string())?;
        let fake = FakeDaemon::start_at(&home.join("fake.sock")).map_err(|e| e.to_string())?;
        for id in ["g9-a", "g9-b"] {
            fake.set_instance(InstanceView {
                instance_id: id.into(),
                team_id: "general".into(),
                backend: "claude".into(),
                state: AgentState::Unknown,
                working_directory: None,
            });
        }
        Ok(Proxied {
            fake,
            proxy: None,
            home: home.to_path_buf(),
        })
    }

    fn proxy(&mut self, options: Options) -> Result<(), String> {
        self.proxy = None;
        let proxy = Proxy::start_at(
            &self.home.join(DAEMON_SOCKET),
            self.home.join("fake.sock"),
            options,
        )
        .map_err(|e| e.to_string())?;
        self.proxy = Some(proxy);
        Ok(())
    }

    /// Restarts the fake itself (`daemon_restart` without a binary): every
    /// connection closes, the state stays.
    fn restart_fake(&self) -> Result<(), String> {
        let (mut c, _) =
            ProbeClient::hello(&self.home.join("fake.sock"), None).map_err(|e| e.to_string())?;
        let request = ClientRequest::Operator {
            data: OperatorData {
                request_id: "g9-restart".into(),
                command: OperatorCommand::DaemonRestart { binary: None },
            },
        };
        match c.request(&request).map_err(|e| e.to_string())? {
            ClientResponse::CommandResult { data }
                if matches!(data.result, CommandResult::Restarting { .. }) =>
            {
                Ok(())
            }
            other => Err(format!("fake restart: {other:?}")),
        }
    }
}

/// Drops the first reply to a request whose command is `command` (the
/// request reached the daemon, its answer is lost); sets `dropped`.
fn drop_first_reply(command: &'static str, dropped: Arc<AtomicBool>) -> Options {
    let transform: Transform = Arc::new(move |state, direction, line| {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            return vec![line];
        };
        match direction {
            Direction::ToServer if v["data"]["command"]["command"] == command => state.flag = true,
            Direction::ToClient
                if state.flag
                    && v["type"] == "command_result"
                    && !dropped.swap(true, Ordering::SeqCst) =>
            {
                return vec![];
            }
            _ => {}
        }
        vec![line]
    });
    Options::rewrite(transform)
}

fn requests_of(fake: &FakeDaemon, command: &str) -> Vec<Value> {
    fake.requests()
        .iter()
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .filter(|v| v["data"]["command"]["command"] == command)
        .collect()
}

/// `== resend` (P5): a `send` whose reply was lost is sent again with the
/// same message id and kept once; `instance add` is never sent again.
pub fn resend(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(4);
    let mut px = Proxied::new(&home)?;
    let cli = Cli::new(&lab.agend, &home);
    let mut out = Vec::new();
    for (command, args, caller) in [
        (
            "send",
            &["send", "g9-b", "across a restart"][..],
            Some("g9-a"),
        ),
        (
            "instance_add",
            &["instance", "add", "g9-x", "claude"][..],
            None,
        ),
    ] {
        let dropped = Arc::new(AtomicBool::new(false));
        px.proxy(drop_first_reply(command, Arc::clone(&dropped)))?;
        let (child, started) = cli.spawn(Some(&home), caller, args, &[])?;
        wait_for(Duration::from_secs(10), "the lost reply", || {
            dropped.load(Ordering::SeqCst)
        })?;
        px.restart_fake()?;
        let run = finish(
            describe(caller, args),
            child,
            started,
            Duration::from_secs(30),
        );
        let sent = requests_of(&px.fake, command);
        out.extend(run.shown());
        if command == "send" {
            expect_run(&run, 0, &["accepted: message ", "(retried "])?;
            let ids: Vec<&Value> = sent
                .iter()
                .map(|v| &v["data"]["command"]["message_id"])
                .collect();
            ensure(
                sent.len() == 2 && ids[0] == ids[1] && ids[0].is_string(),
                || format!("the fake got these sends: {sent:?}"),
            )?;
            let kept = px.fake.message_ids_to("g9-b");
            ensure(kept.len() == 1, || format!("g9-b has {kept:?}"))?;
            out.push(format!(
                "the daemon got the send twice with the same message id {}; g9-b has it once",
                ids[0].as_str().unwrap_or("?")
            ));
        } else {
            expect_run(
                &run,
                1,
                &["agend: daemon restarted during the request; check with agend instance list"],
            )?;
            ensure(sent.len() == 1, || {
                format!("instance_add sent {} times", sent.len())
            })?;
            out.push("instance_add reached the daemon once and was not sent again".into());
        }
    }
    Ok(out)
}

/// `== restart waits` (P7): the CLI reports success only after the old
/// connection ended and a daemon with another boot id answers.
pub fn restart_waits(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(5);
    let mut px = Proxied::new(&home)?;
    let cli = Cli::new(&lab.agend, &home);
    let mut out = Vec::new();
    // The old connection never ends.
    let identity: Transform = Arc::new(|_, _, line| vec![line]);
    px.proxy(Options {
        transform: Arc::clone(&identity),
        keep_client_open: true,
        unbounded: false,
    })?;
    let run = cli.run(None, &["daemon", "restart"]);
    expect_run(
        &run,
        1,
        &[
            "agend: the daemon accepted the restart but did not stop within 30 s; check the daemon's terminal or log",
        ],
    )?;
    ensure(run.took >= Duration::from_secs(30), || {
        format!("gave up after {:?}", run.took)
    })?;
    out.extend(run.shown());
    // Every daemon that answers has the old boot id.
    let same_boot: Transform = Arc::new(|_, direction, line| {
        let Ok(mut v) = serde_json::from_str::<Value>(&line) else {
            return vec![line];
        };
        if direction == Direction::ToClient && v["type"] == "hello" {
            v["data"]["boot_id"] = json!(1);
        }
        vec![v.to_string()]
    });
    px.proxy(Options::rewrite(same_boot))?;
    let run = cli.run(None, &["daemon", "restart"]);
    expect_run(
        &run,
        1,
        &["agend: the daemon did not come back within 10 s (the old one still answers)"],
    )?;
    out.extend(run.shown());
    // A real new boot.
    px.proxy(Options::rewrite(identity))?;
    let run = cli.run(None, &["daemon", "restart"]);
    expect_run(&run, 0, &["the daemon is back: pid "])?;
    out.extend(run.shown());
    Ok(out)
}

/// `== after exec` (P7): holders the old image started are reaped when they
/// end; the new image's own holders and the preflight child keep their
/// exit status (no `waitpid(-1)`).
pub fn after_exec(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(6);
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    let inherited = add_sleeper(&cli, "g9-i")?;
    expect_run(
        &cli.run(None, &["daemon", "restart"]),
        0,
        &["the daemon is back"],
    )?;
    let line = daemon.expect("inherited holder children (exec restart): ")?;
    ensure(line.contains(&inherited.to_string()), || line.clone())?;
    let removed = cli.run(None, &["instance", "remove", "g9-i", "--yes"]);
    expect_run(&removed, 0, &["removed g9-i"])?;
    let reaped = daemon.expect_within(
        &format!("reaped inherited holder pid {inherited}"),
        Duration::from_secs(10),
    )?;
    let left = state_of(inherited);
    ensure(!left.starts_with('Z'), || {
        format!("holder {inherited} is {left}")
    })?;
    let own = add_sleeper(&cli, "g9-o")?;
    expect_run(
        &cli.run(None, &["instance", "remove", "g9-o", "--yes"]),
        0,
        &["removed g9-o"],
    )?;
    let exited = daemon.expect(&format!("holder g9-o (pid {own}) exited: "))?;
    ensure(!exited.contains("wait:"), || exited.clone())?;
    let failed = cli.run(None, &["daemon", "restart", "--binary", "/usr/bin/false"]);
    expect_run(&failed, 1, &["daemon preflight exited with status 1"])?;
    let out = vec![
        format!(
            "inherited holder g9-i (pid {inherited}) removed: {}",
            crate::lab::untimed(&reaped)
        ),
        format!("ps -o stat= -p {inherited}: {:?} (no zombie)", left),
        format!("own holder g9-o: {}", crate::lab::untimed(&exited)),
        format!(
            "preflight child: {}",
            failed.stderr.trim().trim_start_matches("agend: ")
        ),
    ];
    daemon.interrupt()?;
    Ok(out)
}

/// The milestone (gates 1–9): two fake codex agents send each other 10
/// messages; the daemon restarts in the middle, and one `send` loses its
/// reply to it (a proxy drops the answer; the CLI resends after the new
/// daemon is up). Every message arrives once and ends `confirmed`.
pub fn milestone(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(7);
    let fake_codex = crate::codex::fake_codex()?;
    let _bound = [
        crate::codex::BoundSocket::of(&home, "g9-a"),
        crate::codex::BoundSocket::of(&home, "g9-b"),
    ];
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    let program = fake_codex.display().to_string();
    for id in ["g9-a", "g9-b"] {
        let run = cli.run(
            None,
            &[
                "instance",
                "add",
                id,
                "codex",
                "--program",
                &program,
                "--",
                "--turn-ms",
                "200",
            ],
        );
        expect_run(&run, 0, &[&format!("added {id} (codex, ")])?;
    }
    for id in ["g9-a", "g9-b"] {
        daemon.expect_within(&format!("{id}: go (resume "), Duration::from_secs(60))?;
    }
    // A second home whose socket is a proxy to the daemon's.
    let side = lab.home(8);
    fs::create_dir_all(side.join("run")).map_err(|e| e.to_string())?;
    let dropped = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&dropped);
    let transform: Transform = Arc::new(move |state, direction, line| {
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            return vec![line];
        };
        match direction {
            Direction::ToServer if v["data"]["command"]["message"] == "a5" => state.flag = true,
            Direction::ToClient
                if state.flag
                    && v["type"] == "command_result"
                    && !seen.swap(true, Ordering::SeqCst) =>
            {
                return vec![];
            }
            _ => {}
        }
        vec![line]
    });
    let _proxy = Proxy::start_at(
        &side.join(DAEMON_SOCKET),
        home.join(DAEMON_SOCKET),
        Options::rewrite(transform),
    )
    .map_err(|e| e.to_string())?;
    let via_proxy = Cli::new(&lab.agend, &side);
    let mut out = Vec::new();
    for i in 1..=10 {
        let a = format!("a{i}");
        if i == 5 {
            let args = ["send", "g9-b", a.as_str()];
            let (child, started) = via_proxy.spawn(Some(&side), Some("g9-a"), &args, &[])?;
            wait_for(Duration::from_secs(20), "the lost reply to a5", || {
                dropped.load(Ordering::SeqCst)
            })?;
            let restart = cli.run(None, &["daemon", "restart"]);
            expect_run(&restart, 0, &["the daemon is back"])?;
            out.push("a5 reached the daemon, its reply was lost; agend daemon restart:".into());
            out.extend(restart.shown().into_iter().map(|l| format!("  {l}")));
            let run = finish(
                describe(Some("g9-a"), &args),
                child,
                started,
                Duration::from_secs(30),
            );
            expect_run(&run, 0, &["accepted: message ", "(retried "])?;
            out.push(format!("a5 after the restart: {}", run.stdout.trim()));
        } else {
            expect_run(
                &cli.run(Some("g9-a"), &["send", "g9-b", &a]),
                0,
                &["accepted"],
            )?;
        }
        let b = format!("b{i}");
        expect_run(
            &cli.run(Some("g9-b"), &["send", "g9-a", &b]),
            0,
            &["accepted"],
        )?;
    }
    let mut ids = Vec::new();
    for (me, from, prefix) in [("g9-b", "g9-a", "a"), ("g9-a", "g9-b", "b")] {
        let run = cli.run(Some(me), &["inbox", "--json"]);
        expect_run(&run, 0, &[])?;
        let value: Value = serde_json::from_str(run.stdout.trim()).map_err(|e| e.to_string())?;
        let messages = value["data"]["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let bodies: Vec<String> = messages
            .iter()
            .map(|m| m["body"].as_str().unwrap_or("").to_owned())
            .collect();
        let expected: Vec<String> = (1..=10).map(|n| format!("{prefix}{n}")).collect();
        ensure(bodies == expected, || format!("{me}'s inbox: {bodies:?}"))?;
        ensure(messages.iter().all(|m| m["from"] == from), || {
            format!("{me}'s senders")
        })?;
        ids.extend(messages.iter().map(|m| {
            (
                me.to_owned(),
                m["message_id"].as_str().unwrap_or("").to_owned(),
            )
        }));
        out.push(format!(
            "agend inbox of {me}: {} lines from {from}, {}..{}{}",
            bodies.len(),
            bodies.first().map_or("", String::as_str),
            bodies.last().map_or("", String::as_str),
            ", each once"
        ));
    }
    for (me, id) in &ids {
        daemon.expect_within(&format!("{me}: {id} confirmed"), Duration::from_secs(60))?;
    }
    out.push(format!(
        "daemon log: all {} messages `confirmed` (e.g. {})",
        ids.len(),
        crate::lab::untimed(&daemon.expect(&format!("{}: {} confirmed", ids[4].0, ids[4].1))?)
    ));
    daemon.interrupt()?;
    // The daemon is gone; the holders and their fake codex still run.
    let store = SqliteStore::open(&home, 0).map_err(|e| e.to_string())?;
    for me in ["g9-a", "g9-b"] {
        let rows = block_on(store.messages_to(me)).map_err(|e| e.to_string())?;
        ensure(
            rows.len() == 10 && rows.iter().all(|r| r.state == DeliveryState::Confirmed),
            || {
                format!(
                    "{me}'s rows: {:?}",
                    rows.iter().map(|r| (&r.body, r.state)).collect::<Vec<_>>()
                )
            },
        )?;
        let thread = block_on(store.instance(me))
            .map_err(|e| e.to_string())?
            .and_then(|i| i.session_id)
            .ok_or("no thread")?;
        let mut probe = Probe::connect(&launch::socket_path(&home, me))?;
        probe.call(
            "initialize",
            json!({"clientInfo": {"name": "g9-test", "title": null, "version": "0"}}),
        )?;
        let page = probe.call(
            "thread/turns/list",
            json!({"threadId": thread, "cursor": null, "limit": 1000}),
        )?;
        let text = page.to_string();
        for row in &rows {
            let n = text
                .matches(&format!("\"clientId\":\"{}\"", row.id))
                .count();
            ensure(n == 1, || {
                format!(
                    "{} ({}) is {n} times in {me}'s codex thread",
                    row.id, row.body
                )
            })?;
        }
        out.push(format!(
            "{me}: 10 rows, all confirmed; each message exactly once in its codex thread {thread}"
        ));
    }
    drop(store);
    for id in ["g9-a", "g9-b"] {
        let _ = agend_daemon::runtime::shutdown_holder(&home, id);
    }
    Ok(out)
}

/// `agend --version` start-up time: the median of `runs` runs.
pub fn startup(bin: &Path, runs: usize) -> Result<Duration, String> {
    let mut took = Vec::new();
    for _ in 0..runs {
        let started = Instant::now();
        let out = std::process::Command::new(bin)
            .arg("--version")
            .output()
            .map_err(|e| e.to_string())?;
        took.push(started.elapsed());
        ensure(out.status.success(), || "agend --version failed".into())?;
    }
    took.sort();
    Ok(took[took.len() / 2])
}

/// A `PATH` of stubs only (plus `/usr/bin:/bin` for `sh`): `git` prints
/// `git_version`, `claude` prints its version; codex and opencode are
/// missing. The real backends are never on it.
pub fn stub_path(lab: &Lab, name: &str, git_version: &str) -> Result<String, String> {
    let dir = lab.root.join(name);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (tool, said) in [
        ("git", format!("git version {git_version}")),
        ("claude", "2.1.3 (Claude Code)".to_owned()),
    ] {
        let path = dir.join(tool);
        fs::write(&path, format!("#!/bin/sh\necho '{said}'\n")).map_err(|e| e.to_string())?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(format!("{}:/usr/bin:/bin", dir.display()))
}

/// `== init and doctor` (P3, P8, P9).
pub fn init_and_doctor(lab: &Lab) -> Result<Vec<String>, String> {
    let root = lab.home(9);
    let home = root.join("home");
    let cli = Cli::new(&lab.agend, &home);
    let good = stub_path(lab, "stubs-new", "2.45.0")?;
    let old = stub_path(lab, "stubs-old", "2.30.0")?;
    let path = |p: &str| [("PATH", p.to_owned())];
    let run = |home: Option<&Path>, args: &[&str], p: &str| {
        let env = path(p);
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        cli.run_with(home, None, args, &env)
    };
    let mut out = Vec::new();
    let unset = run(None, &["init"], &good);
    expect_run(
        &unset,
        2,
        &[
            "agend: AGEND_HOME is not set; choose a directory for AgEnD's data and run: export AGEND_HOME=<absolute path>",
        ],
    )?;
    ensure(!home.exists(), || {
        "init without AGEND_HOME created something".into()
    })?;
    out.extend(unset.shown());
    let first = run(Some(&home), &["init"], &good);
    expect_run(
        &first,
        0,
        &[
            &format!("created {} (0700)", home.display()),
            &format!("ok    home      {} (0700)", home.display()),
            "warn  daemon    not reachable: ",
            "      fix: agend daemon",
            "ok    git       git 2.45.0",
            "ok    claude    2.1.3 (Claude Code)",
            "warn  codex     not on PATH; no instance uses it\n      fix: npm install -g @openai/codex",
            "ok    holders   0 running",
            "ok    disk      ",
            "next: agend daemon   (then, in another terminal) agend instance add dev-1 claude",
        ],
    )?;
    let mode = fs::metadata(&home)
        .map_err(|e| e.to_string())?
        .permissions()
        .mode()
        & 0o777;
    ensure(mode == 0o700, || format!("home is {mode:o}"))?;
    out.extend(first.shown());
    let again = run(Some(&home), &["init"], &good);
    expect_run(&again, 0, &[&format!("{} already exists", home.display())])?;
    out.push(again.shown()[..2].join("\n"));
    let broken_git = run(Some(&home), &["doctor"], &old);
    expect_run(
        &broken_git,
        1,
        &[
            "fail  git       git 2.30.0 is older than 2.38 (merge-tree needs it)\n      fix: ",
            "1 check failed",
        ],
    )?;
    out.extend(broken_git.shown());
    let json = run(Some(&home), &["doctor", "--json"], &old);
    let checks: Value =
        serde_json::from_str(json.stdout.trim()).map_err(|e| format!("doctor --json: {e}"))?;
    let checks = checks.as_array().cloned().unwrap_or_default();
    let names: Vec<&str> = checks.iter().filter_map(|c| c["check"].as_str()).collect();
    ensure(
        names
            == [
                "home", "daemon", "git", "claude", "codex", "opencode", "holders", "disk",
            ],
        || format!("doctor --json checks: {names:?}"),
    )?;
    ensure(
        checks
            .iter()
            .all(|c| c["status"] == "ok" || c["fix"].is_string())
            && json.code == Some(1),
        || {
            format!(
                "a warn or fail without fix, or exit {:?}: {checks:?}",
                json.code
            )
        },
    )?;
    out.push(format!(
        "agend doctor --json: {} checks, every warn/fail has fix, exit 1",
        checks.len()
    ));
    fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    let open = run(Some(&home), &["doctor"], &good);
    expect_run(
        &open,
        0,
        &[
            "warn  home      ",
            "(0755; others can read it)\n      fix: chmod 700 ",
        ],
    )?;
    out.push(open.shown()[1..3].join("\n"));
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    fs::write(home.join("fleet.yaml"), "instances: {}\n").map_err(|e| e.to_string())?;
    let v1 = run(Some(&home), &["init"], &good);
    expect_run(
        &v1,
        1,
        &["looks like an AgEnD v1 home (fleet.yaml); set AGEND_HOME to another directory"],
    )?;
    let v1_doctor = run(Some(&home), &["doctor"], &good);
    expect_run(
        &v1_doctor,
        1,
        &[
            "fail  home      ",
            "looks like an AgEnD v1 home (fleet.yaml)",
        ],
    )?;
    out.extend(v1.shown());
    fs::remove_file(home.join("fleet.yaml")).map_err(|e| e.to_string())?;
    // A daemon (the fake) with a codex instance, and a holder it does not
    // know: codex missing is a failure, the holder an orphan.
    fs::create_dir_all(home.join("run")).map_err(|e| e.to_string())?;
    let fake = FakeDaemon::start_at(&home.join(DAEMON_SOCKET)).map_err(|e| e.to_string())?;
    fake.set_instance(InstanceView {
        instance_id: "g9-c".into(),
        team_id: "general".into(),
        backend: "codex".into(),
        state: AgentState::Unknown,
        working_directory: None,
    });
    let mut holder = std::process::Command::new(&lab.agend)
        .args(["holder", "g9-orphan"])
        .env_clear()
        .env("AGEND_HOME", &home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let found = wait_for(Duration::from_secs(10), "the orphan holder", || {
        files::running(&home, "g9-orphan").ok().flatten().is_some()
    });
    let with_daemon = run(Some(&home), &["doctor"], &good);
    let _ = agend_daemon::runtime::shutdown_holder(&home, "g9-orphan");
    // Our own child: reaped, or killed if Shutdown did not end it.
    if wait_for(Duration::from_secs(10), "the orphan to end", || {
        matches!(holder.try_wait(), Ok(Some(_)))
    })
    .is_err()
    {
        let _ = holder.kill();
        let _ = holder.wait();
    }
    drop(fake);
    found?;
    expect_run(
        &with_daemon,
        1,
        &[
            "ok    daemon    pid ",
            "fail  codex     not on PATH; used by g9-c\n      fix: npm install -g @openai/codex",
            "warn  holders   1 running, 1 orphan: g9-orphan\n      fix: agend daemon restart",
        ],
    )?;
    out.extend(with_daemon.shown());
    Ok(out)
}

/// Whether a process with this pid is still there (and not a zombie):
/// `ps -o stat= -p <pid>`.
fn alive(pid: &str) -> bool {
    let state = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", pid.trim()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    !state.is_empty() && !state.starts_with('Z')
}

/// `== stop during a preflight` (verifier F1): Ctrl-C while a preflight
/// runs removes its home (a copy of agend.db) and ends its child.
pub fn stop_during_preflight(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(10);
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    let pid_file = lab.root.join("stopped-preflight.pid");
    let slow = script(
        lab,
        "stopped-agend",
        &format!("echo $$ > '{}'\nexec sleep 30", pid_file.display()),
    )?;
    let slow = slow.display().to_string();
    let args = ["daemon", "restart", "--binary", slow.as_str()];
    let (child, started) = cli.spawn(Some(&home), None, &args, &[])?;
    let line = daemon.expect("preflight home /tmp/agend-pf-")?;
    let pf = line.rsplit(' ').next().unwrap_or_default().to_owned();
    wait_for(Duration::from_secs(10), "the preflight child", || {
        pid_file.exists()
    })?;
    let pid = fs::read_to_string(&pid_file).map_err(|e| e.to_string())?;
    daemon.interrupt()?;
    let run = finish(
        describe(None, &args),
        child,
        started,
        Duration::from_secs(30),
    );
    let gone = wait_for(Duration::from_secs(5), "the preflight home to go", || {
        !Path::new(&pf).exists()
    });
    let ended = wait_for(Duration::from_secs(5), "the preflight child to end", || {
        !alive(&pid)
    });
    gone?;
    ended?;
    let stopped = daemon.expect(&format!("preflight pid {}: stopped", pid.trim()))?;
    let mut out = run.shown();
    out.push(crate::lab::untimed(&stopped).to_owned());
    out.push(format!(
        "Ctrl-C during the preflight: {pf} removed, preflight child pid {} ended",
        pid.trim()
    ));
    Ok(out)
}

/// `== preflight deadline` (verifier F2): a child that exits but leaves a
/// process holding its stdout is judged at once; one that never exits is
/// stopped after 60 s; either way the next restart may run.
pub fn preflight_deadline(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(11);
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    let mut out = Vec::new();
    // The process it leaves holds its stdout for 8 s: an answer before then
    // did not wait for the pipe (with room for a loaded host).
    let leaves = script(lab, "leaves-agend", "sleep 8 &\nexit 1")?;
    let leaves = leaves.display().to_string();
    let run = cli.run(None, &["daemon", "restart", "--binary", &leaves]);
    expect_run(
        &run,
        1,
        &["agend: preflight_failed: ", "exited with status 1"],
    )?;
    ensure(run.took < Duration::from_secs(8), || {
        format!("waited for the pipe: {:?}", run.took)
    })?;
    out.extend(run.shown());
    let pid_file = lab.root.join("hanging-preflight.pid");
    let hangs = script(
        lab,
        "hanging-agend",
        &format!("echo $$ > '{}'\nexec sleep 90", pid_file.display()),
    )?;
    let hangs = hangs.display().to_string();
    let run = cli.run(None, &["daemon", "restart", "--binary", &hangs]);
    expect_run(
        &run,
        1,
        &[&format!(
            "agend: preflight_failed: {hangs} daemon preflight did not finish within 60 s"
        )],
    )?;
    let pf = crate::lab::untimed(&daemon.expect("preflight home /tmp/agend-pf-")?).to_owned();
    let pid = fs::read_to_string(&pid_file).map_err(|e| e.to_string())?;
    ensure(!alive(&pid), || {
        format!("preflight child {} still runs", pid.trim())
    })?;
    ensure(preflight_left(&daemon).is_empty(), || {
        format!("left: {:?}", preflight_left(&daemon))
    })?;
    out.extend(run.shown());
    let next = cli.run(None, &["daemon", "restart", "--binary", "/usr/bin/false"]);
    expect_run(&next, 1, &["exited with status 1"])?;
    out.push(format!(
        "the child (pid {}) was stopped; {pf}: every preflight home removed; the next restart ran",
        pid.trim()
    ));
    daemon.interrupt()?;
    Ok(out)
}

/// The preflight homes this daemon logged that still exist.
fn preflight_left(daemon: &Daemon) -> Vec<String> {
    daemon
        .log
        .iter()
        .filter_map(|l| l.split("preflight home ").nth(1))
        .map(str::to_owned)
        .filter(|p| Path::new(p).exists())
        .collect()
}

/// `== limits` (verifier F3, L17): a body over 1 MiB is refused before it
/// is stored; a line over 8 MiB is refused and closed without being read;
/// the daemon and the next messages carry on.
pub fn limits(lab: &Lab) -> Result<Vec<String>, String> {
    use agend_core::protocol::client::{
        AgentCommand, ClientCommandData, MAX_LINE_BYTES, MAX_MESSAGE_BYTES,
    };
    let home = lab.home(12);
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    add_sleeper(&cli, "g9-a")?;
    add_sleeper(&cli, "g9-b")?;
    let socket = home.join(DAEMON_SOCKET);
    let send = |c: &mut ProbeClient, id: &str, body: String| {
        c.request(&ClientRequest::Command {
            data: ClientCommandData {
                request_id: id.into(),
                command: AgentCommand::Send {
                    to: "g9-b".into(),
                    message: body,
                    level: None,
                    message_id: None,
                },
            },
        })
        .map_err(|e| e.to_string())
    };
    let (mut c, _) = ProbeClient::hello(&socket, Some("g9-a")).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let reply = send(&mut c, "l-1", "x".repeat(MAX_MESSAGE_BYTES + 1))?;
    let ClientResponse::Error { data } = &reply else {
        return Err(format!("a body of 1 MiB + 1 got {reply:?}"));
    };
    ensure(
        data.code == "invalid_request"
            && data.message
                == format!(
                    "the message is {} bytes; a message is limited to 1048576 bytes (1 MiB)",
                    MAX_MESSAGE_BYTES + 1
                ),
        || format!("{data:?}"),
    )?;
    out.push(format!(
        "send of {} bytes: {}: {}",
        MAX_MESSAGE_BYTES + 1,
        data.code,
        data.message
    ));
    let reply = send(&mut c, "l-2", "y".repeat(MAX_MESSAGE_BYTES))?;
    ensure(
        matches!(&reply, ClientResponse::CommandResult { .. }),
        || format!("a body of exactly 1 MiB got {reply:?}"),
    )?;
    out.push(format!("send of {MAX_MESSAGE_BYTES} bytes: accepted"));
    let (mut long, _) = ProbeClient::hello(&socket, Some("g9-a")).map_err(|e| e.to_string())?;
    let _ = long.send_raw(&"z".repeat(MAX_LINE_BYTES + 1024));
    let answer = long.recv_within(Duration::from_secs(10));
    let closed = matches!(long.recv_within(Duration::from_secs(10)), Ok(None) | Err(_));
    let said = match &answer {
        Ok(Some(ClientResponse::Error { data })) => format!("{}: {}", data.code, data.message),
        other => format!("{other:?}"),
    };
    ensure(
        said.starts_with("invalid_request: a protocol line is limited to 8388608 bytes") && closed,
        || format!("a line over 8 MiB got {said}, closed {closed}"),
    )?;
    out.push(format!("a line over 8 MiB: {said}; closed"));
    let after = cli.run(Some("g9-a"), &["send", "g9-b", "still fine"]);
    expect_run(&after, 0, &["accepted: message "])?;
    let inbox = cli.run(Some("g9-b"), &["inbox"]);
    expect_run(&inbox, 0, &[" from g9-a: still fine"])?;
    out.push(
        "afterwards: agend send g9-b \"still fine\" → accepted, and it is in g9-b's inbox".into(),
    );
    ensure(daemon.log.iter().all(|l| !l.contains("panicked")), || {
        "daemon panicked".into()
    })?;
    daemon.interrupt()?;
    Ok(out)
}

/// Starts `send <to> <body>` from agent `from` on a raw connection and
/// returns the connection and when it was sent; the reply is read later.
fn raw_send(
    home: &Path,
    from: &str,
    to: &str,
    request_id: &str,
    body: String,
) -> Result<ProbeClient, String> {
    use agend_core::protocol::client::{AgentCommand, ClientCommandData};
    let (mut c, _) =
        ProbeClient::hello(&home.join(DAEMON_SOCKET), Some(from)).map_err(|e| e.to_string())?;
    c.send(&ClientRequest::Command {
        data: ClientCommandData {
            request_id: request_id.into(),
            command: AgentCommand::Send {
                to: to.into(),
                message: body,
                level: None,
                message_id: None,
            },
        },
    })
    .map_err(|e| e.to_string())?;
    Ok(c)
}

/// The reply to a `raw_send`, waiting at most `within`.
fn raw_reply(c: &mut ProbeClient, within: Duration) -> Result<ClientResponse, String> {
    let deadline = Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("no reply within {within:?}"));
        }
        match c.recv_within(left) {
            Ok(Some(ClientResponse::Event { .. })) => {}
            Ok(Some(reply)) => return Ok(reply),
            Ok(None) => return Err("the daemon closed the connection".into()),
            Err(_) => {}
        }
    }
}

/// A small first message to `to`, confirmed: the fake (like codex) shows a
/// thread's first user message late and later ones at once, echoing the
/// whole text, which is what makes the big messages overlap.
fn warm_up(cli: &Cli, daemon: &mut Daemon, to: &str) -> Result<(), String> {
    let run = cli.run(Some("g9-a"), &["send", to, "warm up"]);
    expect_run(&run, 0, &["accepted"])?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while !daemon
        .log
        .iter()
        .any(|l| l.contains(&format!("{to}: ")) && l.contains(" confirmed (turn "))
    {
        ensure(Instant::now() < deadline, || {
            format!("warm-up to {to} not confirmed")
        })?;
        let _ = daemon.expect_within("g9-test: no such line", Duration::from_millis(300));
    }
    Ok(())
}

fn add_codex(cli: &Cli, id: &str, extra: &[&str]) -> Result<(), String> {
    let program = crate::codex::fake_codex()?.display().to_string();
    let mut args = vec![
        "instance",
        "add",
        id,
        "codex",
        "--program",
        program.as_str(),
        "--",
        "--turn-ms",
        "200",
    ];
    args.extend_from_slice(extra);
    expect_run(&cli.run(None, &args), 0, &[&format!("added {id} (codex, ")])
}

/// `== large messages` (verifier r2 #1): three 512 KiB messages back to back
/// to a codex agent are each delivered and `confirmed`, and the agent can
/// still be removed. (512 KiB: larger than a unix socket's buffer on macOS
/// and on Linux, so both sides of the connection must read while writing.)
pub fn large_messages(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(13);
    let _bound = [
        crate::codex::BoundSocket::of(&home, "g9-a"),
        crate::codex::BoundSocket::of(&home, "g9-b"),
    ];
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    add_codex(&cli, "g9-a", &[])?;
    add_codex(&cli, "g9-b", &[])?;
    for id in ["g9-a", "g9-b"] {
        daemon.expect_within(&format!("{id}: go (resume "), Duration::from_secs(60))?;
    }
    warm_up(&cli, &mut daemon, "g9-b")?;
    std::thread::sleep(Duration::from_secs(1));
    let mut out = Vec::new();
    let started = Instant::now();
    // Back to back, like agents that do not wait for each other.
    let mut pending = Vec::new();
    for n in 1..=3 {
        let body = format!("{n}").repeat(512 * 1024);
        pending.push(raw_send(&home, "g9-a", "g9-b", &format!("big-{n}"), body)?);
    }
    for (n, c) in pending.iter_mut().enumerate() {
        let reply = raw_reply(c, Duration::from_secs(90))?;
        ensure(
            matches!(&reply, ClientResponse::CommandResult { .. }),
            || format!("512 KiB message {}: {reply:?}", n + 1),
        )?;
    }
    out.push(format!(
        "three 512 KiB messages sent back to back: all accepted within {:.1} s",
        started.elapsed().as_secs_f64()
    ));
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        // Reads more of the log (the needle never appears).
        let _ = daemon.expect_within("g9-test: no such line", Duration::from_millis(500));
        let confirmed = daemon
            .log
            .iter()
            .filter(|l| l.contains("g9-b: ") && l.contains(" confirmed (turn "))
            .count();
        // The warm-up and the three.
        if confirmed >= 4 {
            out.push("all three confirmed in g9-b's thread".into());
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{}/3 confirmed within 60 s",
                confirmed.saturating_sub(1)
            ));
        }
    }
    for id in ["g9-b", "g9-a"] {
        let run = cli.run(None, &["instance", "remove", id, "--yes"]);
        expect_run(&run, 0, &[&format!("removed {id}")])?;
        // It never answered before the fix (the CLI gives up at 10 s).
        expect_run(&run, 0, &[])?;
    }
    out.push("both agents removed".into());
    daemon.interrupt()?;
    Ok(out)
}

/// `== stuck peer` (verifier r2 #1): a codex app-server that stops reading
/// while its own write waits (`fake_codex --disable duplex-io`) never wedges
/// the daemon: the link's write times out, `instance remove` and Ctrl-C
/// finish in seconds.
pub fn stuck_peer(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(14);
    let _bound = [
        crate::codex::BoundSocket::of(&home, "g9-a"),
        crate::codex::BoundSocket::of(&home, "g9-s"),
        crate::codex::BoundSocket::of(&home, "g9-t"),
    ];
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    add_codex(&cli, "g9-a", &[])?;
    for id in ["g9-s", "g9-t"] {
        add_codex(&cli, id, &["--disable", "duplex-io"])?;
    }
    for id in ["g9-a", "g9-s", "g9-t"] {
        daemon.expect_within(&format!("{id}: go (resume "), Duration::from_secs(60))?;
    }
    let mut out = Vec::new();
    for to in ["g9-s", "g9-t"] {
        warm_up(&cli, &mut daemon, to)?;
    }
    std::thread::sleep(Duration::from_secs(1));
    // Three back to back: the app-server echoes the first while the link
    // writes the next, and neither side reads (verifier r2 #1).
    let mut pending = Vec::new();
    for to in ["g9-s", "g9-t"] {
        for n in 1..=3 {
            let body = format!("{n}").repeat(512 * 1024);
            pending.push(raw_send(
                &home,
                "g9-a",
                to,
                &format!("stuck-{to}-{n}"),
                body,
            )?);
        }
    }
    std::thread::sleep(Duration::from_secs(3));
    let run = cli.run(None, &["instance", "remove", "g9-s", "--yes"]);
    out.extend(run.shown());
    expect_run(&run, 0, &["removed g9-s"])?;
    // Before the fix `remove` never answered (the CLI gave up at 10 s) and
    // Ctrl-C did not stop the daemon (`interrupt` gives up at 10 s); the
    // bound leaves room for a loaded host.
    ensure(run.took < Duration::from_secs(20), || {
        format!("remove took {:?}", run.took)
    })?;
    let took = daemon.interrupt()?;
    out.push(format!(
        "a stuck app-server (g9-s): removed in {:.1} s; with another one stuck (g9-t), Ctrl-C stopped the daemon in {:.1} s",
        run.took.as_secs_f64(),
        took.as_secs_f64()
    ));
    Ok(out)
}

/// `== swapped binary` (verifier r2 #3): a binary overwritten in place
/// during its preflight, same size and its modification time put back, is
/// still refused (its change time moved); the daemon does not exec it.
pub fn swapped_binary(lab: &Lab) -> Result<Vec<String>, String> {
    use std::io::{Seek, SeekFrom, Write};
    let home = lab.home(15);
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    let body = "sleep 2\necho 'agend 0.0.0'\necho 'db copy: fine'\necho 'holder: hello ok, spawn ok, shutdown ok'\n# a";
    let path = script(lab, "swapped-agend", body)?;
    let shown = path.display().to_string();
    let args = ["daemon", "restart", "--binary", shown.as_str()];
    let (child, started) = cli.spawn(Some(&home), None, &args, &[])?;
    daemon.expect("preflight pid ")?;
    let mtime = fs::metadata(&path)
        .and_then(|m| m.modified())
        .map_err(|e| e.to_string())?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    // The same bytes but the last one, in place: same inode and size.
    let len = fs::metadata(&path).map_err(|e| e.to_string())?.len();
    file.seek(SeekFrom::Start(len - 1))
        .map_err(|e| e.to_string())?;
    file.write_all(b"b").map_err(|e| e.to_string())?;
    file.set_modified(mtime).map_err(|e| e.to_string())?;
    drop(file);
    let run = finish(
        describe(None, &args),
        child,
        started,
        Duration::from_secs(60),
    );
    expect_run(
        &run,
        1,
        &[&format!(
            "agend: preflight_failed: {shown} changed while its preflight ran"
        )],
    )?;
    let (pid, _) = hello(&home)?;
    ensure(pid == Some(daemon.pid), || "the daemon was replaced".into())?;
    let mut out = run.shown();
    out.push(
        "overwritten in place with the same size and mtime: refused; the daemon still runs".into(),
    );
    daemon.interrupt()?;
    Ok(out)
}

/// `== Ctrl-C during a restart` (found rerunning verifier r2c): a stop
/// signal that arrives after the preflight passed, while the daemon shuts
/// down to `exec`, stops it instead of being lost in the old image (which
/// left the new one running).
pub fn ctrl_c_during_restart(lab: &Lab) -> Result<Vec<String>, String> {
    let home = lab.home(16);
    let mut daemon = start(lab, &home)?;
    let cli = Cli::new(&lab.agend, &home);
    let agend = lab.agend.display().to_string();
    // Slow on purpose, so the signal lands in the hand-off; then the real
    // agend (its preflight passes; the exec would run it as `daemon`).
    let slow = script(
        lab,
        "slow-real-agend",
        &format!("sleep 1\nexec '{agend}' \"$@\""),
    )?;
    let slow = slow.display().to_string();
    let args = ["daemon", "restart", "--binary", slow.as_str()];
    let (child, started) = cli.spawn(Some(&home), None, &args, &[])?;
    daemon.expect("preflight passed; restarting with ")?;
    let stopped = daemon.interrupt()?;
    let run = finish(
        describe(None, &args),
        child,
        started,
        Duration::from_secs(60),
    );
    let execed = daemon.log.iter().any(|l| l.contains(" exec "));
    ensure(!execed, || {
        format!("the daemon exec'd after Ctrl-C:\n{}", daemon.log.join("\n"))
    })?;
    let mut out = run.shown();
    out.push(format!(
        "Ctrl-C right after `preflight passed`: the daemon stopped in {:.1} s and did not exec",
        stopped.as_secs_f64()
    ));
    Ok(out)
}
