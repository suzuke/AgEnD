//! Full production canary runner using native fake backend producers.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_testkit::tempdir::TempDir;
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
const BIN: &str = env!("CARGO_BIN_EXE_agend");

#[test]
fn canary_scope_rejects_other_homes_instances_and_changed_artifacts() {
    use agend_core::{
        model::Backend,
        runtime_records::{Instance, InstanceStatus},
    };
    use agend_daemon::backend_versions::{self, canary_scope};
    let root = TempDir::new("g13-canary-scope").unwrap();
    let source = root.path().join("source");
    let home = root.path().join("probe");
    fs::create_dir_all(home.join("workspace")).unwrap();
    let out = cli(
        &source,
        root.path(),
        &[
            "backend",
            "import",
            "opencode",
            "--version",
            "1.18.35",
            "--program",
            "/bin/echo",
        ],
    );
    assert!(out.status.success(), "{out:?}");
    let artifact = backend_versions::inspect(&source, "opencode", "1.18.35").unwrap();
    let program = source
        .join("backends/opencode/1.18.35/program")
        .canonicalize()
        .unwrap();
    let instance = Instance {
        id: "canary".into(),
        backend: Backend::Opencode,
        program: program.to_str().unwrap().into(),
        args: vec![],
        working_directory: home.join("workspace").to_str().unwrap().into(),
        session_id: None,
        status: InstanceStatus::Running,
        session_started: true,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "push".into(),
    };
    assert_eq!(canary_scope::expected(&home, &instance).unwrap(), None);
    canary_scope::create(&home, &source, &artifact).unwrap();
    assert_eq!(
        canary_scope::expected(&home, &instance).unwrap().as_deref(),
        Some("1.18.35")
    );
    let mut wrong = instance.clone();
    wrong.id = "fleet".into();
    assert!(canary_scope::expected(&home, &wrong).is_err());
    wrong = instance.clone();
    wrong.args.push("--other".into());
    assert!(canary_scope::expected(&home, &wrong).is_err());
    let other = root.path().join("other");
    fs::create_dir_all(other.join("workspace")).unwrap();
    fs::copy(
        home.join("canary-scope.json"),
        other.join("canary-scope.json"),
    )
    .unwrap();
    assert!(canary_scope::expected(&other, &instance).is_err());
    // Scope is not admission: no successful report has been published.
    assert!(backend_versions::verify_canary(&source, "opencode", "1.18.35", "unused").is_err());
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&program, b"changed executable").unwrap();
    assert!(canary_scope::expected(&home, &instance).is_err());
}

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
    let root = lab::Lab::with_prefix(Path::new(BIN), "g13-managed");
    let home = root.root.as_path().join("home");
    let user = root.root.as_path().join("user");
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
                // Match the production default. Unoptimized Linux binaries
                // are large; three concurrent native cases also hash copies.
                // The runner still enforces the same end-to-end deadline.
                "180",
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
    managed_fleet(&root, &home, backend, version);
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
    assert_eq!(admission().unwrap().unwrap().backend, backend);
}

