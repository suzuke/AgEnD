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

// These cases verify producer content and cleanup within a fixed observation
// window, not concurrent startup capacity. Isolate this binary's native labs
// through teardown; do not extend capture deadlines or retry failed cases.
static NATIVE_LAB: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

// This native producer re-renders the real No fixture through the holder.
// Its selected-Yes/after-trust output is synthetic, never a true CLI fixture.
fn trust_worker(root: &Path, columns: u16, mode: &str) -> PathBuf {
    fs::write(root.join("columns"), columns.to_string()).unwrap();
    fs::write(root.join("mode"), mode).unwrap();
    fs::write(
        root.join("no-screen.txt"),
        if columns == 100 {
            include_str!(
                "../../agend-core/tests/fixtures/screens/claude-2.1.284-workspace-trust-100x24.txt"
            )
        } else {
            include_str!(
                "../../agend-core/tests/fixtures/screens/claude-2.1.284-workspace-trust-140x24.txt"
            )
        },
    )
    .unwrap();
    let path = root.join("native-trust-worker");
    fs::write(&path, r#"#!/usr/bin/env python3
import fcntl, os, pathlib, struct, termios, time, tty
root = pathlib.Path(__file__).resolve().parent
(root / 'workspace').write_text(os.getcwd())
mode = (root / 'mode').read_text()
if mode != 'delayed-input':
    tty.setraw(0)
columns = int((root / 'columns').read_text())
while struct.unpack('HHHH', fcntl.ioctl(0, termios.TIOCGWINSZ, b'\0' * 8))[:2] != (24, columns):
    time.sleep(.01)
mode = (root / 'mode').read_text()
text = (root / 'no-screen.txt').read_text().replace('<rec>/h1/workspace/g12-startup-capture', str(pathlib.Path.cwd().resolve()))
if mode == 'wrong-path':
    text = text.replace(str(pathlib.Path.cwd().resolve()), '/tmp/not-this-workspace')
if mode == 'prefix-path':
    text = text.replace(str(pathlib.Path.cwd().resolve()), str(pathlib.Path.cwd().resolve()) + '-foreign')
if mode == 'path-elsewhere':
    text = text.replace(str(pathlib.Path.cwd().resolve()), '/tmp/not-this-workspace')
    text = text.rstrip('\n') + '\n' + str(pathlib.Path.cwd().resolve()) + '\n'
if mode == 'duplicate-path-header':
    text = text.rstrip('\n') + '\nAccessing workspace:\n/tmp/not-this-workspace\n'
if mode == 'unknown':
    text = 'SYNTHETIC UNKNOWN STARTUP PROMPT\n'
yes = text.replace('❯ No, exit\n   Yes, I trust this folder', '  No, exit\n ❯ Yes, I trust this folder')
if mode.startswith('no-extra-'):
    text = text.rstrip('\n') + '\n' + ('❯ 2. Exit' if mode.endswith('exit') else '❯ No, exit') + '\n'
if mode.startswith('yes-extra-'):
    yes = yes.rstrip('\n') + '\n' + ('❯ 2. Exit' if mode.endswith('exit') else '❯ Yes, I trust this folder') + '\n'
def draw(value):
    os.write(1, ('\x1b[2J\x1b[H\x1b[?25l' + value.rstrip('\n').replace('\n', '\r\n')).encode())
draw(yes if mode == 'preselected-yes' else text)
if mode == 'delayed-input':
    time.sleep(.75)
    tty.setraw(0)
if mode == 'transient-unknown':
    time.sleep(.4)
    draw('SYNTHETIC UNKNOWN DURING SETTLE\n')
    time.sleep(.8)
    draw(text)
    stable_at = time.monotonic()
pending = b''
while True:
    chunk = os.read(0, 64)
    if mode == 'transient-unknown' and not (root / 'first-input-after-stable').exists():
        (root / 'first-input-after-stable').write_text(str(time.monotonic() - stable_at))
    with (root / 'input-received').open('ab') as file:
        file.write(chunk)
    pending += chunk
    if b'\x1b[B' in pending:
        draw(text if mode == 'stuck-no' else yes)
    if b'\r' in pending:
        draw('SYNTHETIC AFTER TRUST; NO FURTHER KEYS\n')
"#).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path.canonicalize().unwrap()
}

fn controlled_options(program: &Path, out: &Path, columns: u16) -> capture::Options {
    let mut options = options(program, out, columns);
    options.version_label = "2.1.284".into();
    options.accept_workspace_trust = true;
    // Three stable prompts plus native frame publication need more than four seconds.
    options.seconds = 6;
    options
}

fn development_worker(root: &Path, width: u16, mode: &str) -> PathBuf {
    let program = trust_worker(root, width, "normal");
    let mut screen = if width == 100 {
        include_str!(
            "../../agend-core/tests/fixtures/screens/claude-2.1.284-development-channels-100x24.txt"
        )
    } else {
        include_str!(
            "../../agend-core/tests/fixtures/screens/claude-2.1.284-development-channels-140x24.txt"
        )
    }
    .to_owned();
    match mode {
        "foreign-prefix" => {
            screen = screen.replace("Channels: server:agend", "Channels: server:agend-other")
        }
        "foreign-extra" => {
            screen = screen.replace(
                "Channels: server:agend",
                "Channels: server:agend, server:other",
            )
        }
        "duplicate-header" => screen = format!("{}\nChannels: server:other\n", screen.trim_end()),
        "selected-exit" => {
            screen = screen.replace(
                "❯ 1. I am using this for local development\n    2. Exit",
                "  1. I am using this for local development\n  ❯ 2. Exit",
            )
        }
        "selected-exit-plus-decoy" => {
            screen = screen.replace(
                "❯ 1. I am using this for local development\n    2. Exit",
                "  1. I am using this for local development\n  ❯ 2. Exit\n\n  ❯ 1. I am using this for local development\n    2. Exit",
            );
        }
        "duplicate-selected-local" => {
            screen = format!(
                "{}\n  ❯ 1. I am using this for local development\n",
                screen.trim_end()
            );
        }
        "duplicate-selected-exit" => {
            screen = format!("{}\n  ❯ 2. Exit\n", screen.trim_end());
        }
        "missing-warning" => {
            screen = screen.replace("Do not use this", "Synthetic altered warning")
        }
        "normal" | "premature" | "unchanged" => {}
        _ => panic!("unknown native development producer mode"),
    }
    fs::write(root.join("development-screen.txt"), screen).unwrap();
    let script = fs::read_to_string(&program).unwrap();
    let initial = if mode == "premature" {
        "draw((root / 'development-screen.txt').read_text())"
    } else {
        "draw(yes if mode == 'preselected-yes' else text)"
    };
    let after = if mode == "unchanged" {
        "draw((root / 'development-screen.txt').read_text())"
    } else {
        "draw((root / 'development-screen.txt').read_text() if pending.count(b'\\r') == 1 else 'SYNTHETIC AFTER CHANNEL CONFIRM; NO MORE INPUT\\n')"
    };
    let script = script
        .replace("draw(yes if mode == 'preselected-yes' else text)", initial)
        .replace("draw('SYNTHETIC AFTER TRUST; NO FURTHER KEYS\\n')", after);
    fs::write(&program, script).unwrap();
    program
}

#[test]
fn passive_known_trust_sends_no_startup_keys_at_both_widths() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for width in [100, 140] {
        let root = TempDir::new("g12-passive-known-trust").unwrap();
        let program = trust_worker(root.path(), width, "normal");
        let out = root.path().join("evidence");
        let opts = options(&program, &out, width);
        capture::run(&opts, &agend()).unwrap();
        assert!(!root.path().join("input-received").exists());
        let recorded = fs::read_to_string(out.join("screens.jsonl")).unwrap();
        assert!(
            recorded.contains("No, exit"),
            "missing native trust screen: {recorded}"
        );
        let result: Value =
            serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap()).unwrap();
        assert_eq!(result["input_sent"], false);
        assert_eq!(result["terminal_input_operations_started"], 0);
    }
}

