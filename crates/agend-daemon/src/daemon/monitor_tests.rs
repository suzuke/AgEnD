//! Real daemon shutdown with both bounded native observation workers in flight.
use super::*;
use agend_core::{
    model::Backend,
    runtime_records::{Instance, InstanceStatus},
    setup::backend::ImportedBackend,
    traits::HolderLaunch,
};
use agend_testkit::{block_on, tempdir::TempDir};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    process::{Child, Command, Stdio},
    thread,
};

#[test]
#[ignore = "child entry point; launched by the monitor lifecycle test"]
fn child() {
    let home = PathBuf::from(std::env::var_os("AGEND_MONITOR_TEST_HOME").unwrap());
    let origin = std::env::var("AGEND_MONITOR_TEST_ORIGIN").unwrap();
    let store = SqliteStore::open(&home, 0).unwrap();
    let _signals = stop_flag::install().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let exe = std::env::current_exe().unwrap();
    let binding = crate::backend_versions::ExecutableBinding::capture_running(&exe);
    let stopped = runtime
        .block_on(serve(
            home,
            (exe, binding),
            store,
            agend_core::policy::codex_input::CodexInputPolicy::approved(),
            None,
            None,
            TestControl {
                hold_supervisor_for_stop: false,
                registry_origin: Some(origin),
            },
        ))
        .unwrap();
    assert!(matches!(stopped, Stopped::Signal("SIGINT")));
    runtime.shutdown_timeout(Duration::from_secs(1));
}

fn wait(mut predicate: impl FnMut() -> bool, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out: {label}");
        thread::sleep(Duration::from_millis(10));
    }
}

struct Process {
    child: Child,
    home: PathBuf,
}
impl Drop for Process {
    fn drop(&mut self) {
        if thread::panicking() {
            eprintln!(
                "{}",
                fs::read_to_string(self.home.join("child.log")).unwrap_or_default()
            );
        }
        // Release our own workers even when an assertion failed. Never signal a
        // PID after the Child has been reaped; both probe paths are also bounded.
        let _ = fs::write(self.home.join("release"), []);
        if self.child.try_wait().ok().flatten().is_none() {
            unsafe {
                libc::kill(self.child.id() as i32, libc::SIGINT);
            }
            let deadline = Instant::now() + Duration::from_secs(6);
            while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = self.child.kill();
            }
        }
        let _ = self.child.wait();
        if let Some(pid) = fs::read_to_string(self.home.join("probe-pid"))
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
        {
            let deadline = Instant::now() + Duration::from_secs(6);
            while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            // Observe only: never signal a potentially reused PID.
            if unsafe { libc::kill(pid, 0) } == 0 {
                eprintln!("owned probe absence could not be confirmed: {pid}");
            }
        }
    }
}

