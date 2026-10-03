//! Real Runner, LocalForge and cold checks sandbox adapters.
use agend_core::pipeline::task::Task;
use agend_core::traits::{Runner, Store, StoredEvent, TaskProgress};
use agend_daemon::{
    forge::local::{LocalError, LocalForge},
    git::Git,
    runner::ProcessRunner,
    store::SqliteStore,
};
use agend_testkit::{
    block_on,
    contract::{forge, runner},
    tempdir::TempDir,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}
fn git(repo: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .current_dir(repo)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().into()
}
fn repository(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-b", "main"]);
    git(path, &["config", "user.name", "Pipeline adapter"]);
    git(path, &["config", "user.email", "test@example.invalid"]);
    std::fs::write(path.join("README"), "base\n").unwrap();
    git(path, &["add", "README"]);
    git(path, &["commit", "-m", "base"]);
}
struct RunFixture {
    _dir: TempDir,
    cwd: String,
    runner: ProcessRunner,
}
impl runner::RunnerFixture for RunFixture {
    type Runner = ProcessRunner;
    type Error = std::io::Error;
    fn runner(&self) -> &ProcessRunner {
        &self.runner
    }
    fn working_directory(&self) -> &str {
        &self.cwd
    }
}
#[test]
fn real_runner_satisfies_run_1_to_9() {
    let rt = runtime();
    let _guard = rt.enter();
    let report = runner::run("ProcessRunner", || {
        let dir = TempDir::new("g10-runner").unwrap();
        RunFixture {
            cwd: dir.path().canonicalize().unwrap().display().to_string(),
            runner: Git::discover(dir.path()).unwrap().runner,
            _dir: dir,
        }
    });
    assert!(report.all_passed(), "{report:#?}");
}
struct ForgeFixture {
    _dir: TempDir,
    forge: LocalForge,
    seq: AtomicUsize,
}
impl ForgeFixture {
    fn new() -> Self {
        let dir = TempDir::new("g10-forge").unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let repo = dir.path().join("repo");
        repository(&repo);
        let store = Arc::new(SqliteStore::open(&home, 0).unwrap());
        Self {
            forge: LocalForge {
                git: Git::discover(&home).unwrap(),
                repo,
                store,
                expected_main: None,
            },
            _dir: dir,
            seq: AtomicUsize::new(0),
        }
    }
}
impl forge::ForgeFixture for ForgeFixture {
    type Forge = LocalForge;
    type Error = LocalError;
    fn forge(&self) -> &LocalForge {
        &self.forge
    }
    fn commit_to(&self, branch: &str) -> String {
        let id = agend_core::model::task_id_of_branch(branch).unwrap();
        if block_on(self.forge.store.load_task(id)).unwrap().is_none() {
            let task = Task::new(id, "Contract task", "general", "code", 1);
            block_on(self.forge.store.create_task(&task)).unwrap();
            let progress = TaskProgress {
                pipeline: "{}".into(),
                stage_entered_at_unix_ms: 0,
                merge_intent: None,
                block_reason: None,
            };
            let event = StoredEvent {
                id: "initial".into(),
                occurred_at_unix_ms: 0,
                kind: "contract".into(),
                detail: "".into(),
            };
            block_on(self.forge.store.advance_task(&task, 1, &progress, &event)).unwrap();
        }
        let repo = &self.forge.repo;
        let exists = Command::new("git")
            .current_dir(repo)
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .status()
            .unwrap()
            .success();
        if exists {
            git(repo, &["checkout", branch]);
        } else {
            git(repo, &["checkout", "-b", branch, "main"]);
        }
        let name = format!("file-{}", self.seq.fetch_add(1, Ordering::Relaxed));
        std::fs::write(repo.join(&name), "contract\n").unwrap();
        git(repo, &["add", &name]);
        git(repo, &["commit", "-m", &name]);
        let head = git(repo, &["rev-parse", "HEAD"]);
        git(repo, &["checkout", "main"]);
        head
    }
    fn base_head(&self) -> String {
        git(&self.forge.repo, &["rev-parse", "main"])
    }
    fn base_contains(&self, commit: &str) -> bool {
        Command::new("git")
            .current_dir(&self.forge.repo)
            .args(["merge-base", "--is-ancestor", commit, "main"])
            .status()
            .unwrap()
            .success()
    }
}
#[test]
fn real_local_forge_satisfies_frg_1_to_10() {
    let rt = runtime();
    let _guard = rt.enter();
    let report = forge::run("LocalForge", ForgeFixture::new);
    assert!(report.all_passed(), "{report:#?}");
}
struct CheckLab {
    _dir: TempDir,
    home: PathBuf,
    repo: PathBuf,
    git: Git,
    head: String,
}
impl CheckLab {
    fn new() -> Self {
        let dir = TempDir::new("g10-checks").unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = root.join("home");
        std::fs::create_dir(&home).unwrap();
        let repo = root.join("repo");
        repository(&repo);
        let head = git(&repo, &["rev-parse", "main"]);
        let git = Git::discover(&home).unwrap();
        Self {
            _dir: dir,
            home,
            repo,
            git,
            head,
        }
    }
    async fn run(&self, command: &str) -> agend_core::traits::CommandOutput {
        agend_daemon::checks::run(
            &self.git,
            &self.home,
            &self.repo,
            "t-1/checks/1",
            &self.head,
            command,
            10_000,
        )
        .await
        .unwrap()
    }
}
#[test]
fn checks_deny_metadata_cache_and_outside_writes_and_reject_a_fifo_marker() {
    runtime().block_on(async {
        let lab = CheckLab::new();
        for command in [
            "printf bad >> .git",
            "git config --local alias.poison '!touch /tmp/poison'",
            "git update-ref refs/heads/poison HEAD",
            "mkdir -p \"$HOME/.cache/agend-escape\"", // the user cache is read-only
            "rm \"$TMPDIR/.agend-sandbox-started\"; mkfifo \"$TMPDIR/.agend-sandbox-started\"",
        ] {
            let output = lab.run(command).await;
            assert_ne!(output.exit_code, Some(0), "escape succeeded: {command}");
        }
        let outside = lab.home.join("outside");
        let output = lab
            .run(&format!(
                "ln -s {} escape; touch escape",
                agend_daemon::runner::quote(&outside.display().to_string())
            ))
            .await;
        assert_ne!(output.exit_code, Some(0));
        assert!(!outside.exists());
        assert_eq!(
            std::fs::read_dir(lab.home.join("checks")).unwrap().count(),
            0
        );
    });
}