#[test]
fn separate_development_opt_in_sends_exactly_three_inputs_at_both_widths() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for width in [100, 140] {
        let root = TempDir::new("g12-development-control").unwrap();
        let program = development_worker(root.path(), width, "normal");
        let out = root.path().join("evidence");
        let mut opts = controlled_options(&program, &out, width);
        opts.accept_development_channels = true;
        capture::run(&opts, &agend()).unwrap();
        assert_eq!(
            fs::read(root.path().join("input-received")).unwrap(),
            b"\x1b[B\r\r"
        );
        let result: Value =
            serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap()).unwrap();
        assert_eq!(result["terminal_input_operations_started"], 3);
        assert_eq!(result["terminal_input_operations_completed"], 3);
        assert_eq!(result["startup"], "not_assessed");
        let recorded = fs::read_to_string(out.join("screens.jsonl")).unwrap();
        assert!(recorded.contains("SYNTHETIC AFTER CHANNEL CONFIRM"));
        assert!(recorded.contains("development-enter"));
        assert!(
            !Path::new(
                fs::read_to_string(root.path().join("workspace"))
                    .unwrap()
                    .trim()
            )
            .exists()
        );
    }
}

#[test]
fn development_confirmation_rejects_other_servers_selection_and_incomplete_warning() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for mode in [
        "foreign-prefix",
        "foreign-extra",
        "duplicate-header",
        "selected-exit",
        "missing-warning",
        "premature",
    ] {
        let root = TempDir::new("g12-development-refuse").unwrap();
        let program = development_worker(root.path(), 100, mode);
        let out = root.path().join("evidence");
        let mut opts = controlled_options(&program, &out, 100);
        opts.accept_development_channels = true;
        assert!(capture::run(&opts, &agend()).is_err(), "{mode}");
        let input = fs::read(root.path().join("input-received")).unwrap_or_default();
        assert_eq!(
            input,
            if mode == "premature" {
                b"".as_slice()
            } else {
                b"\x1b[B\r".as_slice()
            },
            "{mode}"
        );
        assert!(
            !Path::new(
                fs::read_to_string(root.path().join("workspace"))
                    .unwrap()
                    .trim()
            )
            .exists()
        );
    }
}

