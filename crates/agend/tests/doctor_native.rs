//! Doctor uses the real daemon and native backend producer, with no model.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
const BIN: &str = env!("CARGO_BIN_EXE_agend");
#[test]
fn doctor_observes_current_native_account_failure_and_recovery_without_model_input() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g13-doctor-native");
    let home = lab.home(1);
    let fake = Path::new(BIN).parent().unwrap().join("examples/fake_codex");
    assert!(fake.is_file(), "build agend example fake_codex first");
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    let cli = |args: &[&str]| {
        Command::new(BIN)
            .env_clear()
            .env("HOME", &lab.root)
            .env("AGEND_HOME", &home)
            .env("PATH", "/usr/bin:/bin")
            .args(args)
            .arg("--json")
            .output()
            .unwrap()
    };
    assert!(
        cli(&[
            "instance",
            "add",
            "native",
            "codex",
            "--program",
            fake.to_str().unwrap()
        ])
        .status
        .success()
    );
    daemon
        .expect_within("native: go (resume", Duration::from_secs(30))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let check = |wanted: &str| -> Value {
        loop {
            let out = cli(&["doctor"]);
            let rows: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
            let value = rows
                .iter()
                .find(|r| r["check"] == "authentication/native")
                .unwrap();
            if value["status"] == wanted {
                return value.clone();
            }
            assert!(Instant::now() < deadline, "{rows:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    assert!(
        check("fail")["detail"]
            .as_str()
            .unwrap()
            .contains("authentication required")
    );
    let socket = agend_daemon::driver::codex::launch::socket_path(&home, "native");
    let mut probe = agend_testkit::fake_agent::codex::Probe::connect(&socket).unwrap();
    probe
        .call(
            "initialize",
            json!({"clientInfo":{"name":"doctor-fixture","version":"0"}}),
        )
        .unwrap();
    probe.call("agendFake/diagnosticAccount",json!({"account":{"type":"apiKey","secret":"synthetic-secret"},"requiresOpenaiAuth":true})).unwrap();
    let value = check("ok");
    assert!(
        value["detail"]
            .as_str()
            .unwrap()
            .contains("provider validity not checked")
    );
    assert!(!value.to_string().contains("synthetic-secret"));
    let count = probe.call("agendFake/accountReadCount", json!({})).unwrap();
    assert_eq!(count["logins"], 0);
    assert!(
        cli(&["instance", "remove", "native", "--yes"])
            .status
            .success()
    );
    daemon.interrupt().unwrap();
    assert!(lab.running_holders().is_empty());
}
