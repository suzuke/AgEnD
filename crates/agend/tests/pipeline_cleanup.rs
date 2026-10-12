//! A failed native holder shutdown must preserve the fixture for recovery.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod lab;
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod processes;

use agend_daemon::runtime::files;
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct OwnedHolder(Child);
impl Drop for OwnedHolder {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[test]
fn failed_shutdown_preserves_home_and_repo_until_the_real_holder_can_stop() {
    let root = processes::Lab::with_prefix(Path::new(env!("CARGO_BIN_EXE_agend")), "g13-pc");
    let home = root.root.join("home");
    let repo = root.root.join("home-repo");
    lab::setup(&home, &[]).unwrap();
    let mut holder = OwnedHolder(
        Command::new(env!("CARGO_BIN_EXE_agend"))
            .args(["holder", "g10-dev"])
            .env_clear()
            .env("AGEND_HOME", &home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let socket = files::socket_path(&home, "g10-dev");
    let deadline = Instant::now() + Duration::from_secs(10);
    while files::running(&home, "g10-dev").unwrap() != Some(holder.0.id()) || !socket.exists() {
        assert!(Instant::now() < deadline, "holder did not become ready");
        assert!(holder.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(10));
    }
    let parked = socket.with_extension("parked");
    fs::rename(&socket, &parked).unwrap();
    let error = lab::teardown(&home).unwrap_err();
    assert!(error.contains("fixture cleanup preserved"), "{error}");
    assert!(home.join("agend.db").is_file());
    assert!(repo.join("README.md").is_file());
    assert_eq!(
        files::running(&home, "g10-dev").unwrap(),
        Some(holder.0.id())
    );
    assert!(holder.0.try_wait().unwrap().is_none());

    fs::rename(&parked, &socket).unwrap();
    lab::teardown(&home).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = holder.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "holder did not exit after Shutdown"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "{status}");
    assert!(!home.exists());
    assert!(!repo.exists());
}