#[test]
fn development_confirmation_rejects_contradictory_or_repeated_menu() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for width in [100, 140] {
        for mode in [
            "selected-exit-plus-decoy",
            "duplicate-selected-local",
            "duplicate-selected-exit",
        ] {
            let root = TempDir::new("g12-development-conflicting").unwrap();
            let program = development_worker(root.path(), width, mode);
            let out = root.path().join("evidence");
            let mut opts = controlled_options(&program, &out, width);
            opts.accept_development_channels = true;
            assert!(capture::run(&opts, &agend()).is_err(), "{width}: {mode}");
            assert_eq!(
                fs::read(root.path().join("input-received")).unwrap(),
                b"\x1b[B\r",
                "{width}: {mode}"
            );
            let result: Value =
                serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap())
                    .unwrap();
            assert_eq!(result["terminal_input_operations_started"], 2);
            assert_eq!(result["terminal_input_operations_completed"], 2);
            assert!(
                !Path::new(
                    fs::read_to_string(root.path().join("workspace"))
                        .unwrap()
                        .trim()
                )
                .exists()
            );
        }
    }
}

#[test]
fn foreign_frame_after_trust_completion_never_authorizes_development_input() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for width in [100, 140] {
        for field in ["instance_id", "view_id"] {
            let root = TempDir::new("g12-capture-identity").unwrap();
            let program = development_worker(root.path(), width, "normal");
            let proxy = root.path().join("native-identity-proxy");
            let script = include_str!("common/claude_capture_identity_proxy.py")
                .replace("__REAL__", &serde_json::to_string(&agend()).unwrap())
                .replace("__FIELD__", &serde_json::to_string(field).unwrap())
                .replace("__PHASE__", "\"outer\"");
            fs::write(&proxy, format!("#!/usr/bin/env python3\n{script}")).unwrap();
            fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).unwrap();
            let out = root.path().join("evidence");
            let mut opts = controlled_options(&program, &out, width);
            opts.accept_development_channels = true;
            assert!(capture::run(&opts, &proxy).is_err(), "{width}: {field}");
            assert_eq!(
                fs::read(root.path().join("input-received")).unwrap(),
                b"\x1b[B\r"
            );
            let result: Value =
                serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap())
                    .unwrap();
            assert_eq!(result["terminal_input_operations_started"], 2);
            assert_eq!(result["terminal_input_operations_completed"], 2);
            let events: Vec<Value> = fs::read_to_string(root.path().join("identity-wire-events"))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            let ack = events
                .iter()
                .position(|event| event["phase"] == "trust_ack_forwarded")
                .unwrap();
            let injected = events
                .iter()
                .position(|event| event["phase"] == "frame_injected")
                .unwrap();
            assert!(
                ack < injected,
                "must test the outer loop after the native trust ACK"
            );
            assert!(
                events
                    .iter()
                    .all(|event| event["request_id"] != "capture-trust-development-enter")
            );
            assert!(
                !Path::new(
                    fs::read_to_string(root.path().join("workspace"))
                        .unwrap()
                        .trim()
                )
                .exists()
            );
        }
    }
}

