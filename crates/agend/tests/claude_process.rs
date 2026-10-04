//! Real daemon/holder lifecycles with shell agents; never runs Claude or a model.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

use agend_daemon::{
    driver::codex::sweep::{argv, group_members},
    runtime::files,
    store::{Instance, InstanceStatus, SqliteStore},
};
use agend_testkit::block_on;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_agend");
const SCRIPT: &str =
    "trap '' HUP; printf 'native Claude sweep ready\\n'; while :; do sleep 1; done";

fn read(home: &Path) -> Instance {
    let store = SqliteStore::open(home, 0).unwrap();
    block_on(store.instance("claude")).unwrap().unwrap()
}

fn matching(pgid: u32, session: &str) -> bool {
    group_members(pgid).into_iter().any(|pid| {
        argv(pid).is_some_and(|args| {
            args.windows(2)
                .any(|w| matches!(w[0].as_str(), "--session-id" | "--resume") && w[1] == session)
        })
    })
}

struct Groups(Vec<(u32, String)>);
impl Drop for Groups {
    fn drop(&mut self) {
        for (pgid, session) in &self.0 {
            if *pgid > 1 && *pgid <= i32::MAX as u32 && matching(*pgid, session) {
                // Only groups created in this lab, attributed again at cleanup.
                unsafe {
                    libc::killpg(*pgid as i32, libc::SIGKILL);
                }
            }
        }
    }
}

fn wait(what: &str, mut check: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(6);
    while !check() {
        assert!(Instant::now() < end, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn kill_holder(home: &Path) {
    let pid = files::running(home, "claude").unwrap().unwrap();
    assert!(pid > 1);
    // This holder was started in this test's private home and still owns its lock.
    assert_eq!(files::running(home, "claude").unwrap(), Some(pid));
    assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGKILL) }, 0);
    wait("holder death", || {
        files::running(home, "claude").unwrap().is_none()
    });
}

#[test]
fn claude_sweep_cleans_dead_holder_groups_online_and_on_next_boot() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g12s");
    let home = lab.home(0);
    let instance = lab::add(&home, "claude", SCRIPT).unwrap();
    let session = instance.session_id.unwrap();
    let mut groups = Groups(vec![]);

    let mut d = lab::Daemon::start(&lab, &home, &[]).unwrap();
    d.ready().unwrap();
    d.interrupt().unwrap();
    let pgid = read(&home).agent_pid.unwrap();
    groups.0.push((pgid, session.clone()));
    wait("first agent argv", || matching(pgid, &session));
    let mut d = lab::Daemon::start(&lab, &home, &[]).unwrap();
    d.ready().unwrap();
    kill_holder(&home);
    let line = d
        .expect(&format!("claude: sweep of agent group {pgid}"))
        .unwrap();
    assert!(line.contains("SIGKILL sent"), "{line}");
    wait("online sweep", || !matching(pgid, &session));
    d.expect("claude: holder pid=").unwrap();
    d.interrupt().unwrap();

    let pgid = read(&home).agent_pid.unwrap();
    groups.0.push((pgid, session.clone()));
    wait("resumed agent argv", || matching(pgid, &session));
    kill_holder(&home);
    assert!(
        matching(pgid, &session),
        "agent must ignore SIGHUP to exercise the sweep"
    );
    let mut d = lab::Daemon::start(&lab, &home, &[]).unwrap();
    d.ready().unwrap();
    let line = d
        .expect(&format!("claude: sweep of agent group {pgid}"))
        .unwrap();
    assert!(line.contains("SIGKILL sent"), "{line}");
    wait("boot sweep", || !matching(pgid, &session));
    d.interrupt().unwrap();
    lab.stop_all_holders();
    assert!(lab.running_holders().is_empty());
}

#[test]
fn failed_claude_with_a_live_holder_is_left_untouched() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g12s");
    let home = lab.home(0);
    let instance = lab::add(&home, "claude", SCRIPT).unwrap();
    let session = instance.session_id.unwrap();
    let mut d = lab::Daemon::start(&lab, &home, &[]).unwrap();
    d.ready().unwrap();
    d.interrupt().unwrap();
    let pgid = read(&home).agent_pid.unwrap();
    let _groups = Groups(vec![(pgid, session.clone())]);
    wait("agent argv", || matching(pgid, &session));
    let holder = files::running(&home, "claude").unwrap();
    {
        let store = SqliteStore::open(&home, 0).unwrap();
        block_on(store.set_instance_status("claude", InstanceStatus::Failed)).unwrap();
    }
    let mut d = lab::Daemon::start(&lab, &home, &[]).unwrap();
    d.ready().unwrap();
    assert!(
        !d.log
            .iter()
            .any(|line| line.contains("sweep of agent group"))
    );
    assert!(matching(pgid, &session));
    assert_eq!(files::running(&home, "claude").unwrap(), holder);
    d.interrupt().unwrap();
    assert_eq!(read(&home).agent_pid, Some(pgid));
    lab.stop_all_holders();
    assert!(lab.running_holders().is_empty());
}

#[test]
fn foreign_claude_configuration_retries_three_times_then_fails_without_running_the_agent() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g12s");
    let home = lab.home(0);
    let instance = lab::add(&home, "claude", "touch agent-was-started; sleep 600").unwrap();
    let work = Path::new(&instance.working_directory);
    fs::write(work.join("CLAUDE.md"), "user instructions").unwrap();
    let mut d = lab::Daemon::start(&lab, &home, &[]).unwrap();
    d.ready().unwrap();
    d.expect("restarted 3 times in 10m and it still died; not restarting")
        .unwrap();
    assert_eq!(
        d.log
            .iter()
            .filter(|line| line.contains("claude: start failed:") && line.contains("CLAUDE.md"))
            .count(),
        4
    );
    assert!(d.log.iter().any(|line| line.contains("CLAUDE.md")));
    assert_eq!(
        fs::read_to_string(work.join("CLAUDE.md")).unwrap(),
        "user instructions"
    );
    assert!(!work.join("agent-was-started").exists());
    assert!(!work.join(".mcp.json").exists());
    assert!(lab.running_holders().is_empty());
    d.interrupt().unwrap();
    assert_eq!(read(&home).status, InstanceStatus::Failed);
    assert!(!read(&home).session_started);
}
