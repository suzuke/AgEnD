//! Gate 13 home bootstrap through the real CLI with isolated OS homes.
use agend_testkit::{fake_daemon::FakeDaemon, tempdir::TempDir};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::{Command, Output},
};

fn command(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_agend"));
    cmd.env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::null());
    cmd
}

fn init(home: &Path) -> Output {
    command(home).arg("init").output().unwrap()
}

#[test]
fn default_home_reaches_the_real_protocol_producer_and_override_wins() {
    let root = TempDir::new("g13").unwrap();
    let default = root.path().join(".agend");
    fs::create_dir_all(default.join("run")).unwrap();
    let _daemon = FakeDaemon::start_at(&default.join("run/daemon.sock")).unwrap();
    let out = command(root.path())
        .args(["debug", "ping"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let other = root.path().join("other");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("fleet.yaml"), "original v1 data").unwrap();
    let out = command(root.path())
        .env("AGEND_HOME", &other)
        .arg("init")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("v1 home"));
    assert!(!other.join("config.toml").exists());
}

#[test]
fn init_publishes_private_config_once_and_preserves_operator_edits() {
    let root = TempDir::new("g13-home-init").unwrap();
    let first = init(root.path());
    assert_ne!(first.status.code(), Some(2), "{first:?}");
    let home = root.path().join(".agend");
    let config = home.join("config.toml");
    assert_eq!(
        fs::metadata(&home).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let text = fs::read_to_string(&config).unwrap();
    assert!(
        agend_daemon::notifier::config::parse(&text)
            .unwrap()
            .telegram
            .is_none()
    );
    fs::write(&config, "# operator-owned comments\n").unwrap();
    init(root.path());
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "# operator-owned comments\n"
    );
    assert!(!fs::read_dir(&home).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".config-init-")
    }));
}

#[test]
fn missing_agent_home_and_invalid_explicit_home_never_use_operator_default() {
    let root = TempDir::new("g13-home-refusal").unwrap();
    for value in ["", "relative"] {
        let out = command(root.path())
            .env("AGEND_HOME", value)
            .arg("init")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
    }
    let out = command(root.path())
        .env("AGEND_INSTANCE", "dev")
        .arg("init")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("daemon-provided environment"));
    assert!(!root.path().join(".agend").exists());
}

#[test]
fn init_refuses_v1_default_and_never_follows_config_symlinks() {
    let root = TempDir::new("g13-home-existing").unwrap();
    let home = root.path().join(".agend");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("fleet.yaml"), "v1 data").unwrap();
    assert_eq!(init(root.path()).status.code(), Some(1));
    assert!(!home.join("config.toml").exists());
    fs::remove_file(home.join("fleet.yaml")).unwrap();
    let foreign = root.path().join("foreign");
    fs::write(&foreign, "untouched").unwrap();
    symlink(&foreign, home.join("config.toml")).unwrap();
    let out = init(root.path());
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("regular file"));
    assert_eq!(fs::read_to_string(foreign).unwrap(), "untouched");
}