#[test]
fn inconsistent_frame_before_resize_ack_stops_without_input() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for width in [100, 140] {
        for field in ["instance_id", "view_id", "generation", "size"] {
            let root = TempDir::new("g12-capture-resize-identity").unwrap();
            let program = development_worker(root.path(), width, "normal");
            let proxy = root.path().join("native-identity-proxy");
            let script = include_str!("common/claude_capture_identity_proxy.py")
                .replace("__REAL__", &serde_json::to_string(&agend()).unwrap())
                .replace("__FIELD__", &serde_json::to_string(field).unwrap())
                .replace("__PHASE__", "\"resize\"");
            fs::write(&proxy, format!("#!/usr/bin/env python3\n{script}")).unwrap();
            fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).unwrap();
            let out = root.path().join("evidence");
            let mut opts = controlled_options(&program, &out, width);
            opts.accept_development_channels = true;
            assert!(capture::run(&opts, &proxy).is_err(), "{width}: {field}");
            assert!(
                fs::read(root.path().join("input-received"))
                    .unwrap_or_default()
                    .is_empty()
            );
            let result: Value =
                serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap())
                    .unwrap();
            assert_eq!(result["terminal_input_operations_started"], 0);
            assert_eq!(result["terminal_input_operations_completed"], 0);
            assert_eq!(result["input_sent"], false);
            let events: Vec<Value> = fs::read_to_string(root.path().join("identity-wire-events"))
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert!(
                events
                    .iter()
                    .any(|event| event["phase"] == "frame_injected")
            );
            assert!(events.iter().all(|event| {
                !event["request_id"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("capture-trust-")
            }));
            assert!(
                !Path::new(
                    fs::read_to_string(root.path().join("identity-native-home"))
                        .unwrap()
                        .trim()
                )
                .exists()
            );
        }
    }
}

#[test]
fn development_confirmation_has_no_default_grant_or_replay() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    let root = TempDir::new("g12-development-default").unwrap();
    let program = development_worker(root.path(), 100, "normal");
    let out = root.path().join("default-evidence");
    capture::run(&controlled_options(&program, &out, 100), &agend()).unwrap();
    assert_eq!(
        fs::read(root.path().join("input-received")).unwrap(),
        b"\x1b[B\r"
    );
    fs::remove_file(root.path().join("input-received")).unwrap();
    let mut opts = controlled_options(&program, &root.path().join("bad-preflight"), 100);
    opts.accept_development_channels = true;
    opts.accept_workspace_trust = false;
    assert!(capture::run(&opts, &agend()).is_err());
    assert!(!opts.out.exists());
    assert!(!root.path().join("input-received").exists());

    let program = development_worker(root.path(), 100, "unchanged");
    let mut opts = controlled_options(&program, &root.path().join("unchanged-evidence"), 100);
    opts.accept_development_channels = true;
    capture::run(&opts, &agend()).unwrap();
    assert_eq!(
        fs::read(root.path().join("input-received")).unwrap(),
        b"\x1b[B\r\r"
    );
}