#[test]
fn daemon_shutdown_waits_for_external_after_registry_finishes() {
    shutdown(true);
}
#[test]
fn daemon_shutdown_waits_for_registry_after_external_finishes() {
    shutdown(false);
}
fn shutdown(registry_first: bool) {
    let dir = TempDir::new("g13-ms").unwrap();
    let home = dir.path().canonicalize().unwrap();
    let script = home.join("backend");
    // The fallback bound also applies if the parent or daemon crashes.
    fs::write(&script, "#!/bin/sh\n[ \"$1\" = --version ] || exit 2\necho $$ > probe-pid\ntouch entered\ni=0\nwhile [ ! -f release ]; do i=$((i+1)); [ $i -lt 200 ] || exit 3; /bin/sleep 0.02; done\nprintf 'fake 1.0\\n'\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    block_on(async {
        let store = SqliteStore::open(&home, 1).unwrap();
        for (id, backend, managed) in [
            ("a-external", Backend::Codex, false),
            ("b-external", Backend::Codex, false),
            ("c-managed", Backend::Claude, true),
            ("d-managed", Backend::Opencode, true),
        ] {
            let instance = Instance {
                id: id.into(),
                backend,
                program: script.to_str().unwrap().into(),
                args: vec![],
                working_directory: home.to_str().unwrap().into(),
                session_id: None,
                status: InstanceStatus::Failed,
                session_started: false,
                agent_pid: None,
                legacy_no_thread: false,
                delivery: "push".into(),
            };
            store.add_instance(&instance).await.unwrap();
            if managed {
                store
                    .prepare_managed_launch(
                        &instance,
                        &HolderLaunch {
                            instance_id: instance.id.clone(),
                            backend,
                            executable: instance.program.clone(),
                            args: vec![],
                            working_directory: instance.working_directory.clone(),
                        },
                        ImportedBackend {
                            format: 1,
                            backend: backend.as_str().into(),
                            version: "1.0".into(),
                            sha256: "a".repeat(64),
                            bytes: 1,
                        },
                        None,
                    )
                    .await
                    .unwrap();
            }
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let log_path = home.join("child.log");
    let log = fs::File::create(&log_path).unwrap();
    let mut process = Process {
        child: Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "daemon::monitor_tests::child",
                "--ignored",
                "--nocapture",
            ])
            .env("AGEND_MONITOR_TEST_HOME", &home)
            .env("AGEND_MONITOR_TEST_ORIGIN", origin)
            .env_remove("AGEND_INSTANCE")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap(),
        home: home.clone(),
    };
    let mut connection = None;
    wait(
        || match listener.accept() {
            Ok((socket, _)) => {
                connection = Some(socket);
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(e) => panic!("accept: {e}"),
        },
        "registry request",
    );
    let mut socket = connection.unwrap();
    // macOS may inherit O_NONBLOCK from the listening socket.
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut reader = BufReader::new(socket.try_clone().unwrap());
    let mut request = String::new();
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        request.push_str(&line);
    }
    assert!(request.starts_with("GET /@anthropic-ai%2fclaude-code/latest "));
    let body = include_bytes!("../../tests/fixtures/backend_registry/claude.json");
    write!(
        socket,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    let middle = body.len() / 2;
    socket.write_all(&body[..middle]).unwrap();
    wait(|| home.join("entered").exists(), "external probe");
    wait(
        || {
            fs::read_to_string(&log_path)
                .unwrap()
                .contains("agend daemon ready:")
        },
        "daemon ready",
    );
    assert!(process.child.try_wait().unwrap().is_none());
    assert_eq!(
        unsafe { libc::kill(process.child.id() as i32, libc::SIGINT) },
        0
    );
    // Both stop requests must be issued before either worker is released.
    // Sequential awaiting would fail here, as would dropping either worker.
    wait(
        || {
            let log = fs::read_to_string(&log_path).unwrap();
            log.contains("Backend registry monitor stop requested")
                && log.contains("Backend version monitor stop requested")
        },
        "both stop requests",
    );
    assert!(process.child.try_wait().unwrap().is_none());
    if registry_first {
        socket.write_all(&body[middle..]).unwrap();
    } else {
        fs::write(home.join("release"), []).unwrap();
    }
    let first = if registry_first {
        "registry"
    } else {
        "version"
    };
    wait(
        || {
            fs::read_to_string(&log_path)
                .unwrap()
                .contains(&format!("Backend {first} monitor stopped"))
        },
        "first worker drained",
    );
    thread::sleep(Duration::from_millis(100));
    assert!(
        process.child.try_wait().unwrap().is_none(),
        "second worker abandoned"
    );
    if registry_first {
        fs::write(home.join("release"), []).unwrap();
    } else {
        socket.write_all(&body[middle..]).unwrap();
    }
    drop(reader);
    drop(socket);
    let mut status = None;
    wait(
        || {
            status = process.child.try_wait().unwrap();
            status.is_some()
        },
        "daemon shutdown",
    );
    assert!(
        status.unwrap().success(),
        "{}",
        fs::read_to_string(&log_path).unwrap()
    );
    assert!(!home.join("run/daemon.sock").exists());
    assert!(
        matches!(listener.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock),
        "next registry backend was queried"
    );
    block_on(async {
        let store = SqliteStore::open(&home, 2).unwrap();
        let external = store
            .system_version_observation("a-external")
            .await
            .unwrap()
            .unwrap();
        assert!(external.completed_ms.is_some());
        assert_eq!(external.latest.unwrap().version_output, "fake 1.0");
        assert!(
            store
                .system_version_observation("b-external")
                .await
                .unwrap()
                .is_none()
        );
        let registry = store
            .registry_observation(Backend::Claude)
            .await
            .unwrap()
            .unwrap();
        assert!(registry.completed_ms.is_some() && registry.latest.is_some());
        assert!(
            store
                .registry_observation(Backend::Opencode)
                .await
                .unwrap()
                .is_none()
        );
    });
    let pid: i32 = fs::read_to_string(home.join("probe-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "owned version probe survived shutdown"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    drop(process);
    drop(listener);
    drop(dir);
    assert!(!home.exists());
}
