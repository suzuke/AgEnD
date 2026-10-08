//! Full production canary runner using native fake backend producers.
#![cfg(unix)]
use agend_testkit::tempdir::TempDir;
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const BIN: &str = env!("CARGO_BIN_EXE_agend");
fn cli(home: &Path, user: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .env_clear()
        .env("AGEND_HOME", home)
        .env("HOME", user)
        .env("PATH", "/usr/bin:/bin")
        .args(args)
        .arg("--json")
        .output()
        .unwrap()
}
#[test]
fn canary_requires_explicit_execution_opt_in_before_creating_a_home() {
    let root = TempDir::new("g13-canary-optin").unwrap();
    let home = root.path().join("home");
    let out = cli(
        &home,
        root.path(),
        &["backend", "canary", "codex", "--version", "0.158.0"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(!home.exists());
}
#[test]
fn native_fake_canary_confirms_three_messages_cleans_and_never_activates() {
    native_canary("codex", "examples/fake_codex", "0.158.0", "0.159.0");
}
#[test]
fn native_opencode_canary_requires_three_successful_parent_bound_responses() {
    native_canary("opencode", "fake-opencode-cli", "1.18.34", "1.18.35");
}
#[test]
fn native_claude_canary_waits_for_three_ack_and_stop_bound_prompts() {
    native_canary("claude", "fake-claude-cli", "2.1.284", "2.1.285");
}
fn native_canary(backend: &str, executable: &str, version: &str, wrong_version: &str) {
    let fake = Path::new(BIN).parent().unwrap().join(executable);
    assert!(
        fake.is_file(),
        "build fake_codex example and agend-testkit fake-opencode-cli first"
    );
    let root = TempDir::new("g13-canary").unwrap();
    let home = root.path().join("home");
    let user = root.path().join("user");
    fs::create_dir(&user).unwrap();
    fs::write(user.join(".claude.json"), b"preserve trust entries").unwrap();
    for (version, passed) in [(version, true), (wrong_version, false)] {
        let out = cli(
            &home,
            &user,
            &[
                "backend",
                "import",
                backend,
                "--version",
                version,
                "--program",
                fake.to_str().unwrap(),
            ],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        let out = cli(
            &home,
            &user,
            &[
                "backend",
                "canary",
                backend,
                "--version",
                version,
                "--allow-model",
                "--timeout-seconds",
                "60",
            ],
        );
        let record = home
            .join("backends")
            .join(backend)
            .join(version)
            .join("canary.json");
        let report: Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
        assert_eq!(
            out.status.success(),
            passed,
            "{}\n{report}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(report["passed"], passed);
        assert_eq!(report["cleanup_complete"], true);
        assert!(report["retained_home"].is_null());
        let id = report["run_id"].as_str().unwrap();
        let temporary = format!("/tmp/agend-canary-{}", &id[..18]);
        assert!(!Path::new(&temporary).exists());
        let ps = Command::new("/bin/ps")
            .args(["-A", "-o", "command="])
            .output()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&ps.stdout).contains(&temporary),
            "canary process remains"
        );
        assert!(
            !home.join("agend.db").exists(),
            "operator fleet must remain unchanged"
        );
        assert!(
            !home
                .join("backends")
                .join(backend)
                .join("active.json")
                .exists()
        );
        if passed {
            verify_retained_report(&home, &user, backend, version, &report);
            let receipts = report["receipts"].as_array().unwrap();
            assert_eq!(receipts.len(), 3);
            for receipt in receipts {
                assert_eq!(receipt["state"], "confirmed");
                assert_eq!(receipt["to_instance"], "canary");
                assert_eq!(
                    receipt["from_instance"],
                    agend_core::protocol::client::OPERATOR_MESSAGE_SENDER
                );
                if backend == "claude" {
                    assert!(receipt["turn_id"].is_null());
                } else {
                    assert!(receipt["turn_id"].is_string());
                }
            }
        } else {
            assert_eq!(report["receipts"].as_array().unwrap().len(), 0);
            assert!(report["error"].as_str().unwrap().contains("version"));
        }
    }
    assert_eq!(
        fs::read(user.join(".claude.json")).unwrap(),
        b"preserve trust entries"
    );
}

fn verify_retained_report(home: &Path, user: &Path, backend: &str, version: &str, report: &Value) {
    let path = home
        .join("backends")
        .join(backend)
        .join(version)
        .join("canary.json");
    let original = fs::read(&path).unwrap();
    let inspect = || {
        let out = cli(
            home,
            user,
            &["backend", "inspect", backend, "--version", version],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    assert_eq!(
        inspect()["canary"],
        "passed",
        "native producer report must validate"
    );
    let mut mutations = Vec::new();
    for (field, value) in [
        ("passed", serde_json::json!(false)),
        ("cleanup_complete", serde_json::json!(false)),
        ("retained_home", serde_json::json!("/tmp/still-running")),
        ("agend_sha256", serde_json::json!("0".repeat(64))),
        ("os", serde_json::json!("other-platform")),
        (
            "observed_version",
            serde_json::json!("codex-cli wrong-version"),
        ),
    ] {
        let mut changed = report.clone();
        changed[field] = value;
        mutations.push((field, changed));
    }
    let mut changed = report.clone();
    changed["artifact"]["sha256"] = serde_json::json!("0".repeat(64));
    mutations.push(("artifact drift", changed));
    let mut changed = report.clone();
    changed["receipts"].as_array_mut().unwrap().pop();
    mutations.push(("missing delivery", changed));
    let mut changed = report.clone();
    changed["receipts"][1] = changed["receipts"][0].clone();
    mutations.push(("duplicate delivery", changed));
    let mut changed = report.clone();
    changed["receipts"][0]["from_instance"] = serde_json::json!("operator");
    mutations.push(("old agent identity", changed));
    let mut changed = report.clone();
    changed["receipts"][0]["state"] = serde_json::json!("sent");
    mutations.push(("unconfirmed delivery", changed));
    let mut changed = report.clone();
    changed["receipts"][0]["attempted_at_unix_ms"] = serde_json::Value::Null;
    mutations.push(("unattempted delivery", changed));
    let mut changed = report.clone();
    changed["outcomes"].as_array_mut().unwrap().pop();
    mutations.push(("missing outcome", changed));
    let mut changed = report.clone();
    changed["outcomes"][0]["state"] = serde_json::json!("failed");
    mutations.push(("failed execution", changed));
    let mut changed = report.clone();
    changed["outcomes"][0]["turn_id"] = serde_json::json!("another-turn");
    mutations.push(("foreign execution", changed));
    let mut changed = report.clone();
    changed["outcomes"][0]["execution_id"] = Value::Null;
    mutations.push(("missing execution identity", changed));
    let mut changed = report.clone();
    changed["outcomes"][1]["execution_id"] = changed["outcomes"][0]["execution_id"].clone();
    mutations.push(("reused execution identity", changed));
    let start = report["started_at_unix_ms"].as_u64().unwrap();
    let end = start + report["elapsed_ms"].as_u64().unwrap();
    for (name, index, field, value) in [
        ("before start", 0, "attempted_at_unix_ms", start - 1),
        (
            "decreasing attempt",
            1,
            "attempted_at_unix_ms",
            report["receipts"][0]["updated_at_unix_ms"]
                .as_u64()
                .unwrap()
                - 1,
        ),
        ("after end", 2, "updated_at_unix_ms", end + 1),
    ] {
        let mut changed = report.clone();
        changed["receipts"][index][field] = serde_json::json!(value);
        mutations.push((name, changed));
    }
    let mut changed = report.clone();
    changed["started_at_unix_ms"] = serde_json::json!(u64::MAX);
    mutations.push(("time overflow", changed));
    let digest = report["agend_sha256"].as_str().unwrap();
    let binding =
        agend_daemon::backend_versions::ExecutableBinding::capture(Path::new(BIN)).unwrap();
    let program = home
        .join("backends")
        .join(backend)
        .join(version)
        .join("program");
    let admission = || {
        agend_daemon::backend_versions::check_launch(
            home,
            backend,
            program.to_str().unwrap(),
            home,
            "/usr/bin:/bin",
            Path::new(BIN),
            Ok(&binding),
        )
    };
    let verify = || agend_daemon::backend_versions::verify_canary(home, backend, version, digest);
    for (name, changed) in mutations {
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(verify().is_err(), "{name}");
        assert!(admission().is_err(), "admission: {name}");
    }
    fs::write(&path, vec![b' '; 65537]).unwrap();
    assert!(verify().is_err());
    let result = inspect();
    assert_eq!(result["canary"], "invalid");
    assert_eq!(result["active"], false);
    assert!(
        result["canary_reason"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    fs::write(path, original).unwrap();
    assert_eq!(inspect()["canary"], "passed");
    assert!(admission().unwrap_err().contains("launch-boundary pinning"));
}
