//! Real CLI import/inspect against native executable bytes. No backend is run.
#![cfg(unix)]
use agend_testkit::tempdir::TempDir;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;
use std::process::{Command, Output};
const BIN: &str = env!("CARGO_BIN_EXE_agend");
#[test]
fn maintenance_blocks_native_import_before_publication_and_release_allows_retry() {
    let root = TempDir::new("g13-backend-maintenance").unwrap();
    let home = root.path().join("data");
    let maintenance = agend_daemon::store::maintenance::Maintenance::acquire(&home).unwrap();
    let out = import(&home, "fixture-1", "/usr/bin/true");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("home is being maintained"));
    assert!(!home.join("backends").exists());
    drop(maintenance);
    assert!(import(&home, "fixture-1", "/usr/bin/true").status.success());
    assert!(inspect(&home).status.success());
}

fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .env_clear()
        .env("HOME", home.parent().unwrap())
        .env("AGEND_HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .args(args)
        .arg("--json")
        .output()
        .unwrap()
}
fn import(home: &Path, version: &str, source: &str) -> Output {
    cli(
        home,
        &[
            "backend",
            "import",
            "claude",
            "--version",
            version,
            "--program",
            source,
        ],
    )
}
fn inspect(home: &Path) -> Output {
    cli(
        home,
        &["backend", "inspect", "claude", "--version", "fixture-1"],
    )
}
#[test]
fn imported_native_bytes_are_isolated_unverified_and_tampering_is_refused() {
    let root = TempDir::new("g13-backend").unwrap();
    let home = root.path().join("data");
    let original = fs::read("/usr/bin/true").unwrap();
    let out = import(&home, "fixture-1", "/usr/bin/true");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let data: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(data["canary"], "not_run");
    assert_eq!(data["active"], false);
    let program = Path::new(data["program"].as_str().unwrap());
    assert_eq!(fs::read(program).unwrap(), original);
    assert_ne!(
        fs::metadata(program).unwrap().ino(),
        fs::metadata("/usr/bin/true").unwrap().ino()
    );
    assert_eq!(fs::metadata(program).unwrap().mode() & 0o777, 0o500);
    assert!(inspect(&home).status.success());
    assert!(!import(&home, "fixture-1", "/usr/bin/true").status.success());
    assert_eq!(fs::read(program).unwrap(), original);
    fs::set_permissions(program, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(program, b"modified executable").unwrap();
    assert!(!inspect(&home).status.success());
    assert_eq!(fs::read("/usr/bin/true").unwrap(), original);
}
#[test]
fn invalid_version_wrapper_and_agent_calls_do_not_create_backend_files() {
    let root = TempDir::new("g13-backend-invalid").unwrap();
    let home = root.path().join("data");
    for version in ["../escape", "", "v/1", "a\nb", ".hidden"] {
        assert_eq!(
            import(&home, version, "/usr/bin/true").status.code(),
            Some(2)
        );
        assert!(!home.exists());
    }
    let wrapper = root.path().join("wrapper");
    fs::write(&wrapper, b"#!/bin/sh\ntouch should-not-run\n").unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        !import(&home, "fixture-1", wrapper.to_str().unwrap())
            .status
            .success()
    );
    assert!(!home.exists());
    let out = Command::new(BIN)
        .env_clear()
        .env("HOME", root.path())
        .env("AGEND_HOME", &home)
        .env("AGEND_INSTANCE", "agent")
        .args([
            "backend",
            "import",
            "claude",
            "--version",
            "fixture-1",
            "--program",
            "/usr/bin/true",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!home.exists());
}
#[test]
fn redirected_storage_and_manifest_identity_changes_are_refused() {
    let root = TempDir::new("g13-backend-links").unwrap();
    let home = root.path().join("data");
    let foreign = root.path().join("foreign");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("sentinel"), b"keep").unwrap();
    symlink(&foreign, home.join("backends")).unwrap();
    assert!(!import(&home, "fixture-1", "/usr/bin/true").status.success());
    assert_eq!(fs::read_dir(&foreign).unwrap().count(), 1);
    fs::remove_file(home.join("backends")).unwrap();
    assert!(import(&home, "fixture-1", "/usr/bin/true").status.success());
    let record = home.join("backends/claude/fixture-1/import.json");
    let mut data: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
    data["backend"] = Value::String("codex".into());
    fs::write(record, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(!inspect(&home).status.success());
    assert_eq!(fs::read(foreign.join("sentinel")).unwrap(), b"keep");
}

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

#[test]
fn daemon_refuses_unverified_or_changed_managed_program_before_starting_a_holder() {
    use agend_daemon::store::{InstanceStatus, SqliteStore};
    use agend_testkit::block_on;
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g13bv");
    for (n, mode) in [
        "pending",
        "changed",
        "alias",
        "path",
        "relative",
        "denied-first",
        "ambiguous",
    ]
    .into_iter()
    .enumerate()
    {
        let home = lab.home(n);
        assert!(import(&home, "fixture-1", "/usr/bin/true").status.success());
        let program = home.join("backends/claude/fixture-1/program");
        let id = format!("g13-{mode}");
        let mut instance = lab::add(&home, &id, "exit 99").unwrap();
        lab::remove(&home, &id).unwrap();
        instance.program = program.to_str().unwrap().into();
        instance.args.clear();
        instance.delivery = "inbox".into();
        let expected = if mode == "ambiguous" {
            "ambiguous relative backend program"
        } else if mode == "changed" {
            fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
            fs::write(&program, b"changed").unwrap();
            "imported executable changed"
        } else {
            if mode == "alias" {
                let alias = home.join("alias");
                symlink(&program, &alias).unwrap();
                instance.program = alias.to_str().unwrap().into();
            }
            "managed backend canary not run"
        };
        if mode == "path" || mode == "denied-first" {
            instance.program = "program".into();
        }
        if mode == "relative" {
            instance.program = "../../backends/claude/fixture-1/program".into();
        }
        if mode == "ambiguous" {
            instance.program = "fixture-1/program".into();
        }
        let store = SqliteStore::open(&home, 0).unwrap();
        block_on(store.add_instance(&instance)).unwrap();
        drop(store);
        let mut search = format!("{}:/usr/bin:/bin", program.parent().unwrap().display());
        if mode == "denied-first" {
            // SAFETY: getuid reads process identity. Root bypasses owner permission.
            assert_ne!(
                unsafe { libc::getuid() },
                0,
                "run this native permission test as a non-root user"
            );
            let first = home.join("first");
            fs::create_dir(&first).unwrap();
            fs::copy("/usr/bin/true", first.join("program")).unwrap();
            fs::set_permissions(first.join("program"), fs::Permissions::from_mode(0o610)).unwrap();
            search = format!("{}:{search}", first.display());
        }
        let mut daemon = lab::Daemon::start(&lab, &home, &[("PATH", &search)]).unwrap();
        daemon.expect(expected).unwrap();
        daemon.ready().unwrap();
        assert!(lab.running_holders().is_empty());
        daemon.interrupt().unwrap();
        assert_eq!(lab::status(&home, &id).unwrap(), InstanceStatus::Failed);
    }
    assert!(lab.running_holders().is_empty());
}