#[test]
fn checks_allow_tcp_but_cannot_connect_to_the_daemon_unix_socket() {
    runtime().block_on(async {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::os::unix::net::UnixListener;
        let lab = CheckLab::new();
        std::fs::create_dir_all(lab.home.join("run")).unwrap();
        // macOS limits Unix socket paths to 104 bytes, regardless of TMPDIR.
        struct Alias(PathBuf);
        impl Drop for Alias { fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); } }
        let alias = Alias(PathBuf::from(format!("/tmp/g10-s-{}", agend_daemon::store::instances::new_session_id().unwrap())));
        std::os::unix::fs::symlink(&lab.home, &alias.0).unwrap();
        let socket = alias.0.join("run/daemon.sock");
        let _unix = UnixListener::bind(&socket).unwrap();
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        tcp.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(8);
            loop {
                match tcp.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
                        let mut bytes = [0; 4];
                        stream.read_exact(&mut bytes).unwrap();
                        assert_eq!(&bytes, b"ping");
                        stream.write_all(b"pong").unwrap();
                        return true;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < until => std::thread::sleep(std::time::Duration::from_millis(10)),
                    Err(_) => return false,
                }
            }
        });
        let script = format!("import socket; s=socket.create_connection(('127.0.0.1',{port}),3); s.sendall(b'ping'); assert s.recv(4)==b'pong'");
        let result = lab.run(&format!("python3 -c {}", agend_daemon::runner::quote(&script))).await;
        assert!(server.join().unwrap(), "sandbox command never connected over TCP");
        assert_eq!(result.exit_code, Some(0), "{}", String::from_utf8_lossy(&result.stderr));
        let script = format!("import socket; s=socket.socket(socket.AF_UNIX); s.settimeout(2); s.connect({:?})", socket.display().to_string());
        let result = lab.run(&format!("python3 -c {}", agend_daemon::runner::quote(&script))).await;
        assert_ne!(result.exit_code, Some(0), "checks connected to the daemon socket");
    });
}

