//! Unit tests for `the crate root`.

use super::*;

#[test]
fn dispatches_on_basename() {
    assert_eq!(Tool::from_argv0(OsStr::new("git")), Some(Tool::Git));
    assert_eq!(
        Tool::from_argv0(OsStr::new("/home/u/.agend/bin/git")),
        Some(Tool::Git)
    );
    assert_eq!(Tool::from_argv0(OsStr::new("kill")), Some(Tool::Kill));
    assert_eq!(Tool::from_argv0(OsStr::new("killall")), Some(Tool::Killall));
    assert_eq!(Tool::from_argv0(OsStr::new("./pkill")), Some(Tool::Pkill));
    assert_eq!(
        Tool::from_argv0(OsStr::new("/h/.agend/hooks/pre-push")),
        Some(Tool::Hook("pre-push"))
    );
}

#[test]
fn other_names_are_not_the_shim() {
    for name in [
        "agend",
        "/usr/local/bin/agend",
        "git-lfs",
        "gitk",
        "",
        "/",
        "update",
    ] {
        assert_eq!(Tool::from_argv0(OsStr::new(name)), None, "{name}");
    }
}

#[test]
fn refusals_render_reason_and_next_step() {
    let r = Refusal::new("x", "because", "do this");
    assert_eq!(
        r.render("git checkout main"),
        vec![
            "agend-shim: refused `git checkout main`",
            "agend-shim: why: because",
            "agend-shim: next step: do this",
        ]
    );
}
