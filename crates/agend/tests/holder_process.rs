//! Cross-process holder tests with the real `agend` binary (gate 4 P9). D3's
//! promise is that the holder outlives whatever started it, so the launcher
//! and each "daemon boot" here is a separate process: this test binary run
//! again with `HOLDER_PROBE_ROLE` set (see `probe_child`). No two boots are
//! alive at the same time.
//!
//! Safety: every test has its own short `AGEND_HOME` and instance ids unique
//! to this test process. Holders are stopped with `Shutdown`; the fallback
//! only SIGKILLs a pid read from this test's own lock file, and only if > 1.
//! Leftover checks use read-only `ps`.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use agend_core::protocol::holder::{
    ControlKey, HolderRequest, HolderResponse, SendControlKeyData, SpawnData,
};
use agend_holder::client::HolderClient;
use agend_holder::paths::{HolderPaths, is_running};
use agend_testkit::tempdir::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_agend");
const ROLE: &str = "HOLDER_PROBE_ROLE";
const COUNTER: &str = "n=0; i=0; while :; do i=$((i+1)); echo \"counter=$i keys=$n pid=$$\"; \
if IFS= read -r -s -n 1 -t 1 k; then n=$((n+1)); echo \"got $k\"; fi; done";

/// Instance id unique to this test process, short enough for the socket path.
fn instance(tag: &str) -> String {
    format!("{tag}{}", std::process::id() % 100_000)
}

/// Ends the whole test process if a test runs longer than 180 s, so a hang
/// fails in minutes instead of running into the CI job timeout.
struct Watchdog(Option<std::sync::mpsc::Sender<()>>);

impl Watchdog {
    fn arm() -> Self {
        let name = std::thread::current().name().unwrap_or("test").to_string();
        let (done, wait) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                wait.recv_timeout(Duration::from_secs(180))
            {
                eprintln!("watchdog: {name} ran over 180 s; aborting the test process");
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

/// Waits for a child this test spawned, for at most `limit`. On timeout the
/// child (never anything else) is killed and the test fails.
fn wait_within(child: &mut std::process::Child, limit: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() > deadline {
            assert!(child.id() > 1);
            let _ = child.kill();
            let _ = child.wait();
            panic!("child {} did not finish within {limit:?}", child.id());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `Command::output` with a deadline.
fn output_within(cmd: &mut Command, limit: Duration) -> Output {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut buf).map(|_| buf)
    });
    let err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut stderr, &mut buf).map(|_| buf)
    });
    let status = wait_within(&mut child, limit);
    Output {
        status,
        stdout: out.join().unwrap().unwrap_or_default(),
        stderr: err.join().unwrap().unwrap_or_default(),
    }
}

struct Home {
    _watchdog: Watchdog,
    dir: TempDir,
    ids: Vec<String>,
}

impl Home {
    fn new() -> Self {
        Self {
            _watchdog: Watchdog::arm(),
            dir: TempDir::new("hp").unwrap(),
            ids: Vec::new(),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn paths(&self, id: &str) -> HolderPaths {
        HolderPaths::new(self.path(), id)
    }

    /// Runs this test binary as a probe process with `role`.
    fn probe(&self, role: &str, id: &str, extra: &[(&str, &str)]) -> Vec<String> {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "probe_child", "--nocapture", "--test-threads=1"])
            .env(ROLE, role)
            .env("AGEND_HOME", self.path())
            .env("HOLDER_ID", id)
            .stdin(Stdio::null());
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let out = output_within(&mut cmd, Duration::from_secs(60));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "probe {role} failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        stdout
            .lines()
            // libtest may print `test probe_child ... ` on the same line.
            .filter_map(|l| l.split_once("probe: ").map(|(_, rest)| rest))
            .map(str::to_string)
            .collect()
    }

    /// Starts a holder through a launcher process that exits right away.
    fn launch(&mut self, id: &str, script: &str) -> u32 {
        self.ids.push(id.to_string());
        let lines = self.probe("launch", id, &[("HOLDER_SCRIPT", script)]);
        let pid = u32::try_from(field(&lines[0], "pid")).unwrap();
        assert_eq!(is_running(&self.paths(id)).unwrap(), Some(pid));
        pid
    }

