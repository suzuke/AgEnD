//! Unit tests for `kill_guard`.

use super::*;

fn argv(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

fn names(pid: u32) -> Option<String> {
    match pid {
        100 => Some("/usr/local/bin/agend".into()),
        200 => Some("agend".into()),
        300 => Some("/usr/bin/sleep".into()),
        _ => None,
    }
}

fn code(tool: Tool, cmd: &str) -> &'static str {
    match classify(tool, &argv(cmd), &names) {
        Ok(()) => "allow",
        Err(r) => r.code,
    }
}

#[test]
fn pattern_kills_are_refused_unless_informational() {
    for cmd in [
        "-f resume --last",
        "agend",
        "-u me",
        "-- -weird",
        "-9 codex",
    ] {
        assert_eq!(code(Tool::Pkill, cmd), "pattern_kill", "{cmd}");
        assert_eq!(code(Tool::Killall, cmd), "pattern_kill", "{cmd}");
    }
    for cmd in ["", "--help", "-l", "--version"] {
        assert_eq!(code(Tool::Pkill, cmd), "allow", "{cmd}");
    }
    let r = classify(Tool::Pkill, &argv("-f my-server"), &names).unwrap_err();
    assert!(r.next.contains("pgrep -fl -- 'my-server'"), "{}", r.next);
}

#[test]
fn kill_refuses_agend_processes_and_groups() {
    assert_eq!(code(Tool::Kill, "100"), "kill_protected");
    assert_eq!(code(Tool::Kill, "-9 300 200"), "kill_protected");
    assert_eq!(code(Tool::Kill, "-s TERM 100"), "kill_protected");
    assert_eq!(code(Tool::Kill, "--signal=TERM 100"), "kill_protected");
    assert_eq!(code(Tool::Kill, "-- -1"), "group_kill");
    assert_eq!(code(Tool::Kill, "-9 -1234"), "group_kill");
    for cmd in ["300", "-9 300", "999", "-l", "-l 9", "--", ""] {
        assert_eq!(code(Tool::Kill, cmd), "allow", "{cmd}");
    }
}

/// T9 round 1: forms that reached a holder or a group before.
#[test]
fn kill_targets_are_normalised_and_deny_by_default() {
    let k = |args: &[&str]| {
        let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match classify(Tool::Kill, &v, &names) {
            Ok(()) => "allow",
            Err(r) => r.code,
        }
    };
    for spaced in [" 100", "100 ", "\t100\n", "+100", "0100", "000200"] {
        assert_eq!(k(&[spaced]), "kill_protected", "{spaced:?}");
        assert_eq!(k(&["-9", spaced]), "kill_protected", "{spaced:?}");
    }
    for group in [
        &["0"][..],
        &["-9", "0"],
        &["-1"],
        &["-9"],
        &["-s", "9", "-1"],
        &["-9", "--", "-300"],
        &["00"],
    ] {
        assert_eq!(k(group), "group_kill", "{group:?}");
    }
    for bad in [
        &["agend"][..],
        &["%1"],
        &["-9", "-a", "300"],
        &["--timeout", "1", "300"],
        &["1e3"],
        &["99999999999"],
    ] {
        assert_eq!(k(bad), "kill_target", "{bad:?}");
    }
}

/// T9 round 2: BSD/macOS `kill` stores `strtol()` in an `int`, so
/// 4294967295 wraps to -1 (every process you own) and an overlong
/// string saturates to `LONG_MAX`, whose low 32 bits are also -1.
/// Classifier only: nothing here sends a signal.
#[test]
fn pids_outside_the_int_range_are_refused() {
    let k = |args: &[&str]| {
        let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        match classify(Tool::Kill, &v, &names) {
            Ok(()) => "allow",
            Err(r) => r.code,
        }
    };
    for big in [
        "2147483648",
        "4294967295",
        "4294967296",
        "4294967297",
        "+4294967295",
        " 4294967295",
        "4294967295\n",
        "04294967295",
        "0000004294967295",
        "18446744073709551615",
        "9223372036854775807",
        "99999999999999999999999999999999999999999999999999",
    ] {
        assert_eq!(k(&[big]), "kill_target", "{big:?}");
        assert_eq!(k(&["-9", big]), "kill_target", "-9 {big:?}");
        assert_eq!(k(&["-s", "KILL", "--", big]), "kill_target", "{big:?}");
        assert_eq!(k(&["300", big]), "kill_target", "300 {big:?}");
    }
    // Overlong spellings of small pids are refused too: no kill needs
    // more digits than i32::MAX has.
    for long in ["00000000300", "+00000000300", "000000000000000000001"] {
        assert_eq!(k(&[long]), "kill_target", "{long:?}");
    }
    // Zero after normalisation stays a group kill.
    for zero in ["+0", " 0 ", "0000000000", "-0", "+00"] {
        assert_eq!(k(&[zero]), "group_kill", "{zero:?}");
    }
    // The boundary itself is a plain pid.
    assert_eq!(k(&["2147483647"]), "allow");
    assert_eq!(k(&["0000000300"]), "allow");
    assert!(kill_target("2147483647").is_ok());
    assert!(kill_target("2147483648").is_err());
}

#[test]
fn protected_name_is_the_basename() {
    assert!(is_protected_name("/Users/x/.cargo/bin/agend"));
    assert!(is_protected_name("agend\n"));
    assert!(!is_protected_name("agend-git"));
    assert!(!is_protected_name("/bin/sleep"));
}

#[cfg(unix)]
#[test]
fn process_name_reads_a_live_process() {
    let me = process_name(std::process::id()).expect("own process visible to ps");
    assert!(!me.is_empty());
    assert_eq!(process_name(u32::MAX - 1), None);
}
