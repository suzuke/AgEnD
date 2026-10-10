//! The real CLI emits reviewable definitions; native parsers check their syntax.
//! All files are isolated; no service-manager registration occurs here.
#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::Command;

use agend_testkit::tempdir::TempDir;
use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_agend");

fn preview(root: &Path, manager: &str) -> std::process::Output {
    Command::new(BIN)
        .env_clear()
        .env("HOME", root.join("user & <é>"))
        .env("AGEND_HOME", root.join("data % $dollar \"quote\""))
        .env("PATH", "/usr/bin:/bin")
        .args(["service", "plan", "--manager", manager, "--json"])
        .output()
        .unwrap()
}

#[test]
fn preview_is_read_only_and_selects_the_user_service_location() {
    let root = TempDir::new("g13s").unwrap();
    for manager in ["launchd", "systemd"] {
        let out = preview(root.path(), manager);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["preview"], true);
        assert_eq!(value["spec"]["manager"], manager);
        let suffix = if manager == "launchd" {
            "Library/LaunchAgents/dev.agend.daemon.plist"
        } else {
            ".config/systemd/user/agend-daemon.service"
        };
        assert_eq!(
            value["service_path"],
            root.path()
                .join("user & <é>")
                .join(suffix)
                .to_str()
                .unwrap()
        );
        assert_eq!(
            value["spec"]["program"],
            root.path()
                .join("data % $dollar \"quote\"")
                .join("service/agend")
                .to_str()
                .unwrap()
        );
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn bad_environment_cannot_escape_into_a_unit() {
    let root = TempDir::new("g13s-invalid").unwrap();
    for (key, value) in [
        ("AGEND_HOME", "/tmp/a\nExecStart=/bad"),
        ("HOME", "/tmp/a\r<key>Bad</key>"),
        ("PATH", "/bin:relative"),
        ("XDG_CONFIG_HOME", "relative"),
    ] {
        let out = Command::new(BIN)
            .env_clear()
            .env("HOME", root.path())
            .env("PATH", "/bin")
            .env(key, value)
            .args(["service", "plan", "--manager", "systemd", "--json"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["error"]["code"], "usage");
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn current_platform_parser_accepts_the_real_generated_definition() {
    let root = TempDir::new("g13s-native").unwrap();
    let manager = if cfg!(target_os = "macos") {
        "launchd"
    } else {
        "systemd"
    };
    let out = preview(root.path(), manager);
    assert!(out.status.success());
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    let path = Path::new(value["service_path"].as_str().unwrap());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, value["definition"].as_str().unwrap()).unwrap();
    let program = Path::new(value["spec"]["program"].as_str().unwrap());
    fs::create_dir_all(program.parent().unwrap()).unwrap();
    fs::copy(BIN, program).unwrap();
    let result = if cfg!(target_os = "macos") {
        Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(path)
            .output()
            .unwrap()
    } else {
        Command::new("systemd-analyze")
            .arg("verify")
            .arg(path)
            .output()
            .unwrap()
    };
    assert!(
        result.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

fn install(root: &Path) -> std::process::Output {
    Command::new(BIN)
        .env_clear()
        .env("HOME", root)
        .env("PATH", "/usr/bin:/bin")
        .args(["service", "install", "--no-start", "--json"])
        .output()
        .unwrap()
}

#[test]
fn real_cli_prepares_private_owned_files_and_reuses_them_without_touching_data() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new("g13s-install").unwrap();
    let first = install(root.path());
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    let value: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(value["phase"], "prepared");
    let record = &value["installation"];
    let unit = Path::new(record["service_path"].as_str().unwrap());
    let program = Path::new(record["spec"]["program"].as_str().unwrap());
    let home = Path::new(record["spec"]["home"].as_str().unwrap());
    assert_eq!(
        fs::metadata(unit).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(program).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let before = fs::metadata(program).unwrap().modified().unwrap();
    fs::write(home.join("config.toml"), b"# user settings\n").unwrap();
    let second = install(root.path());
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stdout)
    );
    assert_eq!(fs::metadata(program).unwrap().modified().unwrap(), before);
    assert_eq!(
        fs::read(home.join("config.toml")).unwrap(),
        b"# user settings\n"
    );
    assert_eq!(fs::read_dir(home.join("service")).unwrap().count(), 3);
}

#[test]
fn real_cli_preserves_an_edited_unit_and_agent_setup_is_refused() {
    let root = TempDir::new("g13s-edit").unwrap();
    let first = install(root.path());
    assert!(first.status.success());
    let value: Value = serde_json::from_slice(&first.stdout).unwrap();
    let unit = Path::new(value["installation"]["service_path"].as_str().unwrap());
    fs::write(unit, b"operator replacement").unwrap();
    let second = install(root.path());
    assert_eq!(second.status.code(), Some(1));
    assert_eq!(fs::read(unit).unwrap(), b"operator replacement");
    let agent = Command::new(BIN)
        .env_clear()
        .env("HOME", root.path())
        .env("PATH", "/bin")
        .env("AGEND_INSTANCE", "agent")
        .args(["service", "install", "--no-start", "--json"])
        .output()
        .unwrap();
    assert_eq!(agent.status.code(), Some(2));
}

#[test]
fn data_deletion_requires_exact_confirmation_and_an_owned_receipt_before_io() {
    let root = TempDir::new("g13s-confirm").unwrap();
    let home = root.path().canonicalize().unwrap().join("data");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("sentinel"), b"keep").unwrap();
    for (extra, expected) in [
        (vec!["--delete-data"], 2),
        (vec!["--delete-data", "--confirm-home", "/wrong-home"], 2),
        (vec!["--confirm-home", home.to_str().unwrap()], 2),
        (
            vec!["--delete-data", "--confirm-home", home.to_str().unwrap()],
            1,
        ),
    ] {
        let out = Command::new(BIN)
            .env_clear()
            .env("HOME", root.path())
            .env("AGEND_HOME", &home)
            .env("PATH", "/usr/bin:/bin")
            .args(["uninstall", "--json"])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(fs::read(home.join("sentinel")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&home).unwrap().count(), 1);
    }
}
