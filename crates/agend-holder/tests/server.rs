//! In-process tests of the holder protocol server (`server::serve`) with real
//! agents (`/bin/bash`, `/usr/bin/env`, `yes`) on real PTYs. The process-level
//! behaviour (setsid, ignored signals, lock, surviving the launcher) is tested
//! with the real binary in `crates/agend/tests/holder_process.rs`.
//!
//! Every holder is stopped with `Shutdown` (also on panic, via `Drop`).

use std::collections::BTreeMap;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agend_core::protocol::holder::{
    ControlKey, ExitedData, HolderRequest, HolderResponse, SendControlKeyData, SpawnData,
};
use agend_core::protocol::{Hello, ProtocolVersion};
use agend_holder::client::{HolderClient, decode_bytes};
use agend_holder::server::{Config, Stop, serve};
use agend_testkit::tempdir::TempDir;

const ID: &str = "t1";
const LONG: Duration = Duration::from_secs(30);

/// Ends the whole test process if a test runs longer than 120 s, so a hang
/// fails in minutes instead of running into the CI job timeout.
struct Watchdog(Option<std::sync::mpsc::Sender<()>>);

impl Watchdog {
    fn arm(name: &str) -> Self {
        let (done, wait) = std::sync::mpsc::channel::<()>();
        let name = name.to_string();
        std::thread::spawn(move || {
            if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                wait.recv_timeout(Duration::from_secs(120))
            {
                eprintln!("watchdog: {name} ran over 120 s; aborting the test process");
                std::process::exit(101);
            }
        });
        Self(Some(done))
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Some(done) = self.0.take() {
            let _ = done.send(());
        }
    }
}

struct TestHolder {
    _watchdog: Watchdog,
    dir: TempDir,
    home: PathBuf,
    socket: PathBuf,
    server: Option<JoinHandle<Stop>>,
}

impl TestHolder {
    fn start(idle_exit: Duration) -> Self {
        Self::start_with(idle_exit, agend_holder::server::LAG_LIMIT)
    }

    fn start_with(idle_exit: Duration, lag_limit: usize) -> Self {
        let dir = TempDir::new("hs").unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let socket = dir.path().join("h.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let config = Config {
            instance_id: ID.into(),
            agend_home: home.clone(),
            idle_exit,
            lag_limit,
        };
        let watchdog = Watchdog::arm(std::thread::current().name().unwrap_or("test"));
        let server = std::thread::spawn(move || serve(listener, config));
        Self {
            _watchdog: watchdog,
            dir,
            home,
            socket,
            server: Some(server),
        }
    }

    fn connect(&self) -> (HolderClient, agend_holder::client::Greeting) {
        HolderClient::connect(&self.socket, Duration::from_millis(300)).unwrap()
    }

    fn spawn_request(&self, program: &str, args: &[&str]) -> HolderRequest {
        HolderRequest::Spawn {
            data: SpawnData {
                instance_id: ID.into(),
                program: program.into(),
                args: args.iter().map(|a| a.to_string()).collect(),
                env: BTreeMap::from([("PATH".into(), "/bin:/usr/bin".into())]),
                working_directory: self.dir.path().display().to_string(),
            },
        }
    }

    /// Connects, spawns `bash -c script`, returns the client and agent pid.
    fn spawn_bash(&self, script: &str) -> (HolderClient, u32) {
        let (mut client, _) = self.connect();
        client
            .send(&self.spawn_request("/bin/bash", &["-c", script]))
            .unwrap();
        let pid = loop {
            match client.recv(LONG).unwrap() {
                Some(HolderResponse::Spawned { data }) => break data.process_id.unwrap(),
                Some(HolderResponse::PtyBytes { .. }) => {}
                other => panic!("expected Spawned, got {other:?}"),
            }
        };
        (client, pid)
    }