    fn holder_cmd(&self, id: &str) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.args(["holder", id])
            .env("AGEND_HOME", self.path())
            .stdin(Stdio::null());
        cmd
    }

    fn shutdown(&self, id: &str) {
        let paths = self.paths(id);
        let (mut client, _) = HolderClient::connect(&paths.socket, Duration::from_millis(50))
            .expect("connect for Shutdown");
        client.send(&HolderRequest::Shutdown).unwrap();
        wait_until("lock released", || is_running(&paths).unwrap().is_none());
        assert!(!paths.socket.exists(), "socket left after Shutdown");
    }

    /// Fails if a holder of this test is still alive.
    fn assert_no_leftovers(&self) {
        let out = output_within(
            Command::new("/bin/ps").args(["-A", "-o", "command="]),
            Duration::from_secs(30),
        );
        let ps = String::from_utf8_lossy(&out.stdout);
        for id in &self.ids {
            assert_eq!(
                is_running(&self.paths(id)).unwrap(),
                None,
                "{id} still locked"
            );
            let needle = format!("agend holder {id}");
            assert!(
                !ps.lines()
                    .any(|l| l.contains(&needle) && l.trim_end().ends_with(id.as_str())),
                "holder process {id} left over"
            );
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        for id in &self.ids {
            let paths = self.paths(id);
            let Ok(Some(pid)) = is_running(&paths) else {
                continue;
            };
            if let Ok((mut client, _)) =
                HolderClient::connect(&paths.socket, Duration::from_millis(50))
            {
                let _ = client.send(&HolderRequest::Shutdown);
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline && matches!(is_running(&paths), Ok(Some(_))) {
                std::thread::sleep(Duration::from_millis(100));
            }
            // Last resort: only the pid from this test's own lock file.
            if matches!(is_running(&paths), Ok(Some(p)) if p == pid) && pid > 1 {
                // SAFETY: kill(2) on a single positive pid > 1 that this
                // test's holder wrote into its lock file.
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            }
        }
    }
}

fn field(line: &str, name: &str) -> u64 {
    line.split(' ')
        .find_map(|f| f.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name}= in {line:?}"))
        .parse()
        .unwrap()
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn last_counter(screen: &str) -> u64 {
    let line = screen
        .lines()
        .rfind(|l| l.starts_with("counter="))
        .unwrap_or_else(|| panic!("no counter on screen:\n{screen}"));
    field(line, "counter")
}

#[test]
fn a_holder_outlives_its_launcher_across_four_separate_boots() {
    let mut home = Home::new();
    let id = instance("b");
    let pid = home.launch(&id, COUNTER);

    let mut boots = Vec::new();
    for n in 1..=4 {
        std::thread::sleep(Duration::from_millis(1200));
        let lines = home.probe("boot", &id, &[("HOLDER_BOOT", &n.to_string())]);
        boots.push(lines[0].clone());
    }
    let pids: Vec<u64> = boots.iter().map(|b| field(b, "pid")).collect();
    let counters: Vec<u64> = boots.iter().map(|b| field(b, "counter")).collect();
    let inodes: Vec<u64> = boots.iter().map(|b| field(b, "inode")).collect();
    assert_eq!(pids, [u64::from(pid); 4], "{boots:?}");
    assert!(
        inodes.iter().all(|i| *i == inodes[0]),
        "socket re-bound: {boots:?}"
    );
    assert!(
        counters.windows(2).all(|w| w[1] > w[0]),
        "counter did not keep growing: {boots:?}"
    );
    // Keys sent by boots 1 and 3 reached the same bash.
    assert_eq!(field(&boots[3], "keys"), 2, "{boots:?}");

    home.shutdown(&id);
    home.assert_no_leftovers();
}

#[test]
fn a_second_holder_for_the_same_instance_exits_1() {
    let mut home = Home::new();
    let id = instance("d");
    let pid = home.launch(&id, "sleep 60");
    let out: Output = output_within(&mut home.holder_cmd(&id), Duration::from_secs(30));
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        format!("holder for {id} already running (pid {pid})")
    );
    assert_eq!(is_running(&home.paths(&id)).unwrap(), Some(pid));
    home.shutdown(&id);
    home.assert_no_leftovers();
}

#[test]
fn the_agent_cannot_stop_its_holder_with_terminating_signals() {
    let mut home = Home::new();
    let id = instance("k");
    let pid = home.launch(
        &id,
        "kill -TERM $PPID; kill -HUP $PPID; kill -INT $PPID; kill -QUIT $PPID; \
         echo signals sent; exec sleep 60",
    );
    let paths = home.paths(&id);
    wait_until("agent sent its signals", || {
        HolderClient::connect(&paths.socket, Duration::from_millis(50))
            .is_ok_and(|(_, g)| g.screen.contains("signals sent"))
    });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(is_running(&paths).unwrap(), Some(pid));
    let (_, greeting) = HolderClient::connect(&paths.socket, Duration::from_millis(50)).unwrap();
    assert_eq!(greeting.exited, None, "agent should still run");
    home.shutdown(&id);
    home.assert_no_leftovers();
}

#[test]
fn idle_safety_net_ends_a_real_holder_whose_agent_exited() {
    let mut home = Home::new();
    let id = instance("i");
    home.ids.push(id.clone());
    let paths = home.paths(&id);
    // Launched directly (not through the probe) to set the idle override.
    let mut holder = home
        .holder_cmd(&id)
        .env(agend_holder::IDLE_EXIT_ENV, "1")
        .spawn()
        .unwrap();
    wait_until("socket", || paths.socket.exists());
    let (mut client, _) = HolderClient::connect(&paths.socket, Duration::from_millis(50)).unwrap();
    client.send(&spawn(&home, &id, "exit 3")).unwrap();
    drop(client);
    let status = wait_within(&mut holder, Duration::from_secs(30));
    assert!(status.success());
    assert!(!paths.socket.exists());
    home.assert_no_leftovers();
}

#[test]
fn a_socket_path_over_100_bytes_is_refused_before_touching_anything() {
    let home = Home::new();
    let deep: PathBuf = home.path().join("d".repeat(60));
    let out = output_within(
        Command::new(BIN)
            .args(["holder", "dev-1"])
            .env("AGEND_HOME", &deep),
        Duration::from_secs(30),
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("over the 100-byte limit"), "{stderr}");
    assert!(stderr.contains("dev-1.sock"), "{stderr}");
    assert!(!deep.exists(), "created directories for a refused holder");
}

/// Verifier r1 scenario s8: a stale lock file (as left by a killed holder)
/// and a tight `is_running` poll while a new holder starts. The probe must
/// never report a pid other than the new holder's (never 0, never the stale
/// one), and the probe's shared lock must never make the start fail.
#[test]
fn lock_probe_never_reports_a_stale_or_zero_pid_while_a_holder_starts() {
    let mut home = Home::new();
    // A pid that certainly exited: a child we ran and reaped.
    let stale = Command::new("/usr/bin/true")
        .spawn()
        .map(|mut c| {
            let _ = c.wait();
            c.id()
        })
        .unwrap();
    for it in 0..200 {
        let id = format!("{}_{it}", instance("q"));
        home.ids.push(id.clone());
        let paths = home.paths(&id);
        std::fs::create_dir_all(&paths.dir).unwrap();
        std::fs::write(&paths.lock, format!("{stale}\n")).unwrap();
        let mut holder = home.holder_cmd(&id).stderr(Stdio::piped()).spawn().unwrap();
        let expected = holder.id();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut after_start = 0;
        while after_start < 20 {
            assert!(
                Instant::now() < deadline,
                "iteration {it}: holder never showed"
            );
            match is_running(&paths) {
                Ok(Some(pid)) => {
                    assert_eq!(pid, expected, "iteration {it}: probe reported pid {pid}");
                    after_start += 1;
                }
                Ok(None) => assert_eq!(after_start, 0, "iteration {it}: lock lost"),
                Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock, "{e}"),
            }
            if let Some(status) = holder.try_wait().unwrap() {
                let mut stderr = String::new();
                let _ = std::io::Read::read_to_string(holder.stderr.as_mut().unwrap(), &mut stderr);
                panic!("iteration {it}: holder exited ({status}): {stderr}");
            }
        }
        // The lock (with the pid) is taken before the socket is bound.
        wait_until("socket bound", || {
            std::os::unix::net::UnixStream::connect(&paths.socket).is_ok()
        });
        home.shutdown(&id);
        assert!(wait_within(&mut holder, Duration::from_secs(30)).success());
    }
    home.assert_no_leftovers();
}