#[test]
fn trust_probe_sends_only_down_and_confirmed_yes_enter_at_both_widths() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for width in [100, 140] {
        let root = TempDir::new("g12-trust-controls").unwrap();
        let program = trust_worker(root.path(), width, "normal");
        let out = root.path().join("evidence");
        capture::run(&controlled_options(&program, &out, width), &agend()).unwrap();
        assert_eq!(
            fs::read(root.path().join("input-received")).unwrap(),
            b"\x1b[B\r"
        );
        let result: Value =
            serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap()).unwrap();
        assert_eq!(result["terminal_input_operations_started"], 2);
        assert_eq!(result["terminal_input_operations_completed"], 2);
        assert_eq!(result["production_daemon_key_path_tested"], false);
        assert_eq!(result["startup"], "not_assessed");
        let recorded = fs::read_to_string(out.join("screens.jsonl")).unwrap();
        assert!(recorded.contains("SYNTHETIC AFTER TRUST"));
        assert!(recorded.contains("workspace-trust-control"));
        assert!(
            !Path::new(
                fs::read_to_string(root.path().join("workspace"))
                    .unwrap()
                    .trim()
            )
            .exists()
        );
    }
}

#[test]
fn unknown_or_foreign_or_preselected_trust_sends_nothing() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for mode in ["unknown", "wrong-path", "preselected-yes"] {
        let root = TempDir::new("g12-trust-refuse").unwrap();
        let program = trust_worker(root.path(), 100, mode);
        let out = root.path().join("evidence");
        assert!(capture::run(&controlled_options(&program, &out, 100), &agend()).is_err());
        assert!(!root.path().join("input-received").exists(), "{mode}");
        let result: Value =
            serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap()).unwrap();
        assert_eq!(result["input_sent"], false);
        assert!(
            !Path::new(
                fs::read_to_string(root.path().join("workspace"))
                    .unwrap()
                    .trim()
            )
            .exists()
        );
    }
}

#[test]
fn trust_path_must_be_bound_exactly_to_one_workspace_header_at_both_widths() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for width in [100, 140] {
        for mode in ["prefix-path", "path-elsewhere", "duplicate-path-header"] {
            let root = TempDir::new("g12-trust-path-binding").unwrap();
            let program = trust_worker(root.path(), width, mode);
            let out = root.path().join("evidence");
            assert!(capture::run(&controlled_options(&program, &out, width), &agend()).is_err());
            assert!(
                !root.path().join("input-received").exists(),
                "{width}: {mode}"
            );
            let result: Value =
                serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap())
                    .unwrap();
            assert_eq!(result["input_sent"], false, "{width}: {mode}");
            assert_eq!(result["terminal_input_operations_started"], 0);
            assert!(
                !Path::new(
                    fs::read_to_string(root.path().join("workspace"))
                        .unwrap()
                        .trim()
                )
                .exists()
            );
        }
    }
}

#[test]
fn trust_confirmation_rejects_extra_or_repeated_selections_at_both_widths() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for width in [100, 140] {
        for mode in [
            "no-extra-exit",
            "no-extra-duplicate",
            "yes-extra-exit",
            "yes-extra-duplicate",
        ] {
            let root = TempDir::new("g12-trust-complete-menu").unwrap();
            let program = trust_worker(root.path(), width, mode);
            let out = root.path().join("evidence");
            assert!(
                capture::run(&controlled_options(&program, &out, width), &agend()).is_err(),
                "{width}: {mode}"
            );
            let expected = if mode.starts_with("no-") {
                b"".as_slice()
            } else {
                b"\x1b[B".as_slice()
            };
            assert_eq!(
                fs::read(root.path().join("input-received")).unwrap_or_default(),
                expected,
                "{width}: {mode}"
            );
            let result: Value =
                serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap())
                    .unwrap();
            assert_eq!(
                result["terminal_input_operations_started"],
                if expected.is_empty() { 0 } else { 1 }
            );
            assert!(
                !Path::new(
                    fs::read_to_string(root.path().join("workspace"))
                        .unwrap()
                        .trim()
                )
                .exists()
            );
        }
    }
}