    /// Stops the holder with `Shutdown` and returns why `serve` ended.
    /// Sends `Shutdown` right after `hello` (without waiting for the
    /// greeting, which a flooding agent may delay) and retries until `serve`
    /// returns. Never blocks forever: after 30 s it panics instead.
    fn shutdown(&mut self) -> Stop {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.server.as_ref().unwrap().is_finished() {
            assert!(Instant::now() < deadline, "holder did not stop");
            if let Ok(mut client) =
                HolderClient::connect_with(&self.socket, &HolderRequest::hello())
            {
                let _ = client.send(&HolderRequest::Shutdown);
                let _ = client.wait_closed(Duration::from_secs(2));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.join()
    }

    /// Waits for `serve` to return, for at most 30 s.
    fn join(&mut self) -> Stop {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.server.as_ref().unwrap().is_finished() {
            assert!(
                Instant::now() < deadline,
                "serve did not return within 30 s"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        self.server.take().unwrap().join().unwrap()
    }
}

impl Drop for TestHolder {
    fn drop(&mut self) {
        if self.server.is_none() {
            return;
        }
        if !std::thread::panicking() {
            self.shutdown();
            return;
        }
        // Already panicking: one best-effort Shutdown, never hang or panic here.
        if let Ok(mut client) = HolderClient::connect_with(&self.socket, &HolderRequest::hello()) {
            let _ = client.send(&HolderRequest::Shutdown);
            let _ = client.wait_closed(Duration::from_secs(10));
        }
    }
}

/// Asks for snapshots until one satisfies `done`.
fn wait_screen(client: &mut HolderClient, done: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + LONG;
    let mut last_screen = String::new();
    loop {
        client.send(&HolderRequest::Snapshot).unwrap();
        loop {
            match client.recv(LONG).unwrap_or_else(|e| {
                panic!(
                    "wait_screen: recv errored after {LONG:?}: {e:?}, last screen: {last_screen:?}"
                )
            }) {
                Some(HolderResponse::ScreenSnapshot { data }) => {
                    last_screen = data.screen.clone();
                    if done(&data.screen) {
                        return data.screen;
                    }
                    break;
                }
                Some(HolderResponse::PtyBytes { .. }) => {}
                other => panic!("unexpected {other:?}, last screen: {last_screen:?}"),
            }
        }
        assert!(
            Instant::now() < deadline,
            "screen never matched within {LONG:?}, last screen: {last_screen:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_exited(client: &mut HolderClient) -> ExitedData {
    let mut last: Option<HolderResponse> = None;
    loop {
        match client.recv(LONG).unwrap_or_else(|e| {
            panic!("wait_exited: recv errored after {LONG:?}: {e:?}, last message: {last:?}")
        }) {
            Some(HolderResponse::Exited { data }) => return data,
            Some(other @ (HolderResponse::PtyBytes { .. } | HolderResponse::Spawned { .. })) => {
                last = Some(other);
            }
            other => panic!("expected Exited, got {other:?}, last message: {last:?}"),
        }
    }
}

fn wait_error(client: &mut HolderClient) -> String {
    loop {
        match client.recv(LONG).unwrap() {
            Some(HolderResponse::Error { data }) => return data.code,
            Some(HolderResponse::PtyBytes { .. }) => {}
            other => panic!("expected Error, got {other:?}"),
        }
    }
}

/// Read-only: pids whose process group is `pgid`.
fn processes_in_group(pgid: u32) -> Vec<String> {
    let out = std::process::Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,pgid="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.split_whitespace().nth(1) == Some(&pgid.to_string()))
        .map(str::to_string)
        .collect()
}

fn numbers(text: &str) -> Vec<u64> {
    text.split("n=")
        .skip(1)
        .filter_map(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .collect()
}

#[test]
fn reconnect_gets_snapshot_then_live_bytes_without_gap_or_duplicate() {
    let holder = TestHolder::start(LONG);
    let script = "i=0; while [ $i -lt 20000 ]; do i=$((i+1)); echo n=$i; \
                  if [ $((i % 400)) -eq 0 ]; then sleep 0.05; fi; done; sleep 30";
    let (first, _) = holder.spawn_bash(script);
    std::thread::sleep(Duration::from_millis(300));
    drop(first);

    let (mut client, greeting) = holder.connect();
    assert_eq!(greeting.version, ProtocolVersion::new(1, 0));
    assert_eq!(greeting.exited, None);
    let last_row = greeting
        .screen
        .lines()
        .rfind(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string();
    let mut stream = String::new();
    let mut chunks = 0;
    while !stream.contains("n=20000") {
        match client.recv(LONG).unwrap() {
            Some(HolderResponse::PtyBytes { data }) => {
                chunks += 1;
                stream.push_str(
                    &String::from_utf8(decode_bytes(&data.bytes_base64).unwrap()).unwrap(),
                );
            }
            other => panic!("expected PtyBytes, got {other:?}"),
        }
    }
    assert!(chunks > 1, "reconnected after the agent finished printing");
    // The last row of the snapshot continues exactly where the stream starts.
    let seen = numbers(&format!("{last_row}{stream}"));
    let first = seen[0];
    assert!(first > 1, "snapshot was taken before any output");
    let expected: Vec<u64> = (first..=20000).collect();
    assert_eq!(
        seen, expected,
        "gap or duplicate between snapshot and stream"
    );
}

#[test]
fn control_key_reaches_the_agent_and_unknown_key_writes_nothing() {
    let holder = TestHolder::start(LONG);
    let (mut client, _) = holder.spawn_bash(
        "n=0; echo ready; while IFS= read -r -s -n 1 k; do n=$((n+1)); echo \"got $k keys=$n\"; done",
    );
    wait_screen(&mut client, |s| s.contains("ready"));
    let key = |key| HolderRequest::SendControlKey {
        data: SendControlKeyData { key },
    };
    client.send(&key(ControlKey::Y)).unwrap();
    wait_screen(&mut client, |s| s.contains("got y keys=1"));

    // A key this holder does not know arrives as `Unknown`.
    client.send(&key(ControlKey::Unknown)).unwrap();
    assert_eq!(wait_error(&mut client), "unknown_control_key");

    client.send(&key(ControlKey::N)).unwrap();
    let screen = wait_screen(&mut client, |s| s.contains("got n"));
    assert!(screen.contains("got n keys=2"), "{screen}");
}

#[test]
fn exit_code_and_signal_are_reported_on_every_connection() {
    let holder = TestHolder::start(LONG);
    let (mut client, _) = holder.spawn_bash("echo bye; exit 7");
    let exited = ExitedData {
        code: Some(7),
        signal: None,
    };
    assert_eq!(wait_exited(&mut client), exited);
    drop(client);
    for _ in 0..2 {
        let (_, greeting) = holder.connect();
        assert_eq!(greeting.exited.as_ref(), Some(&exited));
        assert!(greeting.screen.contains("bye"));
    }

    let holder = TestHolder::start(LONG);
    let (mut client, _) = holder.spawn_bash("kill -KILL $$");
    assert_eq!(
        wait_exited(&mut client),
        ExitedData {
            code: None,
            signal: Some("SIGKILL".into())
        }
    );
}

#[test]
fn agent_environment_is_only_what_spawn_carries() {
    let holder = TestHolder::start(LONG);
    let (mut client, _) = holder.connect();
    client
        .send(&holder.spawn_request("/usr/bin/env", &[]))
        .unwrap();
    wait_exited(&mut client);
    let screen = wait_screen(&mut client, |s| s.contains("TERM="));
    let vars: Vec<&str> = screen.lines().filter(|l| !l.is_empty()).collect();
    assert!(vars.contains(&"PATH=/bin:/usr/bin"), "{vars:?}");
    assert!(vars.contains(&"TERM=xterm-256color"), "{vars:?}");
    // portable-pty always sets SHELL; nothing else from this process leaks.
    for var in &vars {
        assert!(
            var.starts_with("PATH=") || var.starts_with("TERM=") || var.starts_with("SHELL="),
            "leaked into the agent: {var}"
        );
    }
}

#[test]
fn a_second_spawn_is_refused_and_changes_nothing() {
    let holder = TestHolder::start(LONG);
    let (mut client, pid) = holder.spawn_bash("echo first; sleep 30");
    wait_screen(&mut client, |s| s.contains("first"));
    // As a daemon that restarted between holder start and Spawn would do.
    drop(client);
    let (mut client, _) = holder.connect();
    client
        .send(&holder.spawn_request("/bin/bash", &["-c", "echo second"]))
        .unwrap();
    assert_eq!(wait_error(&mut client), "already_spawned");
    std::thread::sleep(Duration::from_millis(300));
    let screen = wait_screen(&mut client, |_| true);
    assert!(screen.contains("first") && !screen.contains("second"));
    assert!(
        !processes_in_group(pid).is_empty(),
        "first agent was replaced"
    );
}

#[test]
fn a_new_client_takes_over_and_the_old_one_is_closed() {
    let holder = TestHolder::start(LONG);
    let (mut old, _) = holder.spawn_bash("sleep 30");
    let (mut new, _) = holder.connect();
    assert!(old.wait_closed(LONG), "old connection still open");
    let screen = wait_screen(&mut new, |_| true);
    assert!(screen.is_empty() || screen.chars().all(|c| c == '\n'));
}

#[test]
fn version_mismatch_is_refused_and_the_holder_keeps_serving() {
    let holder = TestHolder::start(LONG);
    let hello = HolderRequest::Hello {
        data: Hello::new(&[ProtocolVersion::new(99, 0)]),
    };
    let mut client = HolderClient::connect_with(&holder.socket, &hello).unwrap();
    assert_eq!(wait_error(&mut client), "version_mismatch");
    assert!(client.wait_closed(LONG));
    let (_, greeting) = holder.connect();
    assert_eq!(greeting.version, ProtocolVersion::new(1, 0));
}

#[test]
fn a_client_too_far_behind_is_disconnected() {
    // A 64 KiB limit instead of the production 1 MiB, so a slow CI runner
    // still overflows it within the wait; the mechanism is the same.
    let holder = TestHolder::start_with(LONG, 64 * 1024);
    let (mut slow, _) = holder.spawn_bash("exec yes 0123456789abcdef0123456789abcdef");
    // Read nothing while the agent writes far more than the limit.
    std::thread::sleep(Duration::from_secs(3));
    // Still connected, this read would never end; dropped, it hits EOF.
    assert!(
        slow.wait_closed(LONG),
        "lagging client was not disconnected"
    );
    let (_fresh, greeting) = holder.connect();
    assert!(greeting.screen.contains("0123456789abcdef"));
}

#[test]
fn a_client_dropped_for_lagging_can_still_send_shutdown() {
    let mut holder = TestHolder::start_with(LONG, 64 * 1024);
    let (mut slow, _) = holder.spawn_bash("exec yes 0123456789abcdef0123456789abcdef");
    std::thread::sleep(Duration::from_secs(3));
    // Sent while (or after) being dropped, then the client goes away.
    slow.send(&HolderRequest::Shutdown).unwrap();
    drop(slow);
    assert_eq!(holder.join(), Stop::Shutdown);
}

#[test]
fn a_replaced_client_cannot_shut_the_holder_down() {
    let holder = TestHolder::start(LONG);
    let (mut old, _) = holder.spawn_bash("sleep 60");
    let (mut new, _) = holder.connect();
    let _ = old.send(&HolderRequest::Shutdown);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!holder.server.as_ref().unwrap().is_finished());
    wait_screen(&mut new, |_| true);
}

#[test]
fn terminal_queries_are_answered_by_the_holder() {
    let holder = TestHolder::start(LONG);
    let (mut client, _) = holder.spawn_bash(
        "stty -icanon -echo; printf '\\033[6n'; IFS= read -r -d R reply; echo \"reply=${reply#?}R\"; sleep 30",
    );
    wait_screen(&mut client, |s| s.contains("reply=[1;1R"));
}

#[test]
fn shutdown_hangs_up_then_kills_an_agent_that_ignores_hup() {
    let mut holder = TestHolder::start(LONG);
    let (mut client, pid) = holder.spawn_bash("trap '' HUP; echo ready; while :; do sleep 1; done");
    wait_screen(&mut client, |s| s.contains("ready"));
    let started = Instant::now();
    client.send(&HolderRequest::Shutdown).unwrap();
    assert_eq!(holder.join(), Stop::Shutdown);
    let took = started.elapsed();
    assert!(took >= Duration::from_secs(5), "no grace period: {took:?}");
    assert!(took < Duration::from_secs(9), "shutdown too slow: {took:?}");
    assert!(processes_in_group(pid).is_empty(), "agent group survived");
}

#[test]
fn shutdown_after_the_agent_exited_kills_its_leftover_children() {
    let mut holder = TestHolder::start(LONG);
    let (mut client, pid) = holder
        .spawn_bash("trap '' HUP; sleep 61 </dev/null >/dev/null 2>&1 & echo started; exit 0");
    wait_exited(&mut client);
    assert!(
        !processes_in_group(pid).is_empty(),
        "leftover child expected"
    );
    assert_eq!(holder.shutdown(), Stop::Shutdown);
    assert!(
        processes_in_group(pid).is_empty(),
        "leftover child survived"
    );
}

#[test]
fn oversized_resize_is_refused_and_the_holder_stays_responsive() {
    let holder = TestHolder::start(LONG);
    let (mut client, _) = holder.spawn_bash("echo ready; sleep 60");
    for (rows, columns) in [(65535, 65535), (0, 80), (24, 1001)] {
        client
            .send(&HolderRequest::Resize {
                data: agend_core::protocol::holder::ResizeData { rows, columns },
            })
            .unwrap();
        assert_eq!(wait_error(&mut client), "invalid_size");
    }
    client
        .send(&HolderRequest::Resize {
            data: agend_core::protocol::holder::ResizeData {
                rows: 1000,
                columns: 1000,
            },
        })
        .unwrap();
    let started = Instant::now();
    let screen = wait_screen(&mut client, |s| s.contains("ready"));
    assert_eq!(screen.split('\n').count(), 1000);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_request_line_over_1_mib_is_refused_and_closed() {
    let holder = TestHolder::start(LONG);
    let huge = format!(
        "{{\"type\":\"operator_terminal_input\",\"data\":{{\"bytes_base64\":\"{}\"}}}}",
        "A".repeat(agend_holder::server::MAX_REQUEST_LINE)
    );
    use std::io::Write;
    // A raw connection: hello, then the huge line.
    let mut raw = std::os::unix::net::UnixStream::connect(&holder.socket).unwrap();
    let hello = serde_json::to_string(&HolderRequest::hello()).unwrap();
    raw.write_all(format!("{hello}\n").as_bytes()).unwrap();
    let _ = raw.write_all(huge.as_bytes());
    let mut reply = String::new();
    use std::io::Read;
    raw.set_read_timeout(Some(LONG)).unwrap();
    let _ = raw.read_to_string(&mut reply);
    assert!(reply.contains("request_too_large"), "{reply}");
    // The holder still serves new clients.
    holder.connect();
}

#[test]
fn hello_must_complete_within_10_s_even_when_trickled() {
    let holder = TestHolder::start(LONG);
    use std::io::{Read, Write};
    let mut raw = std::os::unix::net::UnixStream::connect(&holder.socket).unwrap();
    let started = Instant::now();
    raw.set_read_timeout(Some(Duration::from_millis(900)))
        .unwrap();
    let mut buf = [0u8; 64];
    loop {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "trickle never cut off"
        );
        if raw.write_all(b"{").is_err() {
            break;
        }
        if let Ok(0) = raw.read(&mut buf) {
            break;
        }
    }
    assert!(started.elapsed() >= Duration::from_secs(9));
}

#[test]
fn shutdown_of_a_normal_agent_is_quick_and_leaves_no_process() {
    let mut holder = TestHolder::start(LONG);
    let (mut client, pid) = holder.spawn_bash("echo ready; while :; do sleep 1; done");
    wait_screen(&mut client, |s| s.contains("ready"));
    let started = Instant::now();
    assert_eq!(holder.shutdown(), Stop::Shutdown);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(processes_in_group(pid).is_empty());
}

#[test]
fn idle_safety_net_exits_after_the_agent_ended_and_nobody_connects() {
    let mut holder = TestHolder::start(Duration::from_secs(1));
    let (mut client, _) = holder.spawn_bash("exit 0");
    wait_exited(&mut client);
    drop(client);
    let started = Instant::now();
    assert_eq!(holder.join(), Stop::Idle);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn idle_safety_net_never_fires_while_the_agent_runs() {
    let mut holder = TestHolder::start(Duration::from_secs(1));
    let (client, _) = holder.spawn_bash("sleep 30");
    drop(client);
    std::thread::sleep(Duration::from_secs(3));
    assert!(!holder.server.as_ref().unwrap().is_finished());
    assert_eq!(holder.shutdown(), Stop::Shutdown);
}

#[test]
fn deleting_agend_home_stops_the_holder_and_its_agent() {
    let mut holder = TestHolder::start(Duration::from_secs(2));
    let (mut client, pid) = holder.spawn_bash("echo ready; while :; do sleep 1; done");
    wait_screen(&mut client, |s| s.contains("ready"));
    std::fs::remove_dir(&holder.home).unwrap();
    assert_eq!(holder.join(), Stop::HomeDeleted);
    assert!(processes_in_group(pid).is_empty());
}
