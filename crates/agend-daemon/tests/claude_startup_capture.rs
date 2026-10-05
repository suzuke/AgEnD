//! Native passive capture regression; no true Claude executable is used.
#[path = "common/claude_startup_capture.rs"]
mod capture;
use agend_testkit::tempdir::TempDir;
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

fn agend() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let path = std::env::var_os("AGEND_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| exe.parent().unwrap().parent().unwrap().join("agend"));
    assert!(path.is_file(), "build first: cargo build -p agend --bins");
    path
}
fn worker(root: &Path) -> PathBuf {
    let path = root.join("native-worker");
    fs::write(
        &path,
        r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$@" > "$root/argv"
pwd > "$root/workspace"
stty -echo -icanon min 0 time 1
trap 'printf "\r\nSIZE:%s\r\n" "$(stty size)"' WINCH
printf '\033[32mNATIVE STARTUP CAPTURE\033[0m\r\n繁中 · 界 · é\r\n'
printf 'SIZE:%s\r\n' "$(stty size)"
while :; do
    byte=$(dd bs=1 count=1 2>/dev/null)
    if [ -n "$byte" ]; then printf '%s' "$byte" >> "$root/input-received"; fi
    sleep 0.05
done
"#,
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path.canonicalize().unwrap()
}
fn options(program: &Path, out: &Path, columns: u16) -> capture::Options {
    capture::Options::parse(&[
        "--program".into(),
        program.display().to_string(),
        "--sha256".into(),
        capture::digest(program).unwrap(),
        "--version-label".into(),
        "native-producer-only".into(),
        "--columns".into(),
        columns.to_string(),
        "--rows".into(),
        "24".into(),
        "--seconds".into(),
        "2".into(),
        "--out".into(),
        out.display().to_string(),
    ])
    .unwrap()
}
#[test]
fn real_holder_captures_two_widths_without_any_input_and_cleans_up() {
    let root = TempDir::new("g12-startup-native").unwrap();
    let program = worker(root.path());
    let daemon = agend();
    for columns in [100, 140] {
        let out = root.path().join(format!("capture-{columns}"));
        let options = options(&program, &out, columns);
        let count = capture::run(&options, &daemon).unwrap();
        let frames: Vec<Value> = fs::read_to_string(out.join("screens.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(frames.len(), count + 1);
        let header = &frames[0];
        assert_eq!(header["program_sha256"], capture::digest(&program).unwrap());
        assert_eq!(header["version_was_queried"], false);
        assert_eq!(header["terminal_input_operations"], 0);
        assert_eq!(header["message_delivery_operations"], 0);
        assert_eq!(header["startup"], "not_assessed");
        let rendered = frames[1..]
            .iter()
            .map(|frame| {
                assert_eq!(frame["msg"]["size"]["rows"], 24);
                assert_eq!(frame["msg"]["size"]["columns"], columns);
                frame["msg"]["text"].as_str().unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("NATIVE STARTUP CAPTURE"));
        assert!(rendered.contains("繁中 · 界 · é"), "{rendered}");
        assert!(
            rendered.contains(&format!("SIZE:24 {columns}")),
            "{rendered}"
        );
        let argv = fs::read_to_string(root.path().join("argv")).unwrap();
        let args: Vec<_> = argv.lines().collect();
        for pair in [
            ["--model", "haiku"],
            ["--effort", "low"],
            ["--permission-mode", "bypassPermissions"],
        ] {
            assert!(args.windows(2).any(|actual| actual == pair), "{args:?}");
        }
        for flag in ["--session-id", "--settings"] {
            let i = args.iter().position(|arg| *arg == flag).unwrap();
            assert!(i + 1 < args.len());
            if flag != "--session-id" {
                assert!(!Path::new(args[i + 1]).exists(), "capture home not removed");
            }
        }
        let workspace = fs::read_to_string(root.path().join("workspace")).unwrap();
        assert!(
            !Path::new(workspace.trim()).exists(),
            "capture workspace survived cleanup"
        );
        assert!(!args.contains(&"--version"));
        assert!(!root.path().join("input-received").exists());
        let result: Value =
            serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap()).unwrap();
        assert_eq!(result["ok"], true);
        assert_eq!(result["input_sent"], false);
        assert_eq!(
            fs::metadata(out.join("screens.jsonl"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let ps = std::process::Command::new("ps")
        .args(["-axo", "command="])
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&ps.stdout).contains(program.to_str().unwrap()),
        "native producer survived cleanup"
    );
}
#[test]
fn wrong_program_hash_and_existing_evidence_never_start_the_program() {
    let root = TempDir::new("g12-startup-preflight").unwrap();
    let program = worker(root.path());
    let out = root.path().join("evidence");
    let mut opts = options(&program, &out, 100);
    opts.sha256 = "0".repeat(64);
    assert!(
        capture::run(&opts, &agend())
            .unwrap_err()
            .contains("hash mismatch")
    );
    assert!(!out.exists());
    assert!(!root.path().join("argv").exists());
    opts.sha256 = capture::digest(&program).unwrap();
    fs::create_dir(&out).unwrap();
    fs::write(out.join("prior-evidence"), b"keep").unwrap();
    assert!(capture::run(&opts, &agend()).is_err());
    assert_eq!(fs::read(out.join("prior-evidence")).unwrap(), b"keep");
    assert!(!root.path().join("argv").exists());
}

#[test]
fn a_rejected_screen_is_not_written_and_failure_still_cleans_the_holder() {
    let root = TempDir::new("g12-startup-screen-refusal").unwrap();
    let program = worker(root.path());
    let source = fs::read_to_string(&program)
        .unwrap()
        .replace("NATIVE STARTUP CAPTURE", "Bearer capture-test-only");
    fs::write(&program, source).unwrap();
    let out = root.path().join("evidence");
    let error = capture::run(&options(&program, &out, 100), &agend()).unwrap_err();
    assert!(error.contains("secret scan"), "{error}");
    let stored = fs::read_to_string(out.join("screens.jsonl")).unwrap();
    assert!(!stored.contains("Bearer "));
    let result: Value =
        serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["ok"], false);
    let workspace = fs::read_to_string(root.path().join("workspace")).unwrap();
    assert!(!Path::new(workspace.trim()).exists());
    let ps = std::process::Command::new("ps")
        .args(["-axo", "command="])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&ps.stdout).contains(program.to_str().unwrap()));
}

#[test]
fn soft_wrapped_identifiers_are_redacted_as_a_complete_native_line() {
    let root = TempDir::new("g12-startup-soft-wrap").unwrap();
    let program = worker(root.path());
    let source = fs::read_to_string(&program).unwrap().replace(
        "trap 'printf",
        "emit_wrapped() {\n    size=$(stty size)\n    cols=${size#* }\n    padding=$((cols - 14))\n    printf \"%${padding}s%s\\r\\n\" '' 'captureprivate@example.invalid'\n}\nemit_wrapped\ntrap 'emit_wrapped; printf",
    );
    fs::write(&program, source).unwrap();
    let out = root.path().join("evidence");
    capture::run(&options(&program, &out, 100), &agend()).unwrap();
    let stored = fs::read_to_string(out.join("screens.jsonl")).unwrap();
    assert!(
        !stored.contains("captureprivate"),
        "wrapped identifier leaked"
    );
    assert!(
        stored.contains("<email>"),
        "native wrapped email was not redacted"
    );
}

#[test]
fn a_soft_wrapped_bearer_prefix_is_refused_before_writing_the_frame() {
    let root = TempDir::new("g12-startup-wrapped-refusal").unwrap();
    let program = worker(root.path());
    let source = fs::read_to_string(&program).unwrap().replace(
        "trap 'printf",
        "emit_wrapped() {\n    size=$(stty size)\n    cols=${size#* }\n    padding=$((cols - 3))\n    printf \"%${padding}s%s\\r\\n\" '' 'Bearer synthetic-wrap-only'\n}\nwhile [ \"$(stty size)\" != \"24 100\" ]; do sleep .02; done\nemit_wrapped\ntrap 'emit_wrapped; printf",
    );
    fs::write(&program, source).unwrap();
    let out = root.path().join("evidence");
    let error = capture::run(&options(&program, &out, 100), &agend()).unwrap_err();
    assert!(error.contains("secret scan"), "{error}");
    let stored = fs::read_to_string(out.join("screens.jsonl")).unwrap();
    assert!(!stored.contains("synthetic-wrap-only"));
    let result: Value =
        serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["ok"], false);
    let workspace = fs::read_to_string(root.path().join("workspace")).unwrap();
    assert!(!Path::new(workspace.trim()).exists());
}
