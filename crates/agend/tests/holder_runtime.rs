//! The daemon's `HolderRuntime` against real `agend holder` processes (gate 6
//! P4, contract layers 1 and 2):
//!
//! 1. RTM-1..9 (CONTRACTS.md) with the real runtime and real holders; a
//!    "daemon boot" is a new `HolderRuntime` over the same home.
//! 2. Four boots in four processes: this test binary re-executed
//!    (`boot_child`), one process per boot, none alive in between, like gate
//!    5 P7. The negative check (a new `AGEND_HOME` each boot) must fail.
//!
//! Safety: homes are fresh directories under `/tmp` (`common::Lab`); holders
//! are stopped with `Shutdown`, and the lab's cleanup only SIGKILLs a pid
//! (> 1) from its own lock files. Children are this binary, waited with a
//! deadline.
#![cfg(unix)]

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agend_core::model::Backend;
use agend_core::traits::{HolderHandle, HolderLaunch, Runtime};
use agend_daemon::runtime::{HolderRuntime, files};
use agend_testkit::block_on;
use agend_testkit::contract::runtime::{self as contract, RuntimeFixture};

const BIN: &str = env!("CARGO_BIN_EXE_agend");
const ROLE: &str = "G6_RUNTIME_BOOT";

fn runtime(home: &Path) -> HolderRuntime {
    HolderRuntime::new(home, Path::new(BIN), Vec::new(), Arc::new(|_| {}))
}

fn launch(home: &Path, id: &str, script: &str) -> HolderLaunch {
    HolderLaunch {
        instance_id: id.into(),
        backend: Backend::Claude,
        executable: "/bin/bash".into(),
        args: vec!["-c".into(), script.into()],
        working_directory: home.display().to_string(),
    }
}

// ---- layer 1: RTM-1..9 ----

struct Fixture {
    home: PathBuf,
    runtime: HolderRuntime,
}

impl RuntimeFixture for Fixture {
    type Runtime = HolderRuntime;
    type Error = agend_daemon::runtime::RuntimeError;
    type Persisted = PathBuf;

    fn runtime(&self) -> &HolderRuntime {
        &self.runtime
    }

    fn launch(&self, instance_id: &str) -> HolderLaunch {
        launch(&self.home, instance_id, "exec sleep 600")
    }

    /// The lock alone (a connection would take over the runtime's own).
    fn is_running(home: &PathBuf, handle: &HolderHandle) -> bool {
        let running = files::running(home, &handle.instance_id).ok().flatten();
        running.is_some() && running == handle.process_id
    }

    fn persisted(&self) -> PathBuf {
        self.home.clone()
    }

    fn boot(home: &PathBuf) -> Self {
        Self {
            home: home.clone(),
            runtime: runtime(home),
        }
    }
}

#[test]
fn contract_rtm_1_to_9_passes_against_real_holders() {
    let lab = lab::Lab::new(Path::new(BIN));
    let mut n = 0;
    let report = contract::run("holder", || {
        n += 1;
        let home = lab.home(n);
        Fixture {
            runtime: runtime(&home),
            home,
        }
    });
    println!("{}", report.summary());
    assert_eq!(
        lab.stop_all_holders(),
        0,
        "the contract left holders running"
    );
    report.assert_passed();
    assert_eq!(report.total(), contract::cases::<Fixture>().len());
}

// ---- layer 2: four boots, four processes ----

const COUNTER: &str = "i=0; while :; do i=$((i+1)); echo \"counter=$i\"; sleep 1; done";

fn say(line: &str) {
    println!("\n@{line}");
}

fn last_counter(screen: &str) -> u64 {
    screen
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("counter="))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("no counter on screen:\n{screen}"))
}

