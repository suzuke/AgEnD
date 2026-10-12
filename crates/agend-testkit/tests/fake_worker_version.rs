//! A version probe must not become a second inbox consumer.
#![cfg(unix)]
use agend_testkit::tempdir::TempDir;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[test]
fn native_version_probe_exits_without_cli_calls_or_cursor_writes() {
    let root = TempDir::new("fake-worker-version").unwrap();
    let cli = root.path().join("agend");
    fs::write(
        &cli,
        "#!/bin/sh\n/usr/bin/touch cli-was-called\nprintf '{\"data\":{\"messages\":[]}}\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_fake-worker"))
        .arg("--version")
        .env_clear()
        .env("PATH", root.path())
        .current_dir(root.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let exited = loop {
        if child.try_wait().unwrap().is_some() {
            break true;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child.wait_with_output().unwrap();
    assert!(exited, "--version entered the worker loop");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("agend-fake-worker {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
    assert!(!root.path().join("cli-was-called").exists());
    assert!(!root.path().join(".inbox-cursor").exists());
}