fn spawn(home: &Home, id: &str, script: &str) -> HolderRequest {
    HolderRequest::Spawn {
        data: SpawnData {
            instance_id: id.into(),
            program: "/bin/bash".into(),
            args: vec!["-c".into(), script.into()],
            env: [("PATH".to_string(), "/bin:/usr/bin".to_string())].into(),
            working_directory: home.path().display().to_string(),
        },
    }
}

/// Not a real test: the body of a probe process, run by `Home::probe`.
#[test]
fn probe_child() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    let id = std::env::var("HOLDER_ID").unwrap();
    let home = PathBuf::from(std::env::var("AGEND_HOME").unwrap());
    let paths = HolderPaths::new(&home, &id);
    match role.as_str() {
        "launch" => {
            // Deliberately never waited for: the holder must outlive this
            // launcher, which exits right after Spawn (P9).
            #[allow(clippy::zombie_processes)]
            let holder = Command::new(BIN)
                .args(["holder", &id])
                .stdin(Stdio::null())
                .spawn()
                .unwrap();
            wait_until("holder socket", || {
                HolderClient::connect(&paths.socket, Duration::from_millis(50)).is_ok()
            });
            let (mut client, _) =
                HolderClient::connect(&paths.socket, Duration::from_millis(50)).unwrap();
            let script = std::env::var("HOLDER_SCRIPT").unwrap();
            client
                .send(&HolderRequest::Spawn {
                    data: SpawnData {
                        instance_id: id.clone(),
                        program: "/bin/bash".into(),
                        args: vec!["-c".into(), script],
                        env: [("PATH".to_string(), "/bin:/usr/bin".to_string())].into(),
                        working_directory: home.display().to_string(),
                    },
                })
                .unwrap();
            loop {
                match client.recv(Duration::from_secs(10)).unwrap() {
                    Some(HolderResponse::Spawned { .. }) => break,
                    Some(HolderResponse::Error { data }) => panic!("spawn: {data:?}"),
                    _ => {}
                }
            }
            // Exits without waiting for the holder.
            println!("probe: started pid={}", holder.id());
        }
        "boot" => {
            let n: u32 = std::env::var("HOLDER_BOOT").unwrap().parse().unwrap();
            let pid = is_running(&paths).unwrap().expect("holder running");
            let inode = std::fs::metadata(&paths.socket).unwrap().ino();
            let (mut client, greeting) =
                HolderClient::connect(&paths.socket, Duration::from_millis(100)).unwrap();
            let line = greeting
                .screen
                .lines()
                .rfind(|l| l.starts_with("counter="))
                .expect("counter on screen")
                .to_string();
            if n == 1 || n == 3 {
                client
                    .send(&HolderRequest::SendControlKey {
                        data: SendControlKeyData { key: ControlKey::Y },
                    })
                    .unwrap();
                // Wait until bash counted it, so the key is not lost at exit.
                let keys_before = field(&line, "keys");
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    client.send(&HolderRequest::Snapshot).unwrap();
                    if let Some(HolderResponse::ScreenSnapshot { data }) =
                        client.recv(Duration::from_secs(5)).unwrap()
                        && data.screen.contains(&format!("keys={}", keys_before + 1))
                    {
                        break;
                    }
                    assert!(Instant::now() < deadline, "key not seen");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            println!(
                "probe: boot={n} pid={pid} counter={} keys={} inode={inode}",
                last_counter(&greeting.screen),
                field(&line, "keys")
            );
        }
        other => panic!("unknown role {other}"),
    }
}