/// One daemon boot (a child process): recover first, then the boot's work.
fn boot(home: &Path, n: u32) {
    let rt = runtime(home);
    let pid = std::process::id();
    let recovered = block_on(rt.recover_holders()).unwrap();
    let ids: Vec<String> = recovered
        .iter()
        .map(|h| format!("{}:{}", h.instance_id, h.process_id.unwrap_or(0)))
        .collect();
    match n {
        1 => {
            assert!(recovered.is_empty(), "boot 1 recovered {ids:?}");
            let started = block_on(rt.start(&launch(home, "g6-l2a", COUNTER))).unwrap();
            say(&format!(
                "boot 1 pid={pid} started holder={}",
                started.handle.process_id.unwrap()
            ));
        }
        2 => {
            assert_eq!(recovered.len(), 1, "boot 2 recovered {ids:?}");
            say(&format!(
                "boot 2 pid={pid} (recover only) holders={}",
                ids.join(",")
            ));
        }
        _ => {
            let expected = if n == 3 { 1 } else { 2 };
            assert_eq!(recovered.len(), expected, "boot {n} recovered {ids:?}");
            let first = &recovered[0];
            let pid_a = first.process_id.unwrap();
            let seen = block_on(rt.attach(&launch(home, "g6-l2a", COUNTER), pid_a)).unwrap();
            assert_eq!(
                seen.attached.spawn,
                Some(agend_daemon::runtime::SpawnOutcome::AlreadySpawned)
            );
            let counter = last_counter(&seen.attached.screen);
            if n == 3 {
                let b = block_on(rt.start(&launch(home, "g6-l2b", COUNTER))).unwrap();
                say(&format!(
                    "boot 3 pid={pid} holders={} counter={counter} started g6-l2b={}",
                    ids.join(","),
                    b.handle.process_id.unwrap()
                ));
            } else {
                for handle in &recovered {
                    block_on(rt.stop(&handle.instance_id)).unwrap();
                }
                say(&format!(
                    "boot 4 pid={pid} holders={} counter={counter} ok; stopped both",
                    ids.join(",")
                ));
            }
        }
    }
}

/// Not a real test: the body of a boot process.
#[test]
fn boot_child() {
    let Ok(n) = std::env::var(ROLE) else {
        return;
    };
    let home = PathBuf::from(std::env::var("AGEND_HOME").unwrap());
    boot(&home, n.parse().unwrap());
}

fn run_boot(home: &Path, n: u32) -> Result<String, String> {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "boot_child", "--nocapture", "--test-threads=1"])
        .env(ROLE, n.to_string())
        .env("AGEND_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("boot {n} ran over 60 s"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut out = String::new();
    let mut err = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut().unwrap(), &mut out).unwrap();
    std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut err).unwrap();
    let line = out
        .lines()
        .find_map(|l| l.strip_prefix('@'))
        .map(str::to_owned);
    match (status.success(), line) {
        (true, Some(line)) => Ok(line),
        _ => Err(format!(
            "boot {n} failed: child pid={} {status}\n{out}\n{err}",
            child.id()
        )),
    }
}

fn four_boots(home_of: &dyn Fn(u32) -> PathBuf) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for n in 1..=4 {
        lines.push(run_boot(&home_of(n), n)?);
        std::thread::sleep(Duration::from_millis(1500));
    }
    Ok(lines)
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("no {key}= in {line}"))
}

#[test]
fn four_boots_in_four_processes_recover_the_same_holders() {
    let lab = lab::Lab::new(Path::new(BIN));
    let home = lab.home(1);
    let lines = four_boots(&|_| home.clone()).unwrap();
    for line in &lines {
        println!("{line}");
    }
    let holder = field(&lines[0], "holder");
    assert!(lines[1].contains(&format!("g6-l2a:{holder}")), "{lines:?}");
    assert!(lines[2].contains(&format!("g6-l2a:{holder}")), "{lines:?}");
    assert!(lines[3].contains(&format!("g6-l2a:{holder}")), "{lines:?}");
    let b = field(&lines[2], "g6-l2b");
    assert!(lines[3].contains(&format!("g6-l2b:{b}")), "{lines:?}");
    let c3: u64 = field(&lines[2], "counter").parse().unwrap();
    let c4: u64 = field(&lines[3], "counter").parse().unwrap();
    assert!(c4 > c3, "counter {c3} -> {c4}");
    let pids: std::collections::BTreeSet<&str> = lines.iter().map(|l| field(l, "pid")).collect();
    assert_eq!(pids.len(), 4, "{lines:?}");
    assert_eq!(lab.stop_all_holders(), 0, "boot 4 stopped every holder");
}

#[test]
fn four_boots_with_a_new_home_each_boot_fail() {
    let lab = lab::Lab::new(Path::new(BIN));
    let error = four_boots(&|n| lab.home(n as usize)).expect_err("a new home each boot must fail");
    println!("{}", error.lines().next().unwrap());
    assert!(error.starts_with("boot 2 failed"), "{error}");
    assert!(error.contains("boot 2 recovered []"), "{error}");
}
