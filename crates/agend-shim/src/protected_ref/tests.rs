//! Unit tests for `protected_ref`.

use super::*;

#[test]
fn builtins_and_extras_are_protected() {
    let p = ProtectedRefs::new(&["release/*".into(), "refs/tags/v1".into()]);
    for r in [
        "refs/heads/main",
        "refs/heads/master",
        "refs/heads/release/2026",
        "refs/tags/v1",
    ] {
        assert!(p.is_protected(r), "{r}");
    }
    for r in [
        "refs/heads/agend/t-1/x",
        "refs/heads/mainline",
        "refs/tags/v2",
        "refs/remotes/origin/main",
    ] {
        assert!(!p.is_protected(r), "{r}");
    }
}