#[test]
fn runner_caps_large_output_and_times_out_with_no_exit_code() {
    runtime().block_on(async {
        let dir = TempDir::new("g10-output-cap").unwrap();
        let runner = Git::discover(dir.path()).unwrap().runner;
        let output = runner
            .run(
                "dd if=/dev/zero bs=1048576 count=12; sleep 10",
                &dir.path().display().to_string(),
                500,
            )
            .await
            .unwrap();
        assert!(output.timed_out);
        assert_eq!(output.exit_code, None);
        assert!(output.stdout.len() <= 5 * 1024 * 1024 + 100);
        assert!(output.stdout.len() >= 5 * 1024 * 1024);
    });
}

#[test]
fn runner_parent_crash_probe() {
    let Some(root) = std::env::var_os("AGEND_RUNNER_PARENT_CRASH_PROBE") else {
        return;
    };
    let root = PathBuf::from(root);
    runtime().block_on(async {
        let runner = Git::discover(&root).unwrap().runner;
        runner
            .run(
                "(sleep 2; touch orphan-survived) & touch runner-ready; sleep 30",
                &root.display().to_string(),
                60_000,
            )
            .await
            .unwrap();
    });
}

#[test]
fn a_killed_runner_parent_does_not_leave_command_children_running() {
    let dir = TempDir::new("g10-parent-crash").unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "runner_parent_crash_probe", "--nocapture"])
        .env("AGEND_RUNNER_PARENT_CRASH_PROBE", dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !dir.path().join("runner-ready").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "runner probe never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2500));
    assert!(
        !dir.path().join("orphan-survived").exists(),
        "a command child survived its runner's SIGKILL"
    );
}
#[test]
fn checks_allow_build_outputs_and_use_cold_caches() {
    runtime().block_on(async {
        let lab = CheckLab::new();
        let script = "test -z \"${AGEND_HOME+x}\" && test -z \"${AGEND_INSTANCE+x}\" && test ! -e \"$CARGO_HOME/poison\" && touch \"$CARGO_HOME/poison\" && mkdir build && printf ok > build/output";
        assert_eq!(lab.run(script).await.exit_code,Some(0));
        assert_eq!(lab.run(script).await.exit_code,Some(0));
    });
}
#[test]
fn runner_stops_background_children_after_success_as_well_as_timeout() {
    runtime().block_on(async {
        let dir = TempDir::new("g10-child").unwrap();
        let runner = Git::discover(dir.path()).unwrap().runner;
        let output = runner
            .run(
                "(sleep 1; touch escaped-child) & exit 0",
                &dir.path().display().to_string(),
                5000,
            )
            .await
            .unwrap();
        assert_eq!(output.exit_code, Some(0));
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert!(!dir.path().join("escaped-child").exists());
    });
}

#[test]
fn sandbox_runs_cargo_test_and_npm_test_on_committed_sources() {
    runtime().block_on(async {
        let mut lab = CheckLab::new();
        std::fs::create_dir(lab.repo.join("src")).unwrap();
        std::fs::write(lab.repo.join("Cargo.toml"), "[package]\nname='sandbox-smoke'\nversion='0.1.0'\nedition='2024'\n").unwrap();
        std::fs::write(lab.repo.join("rust-toolchain.toml"), "[toolchain]\nchannel='1.96.0'\n").unwrap();
        std::fs::write(lab.repo.join("src/lib.rs"), "#[test] fn compiled_in_sandbox() { assert_eq!(2 + 2, 4); }\n").unwrap();
        std::fs::write(lab.repo.join("package.json"), r#"{"name":"sandbox-smoke","private":true,"scripts":{"test":"node -e 'require(\"node:assert\").strictEqual(2+2,4)'"}}"#).unwrap();
        git(&lab.repo, &["add", "."]); git(&lab.repo, &["commit", "-m", "Add real build smoke tests"]);
        lab.head = git(&lab.repo, &["rev-parse", "HEAD"]);
        for command in ["cargo test --offline", "npm test --offline"] {
            let output = lab.run(command).await;
            assert_eq!(output.exit_code,Some(0), "{command}: {}", String::from_utf8_lossy(&output.stderr));
        }
    });
}