#[test]
fn prompt_stability_survives_delayed_receiver_and_resets_for_unknown() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    for width in [100, 140] {
        for mode in ["delayed-input", "transient-unknown"] {
            let root = TempDir::new("g12-trust-delayed-input").unwrap();
            let program = trust_worker(root.path(), width, mode);
            let out = root.path().join("evidence");
            capture::run(&controlled_options(&program, &out, width), &agend()).unwrap();
            assert_eq!(
                fs::read(root.path().join("input-received")).unwrap(),
                b"\x1b[B\r"
            );
            if mode == "transient-unknown" {
                let elapsed: f64 = fs::read_to_string(root.path().join("first-input-after-stable"))
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!(
                    elapsed >= 0.9,
                    "unknown frame must restart dwell: {elapsed}"
                );
            }
            let result: Value =
                serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap())
                    .unwrap();
            assert_eq!(result["terminal_input_operations_completed"], 2);
            assert!(
                !Path::new(
                    fs::read_to_string(root.path().join("workspace"))
                        .unwrap()
                        .trim()
                )
                .exists()
            );
        }
    }
}

#[test]
fn private_cleanup_identity_matches_native_session_and_removed_workspace() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    let root = TempDir::new("g12-cleanup-identity").unwrap();
    let program = worker(root.path());
    let out = root.path().join("evidence");
    capture::run(&options(&program, &out, 100), &agend()).unwrap();
    let path = out.join("cleanup-identity.json");
    let identity: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(identity["format"], "startup-cleanup-v1");
    assert_eq!(identity["instance_id"], "g12-startup-capture");
    let argv = fs::read_to_string(root.path().join("argv")).unwrap();
    let args: Vec<_> = argv.lines().collect();
    let session = args
        .windows(2)
        .find(|pair| pair[0] == "--session-id")
        .unwrap()[1];
    assert_eq!(identity["session_id"], session);
    assert!(!Path::new(identity["home"].as_str().unwrap()).exists());
    assert!(!Path::new(identity["workspace"].as_str().unwrap()).exists());
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(identity.as_object().unwrap().len(), 5);
}

#[test]
fn unchanged_no_selection_never_replays_down_or_sends_enter() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    let root = TempDir::new("g12-trust-stuck").unwrap();
    let program = trust_worker(root.path(), 100, "stuck-no");
    let out = root.path().join("evidence");
    assert!(capture::run(&controlled_options(&program, &out, 100), &agend()).is_err());
    assert_eq!(
        fs::read(root.path().join("input-received")).unwrap(),
        b"\x1b[B"
    );
    let result: Value =
        serde_json::from_str(&fs::read_to_string(out.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["terminal_input_operations_started"], 1);
    assert_eq!(result["terminal_input_operations_completed"], 1);
}

#[test]
fn trust_control_requires_the_recorded_version_and_dimensions_before_launch() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
    let root = TempDir::new("g12-trust-preflight").unwrap();
    let program = trust_worker(root.path(), 100, "normal");
    for (version, columns, rows) in [
        ("unknown", 100, 24),
        ("2.1.284", 80, 24),
        ("2.1.284", 100, 25),
    ] {
        let out = root.path().join("evidence");
        let mut opts = controlled_options(&program, &out, 100);
        opts.version_label = version.into();
        opts.size.columns = columns;
        opts.size.rows = rows;
        assert!(capture::run(&opts, &agend()).is_err());
        assert!(!out.exists());
        assert!(!root.path().join("workspace").exists());
    }
}
#[test]
fn real_holder_captures_two_widths_without_any_input_and_cleans_up() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
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
        assert!(
            rendered.contains("NATIVE STARTUP CAPTURE"),
            "missing native banner: {rendered}"
        );
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
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
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
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
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
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
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
        "native wrapped email was not redacted: {stored}"
    );
}

#[test]
fn a_soft_wrapped_bearer_prefix_is_refused_before_writing_the_frame() {
    let _lab = NATIVE_LAB.lock().unwrap_or_else(|error| error.into_inner());
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