// Exercise admission after a report made by the actual production canary.
fn managed_fleet(lab: &lab::Lab, home: &Path, backend: &str, version: &str) {
    use agend_core::runtime_records::ManagedLaunchIntent;
    let program = home
        .join("backends")
        .join(backend)
        .join(version)
        .join("program");
    let mut daemon = lab::Daemon::start(lab, home, &[]).unwrap();
    daemon.ready().unwrap();
    let out = cli(
        home,
        &lab.root,
        &[
            "instance",
            "add",
            "managed",
            backend,
            "--program",
            program.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    daemon
        .expect_within("managed: holder pid=", std::time::Duration::from_secs(30))
        .unwrap();
    let read_intent = || {
        let db = rusqlite::Connection::open(home.join("agend.db")).unwrap();
        let json: String = db
            .query_row(
                "SELECT intent FROM managed_launches WHERE instance_id='managed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str::<ManagedLaunchIntent>(&json).unwrap()
    };
    let holders = lab.running_holders();
    assert_eq!(holders.len(), 1);
    daemon.interrupt().unwrap();
    let original = read_intent();
    // An inherited holder needs its original launch identity, not a canary
    // for a new daemon build. Make the native report's build identity stale,
    // and independently prove it no longer authorizes any new execution.
    let report_path = program.parent().unwrap().join("canary.json");
    let mut report: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    report["agend_sha256"] = Value::String("0".repeat(64));
    fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();
    let binding =
        agend_daemon::backend_versions::ExecutableBinding::capture(Path::new(BIN)).unwrap();
    assert!(
        agend_daemon::backend_versions::check_launch(
            home,
            backend,
            program.to_str().unwrap(),
            home,
            "/usr/bin:/bin",
            Path::new(BIN),
            Ok(&binding),
        )
        .is_err()
    );
    let mut daemon = lab::Daemon::start(lab, home, &[]).unwrap();
    daemon.ready().unwrap();
    daemon.expect("managed: reconnected to holder").unwrap();
    assert_eq!(lab.running_holders(), holders);
    daemon.interrupt().unwrap();
    assert_eq!(read_intent(), original);
    // A valid but foreign UUID cannot be adopted or trigger replacement.
    let mut foreign = original.clone();
    foreign.binding = "12345678-1234-4234-8234-123456789abc".into();
    let db = rusqlite::Connection::open(home.join("agend.db")).unwrap();
    db.execute(
        "UPDATE managed_launches SET binding=?1,intent=?2 WHERE instance_id='managed'",
        rusqlite::params![foreign.binding, serde_json::to_string(&foreign).unwrap()],
    )
    .unwrap();
    drop(db);
    let mut daemon = lab::Daemon::start(lab, home, &[]).unwrap();
    daemon.ready().unwrap();
    assert!(
        daemon
            .log
            .iter()
            .any(|l| l.contains("managed holder identity unproven")),
        "{:?}",
        daemon.log
    );
    assert_eq!(lab.running_holders(), holders);
    daemon.interrupt().unwrap();
    assert_eq!(read_intent(), foreign);
    lab.stop_all_holders();
    assert!(lab.running_holders().is_empty());
}

#[test]
fn native_codex_switch_activates_an_admitted_version_and_preserves_session() {
    native_switch(
        "codex",
        "examples/fake_codex",
        "examples/fake_codex_next",
        "0.158.0",
        "0.159.0",
    );
}
#[test]
fn native_claude_switch_activates_and_rolls_back_with_the_same_session() {
    native_switch(
        "claude",
        "fake-claude-cli",
        "examples/fake_claude_next",
        "2.1.284",
        "2.1.285",
    );
}
#[test]
fn native_opencode_switch_activates_and_rolls_back_with_the_same_session() {
    native_switch(
        "opencode",
        "fake-opencode-cli",
        "examples/fake_opencode_next",
        "1.18.34",
        "1.18.35",
    );
}
fn native_switch(backend: &str, old: &str, next: &str, old_version: &str, next_version: &str) {
    native_switch_case(backend, old, next, old_version, next_version, false);
}

#[test]
fn native_codex_committed_switch_recovers_when_the_target_holder_is_absent() {
    native_switch_case(
        "codex",
        "examples/fake_codex",
        "examples/fake_codex_next",
        "0.158.0",
        "0.159.0",
        true,
    );
}

fn native_switch_case(
    backend: &str,
    old: &str,
    next: &str,
    old_version: &str,
    next_version: &str,
    remove_holder: bool,
) {
    use std::time::{Duration, Instant};
    let root = lab::Lab::with_prefix(Path::new(BIN), "g13-switch-live");
    let home = root.root.join("home");
    let user = root.root.join("user");
    fs::create_dir(&user).unwrap();
    for (version, name) in [(old_version, old), (next_version, next)] {
        let fake = Path::new(BIN).parent().unwrap().join(name);
        assert!(
            fake.is_file(),
            "build native fixture binaries and next-version examples first"
        );
        for args in [
            vec![
                "backend",
                "import",
                backend,
                "--version",
                version,
                "--program",
                fake.to_str().unwrap(),
            ],
            vec![
                "backend",
                "canary",
                backend,
                "--version",
                version,
                "--allow-model",
                "--timeout-seconds",
                "180",
            ],
        ] {
            let out = cli(&home, &user, &args);
            assert!(
                out.status.success(),
                "{} {}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    let mut daemon = lab::Daemon::start(&root, &home, &[]).unwrap();
    daemon.ready().unwrap();
    let program = home
        .join("backends")
        .join(backend)
        .join(old_version)
        .join("program");
    let out = cli(
        &home,
        &user,
        &[
            "instance",
            "add",
            "managed",
            backend,
            "--program",
            program.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    daemon
        .expect_within("managed: holder pid=", Duration::from_secs(30))
        .unwrap();
    let instance = || {
        let db = rusqlite::Connection::open(home.join("agend.db")).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db.query_row(
            "SELECT session_id,agent_pid FROM instances WHERE id='managed'",
            [],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<u32>>(1)?)),
        )
        .unwrap()
    };
    if backend == "codex" {
        daemon
            .expect_within(" created", Duration::from_secs(30))
            .unwrap();
    } else {
        // Claude's exact Ready frame must remain stable for five seconds.
        // OpenCode establishes its native session asynchronously after spawn.
        std::thread::sleep(Duration::from_secs(6));
    }
    let before_holders = root.running_holders();
    assert_eq!(before_holders.len(), 1);
    let out = cli(
        &home,
        &user,
        &[
            "backend",
            "switch",
            "prepare",
            "managed",
            "--version",
            next_version,
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let record: Value = serde_json::from_slice(&out.stdout).unwrap();
    let id = record["id"].as_str().unwrap();
    let hold_start = home.join("workspace/managed/.g13-hold-start");
    fs::write(&hold_start, b"hold only this test's target launch").unwrap();
    let out = cli(
        &home,
        &user,
        &[
            "backend",
            "switch",
            "activate",
            "managed",
            "--switch-id",
            id,
        ],
    );
    assert!(
        out.status.success(),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    {
        let committed: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(committed["phase"], "committed");
        let surviving = root.running_holders();
        assert_eq!(surviving.len(), 1);
        daemon.kill9().unwrap();
        assert_eq!(root.running_holders(), surviving);
        if remove_holder {
            root.stop_all_holders();
            assert!(root.running_holders().is_empty());
        }
        daemon = lab::Daemon::start(&root, &home, &[]).unwrap();
        daemon.expect("daemon ready").unwrap();
        if remove_holder {
            let replacement = root.running_holders();
            assert_eq!(replacement.len(), 1);
            assert_ne!(replacement[0].2, surviving[0].2);
        } else {
            assert_eq!(root.running_holders(), surviving);
        }
        fs::remove_file(hold_start).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let out = cli(&home, &user, &["backend", "switch", "status", "managed"]);
        assert!(out.status.success());
        let status: Value = serde_json::from_slice(&out.stdout).unwrap();
        if status["phase"] == "activated" {
            break;
        }
        if Instant::now() >= deadline {
            let screen = (|| {
                let mut client =
                    agend_client::Client::connect_once(&home.join("run/daemon.sock"), None)?;
                client.sender()?.subscribe_terminal("managed")?;
                client.next_terminal()
            })();
            let diagnostics = daemon
                .expect_within("__activation_timeout__", Duration::from_millis(10))
                .unwrap_err();
            panic!("activation did not finish: {status}; screen={screen:?}; {diagnostics}");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let after_holders = root.running_holders();
    assert_eq!(after_holders.len(), 1);
    assert_ne!(before_holders[0].2, after_holders[0].2);
    let out = cli(
        &home,
        &user,
        &[
            "backend",
            "switch",
            "rollback",
            "managed",
            "--switch-id",
            id,
        ],
    );
    assert!(
        out.status.success(),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let out = cli(&home, &user, &["backend", "switch", "status", "managed"]);
        assert!(out.status.success());
        let status: Value = serde_json::from_slice(&out.stdout).unwrap();
        if status["phase"] == "rolled_back" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "rollback did not finish: {status}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    let restored_holders = root.running_holders();
    assert_eq!(restored_holders.len(), 1);
    assert_ne!(restored_holders[0].2, after_holders[0].2);
    daemon.interrupt().unwrap();
    let after = instance();
    assert_eq!(record["session_id"].as_str(), after.0.as_deref());
    root.stop_all_holders();
    assert!(root.running_holders().is_empty());
}
