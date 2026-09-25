//! `holder_probe`: a small client for poking a running holder by hand, and the
//! gate 4 acceptance demo (`holder_probe demo`, run by `cargo xtask accept
//! holder`). Not a production command (gate 4 P1).
//!
//! Commands (all need `AGEND_HOME`; `start` also finds `agend` through
//! `AGEND_BIN`, default: `target/<profile>/agend` next to this example):
//!   start <id> [--env K=V]... -- <program> [args...]
//!   snapshot <id>
//!   key <id> <key>            (esc, enter, y, n, digit1, ...)
//!   shutdown <id>
//!   watch <id>                (prints counter lines until killed; demo use)
//!   boot <id> <n> [prev-counter first-pid first-inode]   (demo use)
//!   demo                      (AGEND_HOLDER_DEMO_KEEP=1: set up step 9 only)
//!
//! Safety: this program only ever signals a process it spawned itself (the
//! `watch` client in the demo). Holders are stopped with `Shutdown`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use agend_core::protocol::holder::{
    ControlKey, HolderRequest, HolderResponse, SendControlKeyData, SpawnData,
};
use agend_holder::client::{HolderClient, decode_bytes};
use agend_holder::paths::{HolderPaths, is_running};

type Result<T> = std::result::Result<T, String>;

const COUNTER: &str = "n=0; i=0; while :; do i=$((i+1)); echo \"counter=$i keys=$n pid=$$\"; \
if IFS= read -r -s -n 1 -t 1 k; then n=$((n+1)); echo \"got $k\"; fi; done";
const AGENT_KILLS_PARENT: &str = "echo \"agent ran: kill -TERM $PPID\"; kill -TERM $PPID; \
kill -HUP $PPID; kill -INT $PPID; kill -QUIT $PPID; echo \"agent: signals sent\"; exec sleep 3600";
const AGENT_PATH: &str = "PATH=/bin:/usr/bin";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("start") => start(&args[1..]),
        Some("snapshot") => arg(&args, 1).and_then(|id| snapshot(&id)),
        Some("key") => arg(&args, 1).and_then(|id| key(&id, &arg(&args, 2)?)),
        Some("shutdown") => arg(&args, 1).and_then(|id| shutdown(&id)),
        Some("watch") => arg(&args, 1).and_then(|id| watch(&id)),
        Some("boot") => boot(&args[1..]),
        Some("demo") => demo(),
        _ => Err("usage: holder_probe start|snapshot|key|shutdown|watch|boot|demo ...".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            println!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn arg(args: &[String], i: usize) -> Result<String> {
    args.get(i)
        .cloned()
        .ok_or_else(|| format!("missing argument {i}"))
}

fn paths(id: &str) -> Result<HolderPaths> {
    HolderPaths::from_env(id)
}

fn agend_bin() -> Result<PathBuf> {
    if let Some(bin) = std::env::var_os("AGEND_BIN") {
        return Ok(PathBuf::from(bin));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // target/<profile>/examples/holder_probe -> target/<profile>/agend
    let bin = exe
        .parent()
        .and_then(Path::parent)
        .map(|dir| dir.join("agend"))
        .ok_or("cannot locate agend; set AGEND_BIN")?;
    if bin.is_file() {
        Ok(bin)
    } else {
        Err(format!(
            "{} not found; run `cargo build -p agend` or set AGEND_BIN",
            bin.display()
        ))
    }
}

fn connect(id: &str) -> Result<(HolderClient, agend_holder::client::Greeting)> {
    let paths = paths(id)?;
    HolderClient::connect(&paths.socket, Duration::from_millis(300))
        .map_err(|e| format!("connect failed: {e}"))
}

/// Last `counter=<i> keys=<n> pid=<p>` line on a screen.
fn counter(screen: &str) -> Option<(u64, u64, u32)> {
    let line = screen.lines().rfind(|l| l.starts_with("counter="))?;
    let mut fields = line.split(' ').map(|f| f.split('=').nth(1));
    Some((
        fields.next()??.parse().ok()?,
        fields.next()??.parse().ok()?,
        fields.next()??.parse().ok()?,
    ))
}

fn fresh_screen(client: &mut HolderClient) -> Result<String> {
    client
        .send(&HolderRequest::Snapshot)
        .map_err(|e| e.to_string())?;
    loop {
        match client.recv(Duration::from_secs(5)) {
            Ok(Some(HolderResponse::ScreenSnapshot { data })) => return Ok(data.screen),
            Ok(Some(_)) => {}
            other => return Err(format!("no snapshot: {other:?}")),
        }
    }
}

fn wait_screen(
    client: &mut HolderClient,
    what: &str,
    done: impl Fn(&str) -> bool,
) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let screen = fresh_screen(client)?;
        if done(&screen) {
            return Ok(screen);
        }
        if Instant::now() > deadline {
            return Err(format!("timed out waiting for {what}; screen:\n{screen}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ---- probe commands --------------------------------------------------------

fn start(args: &[String]) -> Result<()> {
    let id = arg(args, 0)?;
    let split = args
        .iter()
        .position(|a| a == "--")
        .ok_or("usage: start <id> [--env K=V]... -- <program> [args...]")?;
    let mut env = BTreeMap::new();
    let mut options = args[1..split].iter();
    while let Some(option) = options.next() {
        let pair = match option.as_str() {
            "--env" => options.next().ok_or("--env needs K=V")?,
            other => return Err(format!("unknown option {other}")),
        };
        let (k, v) = pair.split_once('=').ok_or("--env needs K=V")?;
        env.insert(k.to_string(), v.to_string());
    }
    let command = &args[split + 1..];
    let program = command.first().ok_or("missing program")?.clone();
    let paths = paths(&id)?;

    let mut holder = Command::new(agend_bin()?)
        .args(["holder", &id])
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run agend: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut client, _) = loop {
        if let Some(status) = holder.try_wait().map_err(|e| e.to_string())? {
            return Err(format!(
                "holder exited: exit={}",
                status.code().unwrap_or(-1)
            ));
        }
        if let Ok(connected) = HolderClient::connect(&paths.socket, Duration::from_millis(100)) {
            break connected;
        }
        if Instant::now() > deadline {
            return Err("holder did not open its socket".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    client
        .send(&HolderRequest::Spawn {
            data: SpawnData {
                instance_id: id.clone(),
                program,
                args: command[1..].to_vec(),
                env,
                working_directory: cwd.display().to_string(),
            },
        })
        .map_err(|e| e.to_string())?;
    loop {
        match client.recv(Duration::from_secs(10)) {
            Ok(Some(HolderResponse::Spawned { data })) => {
                println!(
                    "holder started: pid {} agent pid {}",
                    holder.id(),
                    data.process_id.unwrap_or(0)
                );
                // Exit without waiting: the holder must outlive its launcher.
                return Ok(());
            }
            Ok(Some(HolderResponse::Error { data })) => {
                return Err(format!("spawn failed: {} {}", data.code, data.message));
            }
            Ok(Some(_)) => {}
            other => return Err(format!("no Spawned reply: {other:?}")),
        }
    }
}

fn snapshot(id: &str) -> Result<()> {
    let (_, greeting) = connect(id)?;
    println!("{}", greeting.screen.trim_end());
    if let Some(exited) = greeting.exited {
        println!("{}", describe_exit(&exited));
    }
    Ok(())
}

fn describe_exit(exited: &agend_core::protocol::holder::ExitedData) -> String {
    match (&exited.code, &exited.signal) {
        (Some(code), _) => format!("exited code={code}"),
        (None, Some(signal)) => format!("exited signal={signal}"),
        (None, None) => "exited (status unknown)".into(),
    }
}

fn key(id: &str, name: &str) -> Result<()> {
    let key: ControlKey = serde_json::from_value(serde_json::Value::String(name.into()))
        .map_err(|e| e.to_string())?;
    let (mut client, _) = connect(id)?;
    println!("{}", send_key(&mut client, key, name)?);
    Ok(())
}

/// Sends a key; waits briefly for an `Error` (success has no reply).
fn send_key(client: &mut HolderClient, key: ControlKey, name: &str) -> Result<String> {
    client
        .send(&HolderRequest::SendControlKey {
            data: SendControlKeyData { key },
        })
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        match client.recv(deadline.saturating_duration_since(Instant::now())) {
            Ok(Some(HolderResponse::Error { data })) => {
                return Ok(format!("sent {name} -> error {}", data.code));
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(format!("sent {name}"))
}

fn shutdown(id: &str) -> Result<()> {
    for line in stop(id)? {
        println!("{line}");
    }
    Ok(())
}

/// `Shutdown`, then confirm the socket is gone and the lock is free.
fn stop(id: &str) -> Result<Vec<String>> {
    let paths = paths(id)?;
    let pid = is_running(&paths).map_err(|e| e.to_string())?;
    let (mut client, _) = connect(id)?;
    client
        .send(&HolderRequest::Shutdown)
        .map_err(|e| e.to_string())?;
    let mut lines = vec!["shutdown".to_string()];
    let deadline = Instant::now() + Duration::from_secs(15);
    while is_running(&paths).map_err(|e| e.to_string())?.is_some() {
        if Instant::now() > deadline {
            return Err(format!("{id}: lock still held after Shutdown"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if paths.socket.exists() {
        return Err(format!("{id}: socket still exists"));
    }
    lines.push("socket gone".into());
    lines.push("lock free".into());
    if let Some(pid) = pid {
        if process_exists(pid) {
            return Err(format!("{id}: holder pid {pid} still exists"));
        }
        lines.push(format!("holder gone (pid {pid})"));
    }
    Ok(lines)
}

/// Read-only check with `ps`; never signals anything.
fn process_exists(pid: u32) -> bool {
    Command::new("/bin/ps")
        .args(["-o", "pid=", "-p", &pid.to_string()])
        .output()
        .map(|out| !String::from_utf8_lossy(&out.stdout).trim().is_empty())
        .unwrap_or(false)
}

fn parent_of(pid: u32) -> Option<String> {
    let out = Command::new("/bin/ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn watch(id: &str) -> Result<()> {
    use std::io::Write;
    let mut out = std::io::stdout();
    let (mut client, greeting) = connect(id)?;
    if let Some(line) = greeting.screen.lines().rfind(|l| counter(l).is_some()) {
        writeln!(out, "{line}").map_err(|e| e.to_string())?;
    }
    let mut text = String::new();
    loop {
        match client.recv(Duration::from_secs(5)) {
            Ok(Some(HolderResponse::PtyBytes { data })) => {
                let bytes = decode_bytes(&data.bytes_base64).map_err(|e| e.to_string())?;
                text.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(end) = text.find('\n') {
                    let line: String = text.drain(..=end).collect();
                    let line = line.trim_end();
                    if counter(line).is_some() {
                        // A closed pipe means the reader is gone: stop quietly.
                        writeln!(out, "{line}").map_err(|e| e.to_string())?;
                    }
                }
            }
            Ok(_) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn boot(args: &[String]) -> Result<()> {
    let id = arg(args, 0)?;
    let n: u32 = arg(args, 1)?.parse().map_err(|_| "boot number")?;
    let paths = paths(&id)?;
    let pid = is_running(&paths)
        .map_err(|e| e.to_string())?
        .ok_or("holder not running")?;
    let inode = std::fs::metadata(&paths.socket)
        .map_err(|e| e.to_string())?
        .ino();
    let (mut client, greeting) = connect(&id)?;
    let (count, _, _) = counter(&greeting.screen).ok_or("no counter on screen")?;
    if n == 3 {
        let prev: u64 = arg(args, 2)?.parse().map_err(|_| "prev counter")?;
        if count <= prev {
            return Err(format!("boot 3: counter {count} did not grow past {prev}"));
        }
    }
    if n == 1 || n == 3 {
        send_key(&mut client, ControlKey::Y, "y")?;
    }
    println!("boot {n}: pid {pid} counter={count}");
    if n == 4 {
        let first_pid = arg(args, 3)?;
        let first_inode = arg(args, 4)?;
        if first_pid != pid.to_string() || first_inode != inode.to_string() {
            return Err(format!(
                "boot 4: pid {pid} socket inode {inode} differ from boot 1 ({first_pid}, {first_inode})"
            ));
        }
        println!("boot 4: same pid and socket (inode {inode}) as boot 1");
    }
    println!("inode {inode}");
    Ok(())
}

// ---- demo ------------------------------------------------------------------

struct Demo {
    home: PathBuf,
    agend: PathBuf,
    probe: PathBuf,
    started: Vec<String>,
}

fn demo() -> Result<()> {
    let home = std::env::temp_dir().join(format!("agend-demo-{}", std::process::id()));
    std::fs::create_dir(&home).map_err(|e| format!("create {}: {e}", home.display()))?;
    let mut demo = Demo {
        home,
        agend: agend_bin()?,
        probe: std::env::current_exe().map_err(|e| e.to_string())?,
        started: Vec::new(),
    };
    // SAFETY: single-threaded here; children and our own lookups read it.
    unsafe { std::env::set_var("AGEND_HOME", &demo.home) };

    if std::env::var_os("AGEND_HOLDER_DEMO_KEEP").is_some() {
        println!("== keep (step 9: paste these in a NEW terminal tab)");
        println!("export AGEND_HOME={}", demo.home.display());
        println!("export AGEND_BIN={}", demo.agend.display());
        println!("PROBE={}", demo.probe.display());
        println!(
            "$PROBE start demo-2 --env {AGENT_PATH} -- /bin/bash -c '{COUNTER}' agend-demo-marker"
        );
        return Ok(());
    }

    let result = demo.run();
    let cleanup = demo.cleanup();
    let _ = std::fs::remove_dir_all(&demo.home);
    result.and(cleanup)
}

impl Demo {
    fn probe(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.probe);
        cmd.args(args)
            .env("AGEND_HOME", &self.home)
            .env("AGEND_BIN", &self.agend);
        cmd
    }

    /// Runs a probe command to completion; returns stdout lines.
    fn run_probe(&self, args: &[&str]) -> Result<Vec<String>> {
        let out = self
            .probe(args)
            .output()
            .map_err(|e| format!("run probe: {e}"))?;
        let lines: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        if out.status.success() {
            Ok(lines)
        } else {
            Err(format!("probe {args:?} failed: {}", lines.join(" / ")))
        }
    }

    fn launch(&mut self, id: &str, script: &str) -> Result<Vec<String>> {
        self.started.push(id.to_string());
        self.run_probe(&[
            "start",
            id,
            "--env",
            AGENT_PATH,
            "--",
            "/bin/bash",
            "-c",
            script,
        ])
    }

    fn holder_pid(&self, id: &str) -> Result<u32> {
        is_running(&paths(id)?)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("holder {id} is not running"))
    }

    fn run(&mut self) -> Result<()> {
        self.detach()?;
        self.reconnect()?;
        self.keys()?;
        self.duplicate()?;
        self.agent_kills_parent()?;
        self.lifecycle()?;
        self.exit()?;
        self.shutdown()?;
        println!("holder demo: all sections passed");
        Ok(())
    }

    fn detach(&mut self) -> Result<()> {
        println!("== detach");
        let lines = self.launch("demo-1", COUNTER)?;
        for line in &lines {
            println!("{line}");
        }
        println!("launcher exited (status 0)");
        let pid = self.holder_pid("demo-1")?;
        println!("holder alive: pid {pid}");
        println!(
            "holder parent pid: {} (the launcher is gone)",
            parent_of(pid).unwrap_or_default()
        );
        let (mut client, _) = connect("demo-1")?;
        let first = wait_screen(&mut client, "a counter", |s| counter(s).is_some())?;
        std::thread::sleep(Duration::from_secs(2));
        let later = fresh_screen(&mut client)?;
        let (a, b) = (
            counter(&first).unwrap().0,
            counter(&later).map_or(0, |c| c.0),
        );
        if b <= a {
            return Err(format!("counter did not increase: {a} -> {b}"));
        }
        println!("counter={a} -> counter={b} (still increasing)");
        Ok(())
    }

    fn reconnect(&mut self) -> Result<()> {
        println!("== reconnect");
        let mut watcher = self
            .probe(&["watch", "demo-1"])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stdout = watcher.stdout.take().ok_or("no watcher stdout")?;
        let mut seen = None;
        for line in BufReader::new(stdout).lines().take(3) {
            seen = Some(line.map_err(|e| e.to_string())?);
        }
        let seen = seen.ok_or("watcher printed nothing")?;
        let (a, _, pid_a) = counter(&seen).ok_or("watcher line has no counter")?;
        println!("before kill: counter={a} bash pid {pid_a}");
        // Only the probe client this demo spawned; std's kill is SIGKILL to that pid.
        assert!(watcher.id() > 1);
        watcher.kill().map_err(|e| e.to_string())?;
        let _ = watcher.wait();
        println!("probe client killed (pid {})", watcher.id());
        std::thread::sleep(Duration::from_secs(2));
        let (_, greeting) = connect("demo-1")?;
        let (b, _, pid_b) = counter(&greeting.screen).ok_or("no counter after reconnect")?;
        println!("after reconnect: counter={b} bash pid {pid_b}");
        if b <= a || pid_a != pid_b {
            return Err("reconnect did not see the same, still-running bash".into());
        }
        println!("counter grew ({a} -> {b}) and bash pid is the same: ok");
        Ok(())
    }

    fn keys(&mut self) -> Result<()> {
        println!("== keys");
        let (mut client, _) = connect("demo-1")?;
        println!("{}", send_key(&mut client, ControlKey::Y, "y")?);
        wait_screen(&mut client, "got y", |s| s.contains("got y"))?;
        println!("screen shows: got y");
        let keys = |s: &str| counter(s).map(|c| c.1);
        let before = wait_screen(&mut client, "keys counted", |s| keys(s) >= Some(1))?;
        let before = keys(&before).unwrap();
        println!("{}", send_key(&mut client, ControlKey::Unknown, "unknown")?);
        std::thread::sleep(Duration::from_millis(2500));
        let after = keys(&fresh_screen(&mut client)?).unwrap_or(before);
        println!("pty bytes written: {}", after - before);
        if after != before {
            return Err("unknown key reached the agent".into());
        }
        Ok(())
    }

    fn duplicate(&mut self) -> Result<()> {
        println!("== duplicate");
        let out = Command::new(&self.agend)
            .args(["holder", "demo-1"])
            .env("AGEND_HOME", &self.home)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        println!("{}", String::from_utf8_lossy(&out.stderr).trim());
        println!("exit={}", out.status.code().unwrap_or(-1));
        if out.status.code() != Some(1) {
            return Err("duplicate holder was not refused".into());
        }
        println!(
            "first holder still alive: pid {}",
            self.holder_pid("demo-1")?
        );
        Ok(())
    }

    fn agent_kills_parent(&mut self) -> Result<()> {
        println!("== agent-kills-parent");
        self.launch("demo-k", AGENT_KILLS_PARENT)?;
        let pid = self.holder_pid("demo-k")?;
        let (mut client, _) = connect("demo-k")?;
        let screen = wait_screen(&mut client, "signals sent", |s| s.contains("signals sent"))?;
        for line in screen.lines().filter(|l| l.starts_with("agent")) {
            println!("{line}");
        }
        std::thread::sleep(Duration::from_secs(1));
        drop(client);
        let after = self.holder_pid("demo-k")?;
        connect("demo-k")?;
        if after != pid {
            return Err("holder changed after TERM/HUP/INT/QUIT".into());
        }
        println!("holder alive: pid {pid}");
        stop("demo-k")?;
        Ok(())
    }

    fn lifecycle(&mut self) -> Result<()> {
        println!("== lifecycle");
        let mut prev = String::new();
        let mut first = (String::new(), String::new());
        for n in 1..=4 {
            let n_text = n.to_string();
            let mut args = vec!["boot", "demo-1", n_text.as_str()];
            if n >= 3 {
                args.extend([prev.as_str(), first.0.as_str(), first.1.as_str()]);
            }
            // Each boot is its own process; it has exited before the next starts.
            let lines = self.run_probe(&args)?;
            for line in lines.iter().filter(|l| l.starts_with("boot")) {
                println!("{line}");
            }
            let head = lines.first().ok_or("boot printed nothing")?;
            prev = head.rsplit('=').next().unwrap_or_default().to_string();
            if n == 1 {
                let pid = head.split(' ').nth(3).unwrap_or_default().to_string();
                let inode = lines
                    .iter()
                    .find_map(|l| l.strip_prefix("inode "))
                    .unwrap_or_default()
                    .to_string();
                first = (pid, inode);
            }
            std::thread::sleep(Duration::from_millis(1500));
        }
        Ok(())
    }

    fn exit(&mut self) -> Result<()> {
        println!("== exit");
        self.launch("demo-x", "echo bye; exit 7")?;
        let wait_exit = |id: &str| -> Result<String> {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let (_, greeting) = connect(id)?;
                if let Some(exited) = greeting.exited {
                    return Ok(describe_exit(&exited));
                }
                if Instant::now() > deadline {
                    return Err(format!("{id}: no Exited"));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        };
        let first = wait_exit("demo-x")?;
        println!("{first}");
        std::thread::sleep(Duration::from_secs(1));
        let again = wait_exit("demo-x")?;
        println!("after reconnect: {again}");
        if first != "exited code=7" || again != first {
            return Err("exit code not kept".into());
        }
        self.launch("demo-s", "kill -KILL $$")?;
        let signal = wait_exit("demo-s")?;
        println!("killed agent: {signal}");
        if signal != "exited signal=SIGKILL" {
            return Err("signal not reported".into());
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        println!("== shutdown");
        for line in stop("demo-1")? {
            println!("{line}");
        }
        stop("demo-x")?;
        stop("demo-s")?;
        Ok(())
    }

    /// Stops (with `Shutdown`) any holder this demo started that still runs,
    /// then fails if one is left.
    fn cleanup(&self) -> Result<()> {
        let mut left = Vec::new();
        for id in &self.started {
            let paths = paths(id)?;
            if matches!(is_running(&paths), Ok(Some(_))) {
                let _ = stop(id);
                if let Ok(Some(pid)) = is_running(&paths) {
                    left.push(format!("{id} (pid {pid})"));
                }
            }
        }
        if left.is_empty() {
            Ok(())
        } else {
            Err(format!("holders left running: {}", left.join(", ")))
        }
    }
}
