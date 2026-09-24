//! End-to-end check of the argv[0] dispatch in the built binary.

use agend_testkit::tempdir::TempDir;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_agend");

#[test]
fn version_prints_the_package_version() {
    let out = Command::new(BIN).arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!("agend {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_command_fails_with_usage_hint() {
    let out = Command::new(BIN).arg("frobnicate").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("agend --help"));
}

#[test]
fn invoked_as_git_reaches_the_shim() {
    let dir = TempDir::new("argv0").unwrap();
    let git = dir.path().join("git");
    std::os::unix::fs::symlink(BIN, &git).unwrap();

    let out = Command::new(&git).arg("--version").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("agend shim (git)"), "stderr was: {stderr}");
    assert!(out.stdout.is_empty(), "the shim must not act like the CLI");
}

#[test]
fn non_utf8_argument_is_an_error_not_a_panic() {
    use std::os::unix::ffi::OsStrExt;
    let arg = std::ffi::OsStr::from_bytes(b"\xff");
    let out = Command::new(BIN).arg(arg).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not valid UTF-8"));
}
