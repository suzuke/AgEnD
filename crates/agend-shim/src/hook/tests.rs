//! Unit tests for `hook`.

use super::*;
use crate::binding::SNAPSHOT_VERSION;
use agend_core::model::work_branch;

const Z: &str = "0000000000000000000000000000000000000000";
const A: &str = "d394e1a7a98a36e1876a728a1d38a9d3a40cd3d3";

fn snap(binding: Option<Binding>) -> Snapshot {
    Snapshot {
        version: SNAPSHOT_VERSION,
        instance: "dev-1".into(),
        source_repo: Some("/repo".into()),
        protected_refs: vec!["release".into()],
        binding,
    }
}

fn work() -> Snapshot {
    snap(Some(Binding::Work {
        task_id: "t-1".into(),
        branch: work_branch("t-1", "fix"),
        worktree: "/wt".into(),
    }))
}

fn code(r: Result<(), Refusal>) -> &'static str {
    r.map_or_else(|r| r.code, |()| "ok")
}

#[test]
fn transaction_refs_outside_the_binding_are_refused() {
    let s = work();
    let u = |new: &str, r: &str| code(check_update(Ok(&s), new, r));
    for r in ["refs/heads/main", "refs/heads/master", "refs/heads/release"] {
        assert_eq!(u(A, r), "protected_ref", "{r}");
        assert_eq!(u(Z, r), "protected_ref", "delete {r}");
    }
    for r in ["refs/heads/feat/x", "refs/heads/agend/t-2/y"] {
        assert_eq!(u(A, r), "ref_not_yours", "{r}");
    }
    assert_eq!(
        u(Z, "refs/heads/agend/t-1/fix"),
        "ref_not_yours",
        "delete bound"
    );
    for r in [
        "HEAD",
        "refs/heads/agend/t-1/fix",
        "refs/heads/agend/t-1/scratch",
        "refs/remotes/origin/main",
        "refs/tags/v1",
        "refs/stash",
        "refs/agend/snapshots/dev-1/1-2",
        "ORIG_HEAD",
    ] {
        assert_eq!(u(A, r), "ok", "{r}");
    }
    assert_eq!(u(Z, "refs/heads/agend/t-1/scratch"), "ok");
}

#[test]
fn without_a_readable_binding_only_remote_tracking_refs_pass() {
    let e = SnapshotError::NotAnAgent;
    assert_eq!(
        code(check_update(Err(&e), A, "refs/heads/main")),
        "no_binding"
    );
    assert_eq!(
        code(check_update(Err(&e), A, "refs/heads/agend/t-1/fix")),
        "no_binding"
    );
    assert_eq!(
        code(check_update(Err(&e), A, "refs/remotes/origin/x")),
        "ok"
    );
    let unbound = snap(None);
    assert_eq!(
        code(check_update(Ok(&unbound), A, "refs/heads/agend/t-1/x")),
        "ref_not_yours"
    );
    assert_eq!(code(check_update(Ok(&unbound), A, "refs/tags/v2")), "ok");
}

#[test]
fn pushes_reach_only_the_bound_branch() {
    let s = work();
    let p = |local: &str, r: &str| code(check_push(Ok(&s), local, r));
    assert_eq!(p(A, "refs/heads/agend/t-1/fix"), "ok");
    assert_eq!(p(A, "refs/heads/main"), "protected_ref");
    assert_eq!(p(A, "refs/heads/agend/t-1/other"), "push_not_yours");
    assert_eq!(p(A, "refs/tags/v1"), "push_not_yours");
    assert_eq!(p(Z, "refs/heads/agend/t-1/fix"), "push_not_yours");
    let review = snap(Some(Binding::Review {
        task_id: "t-2".into(),
        head: A.into(),
        worktree: "/wt".into(),
    }));
    assert_eq!(
        code(check_push(Ok(&review), A, "refs/heads/x")),
        "push_not_yours"
    );
}

#[test]
fn only_the_prepared_phase_is_checked() {
    let s = work();
    let line = format!("{A} {A} refs/heads/main\n");
    let args = |p: &str| vec![p.to_string()];
    let rt = "reference-transaction";
    assert!(check(rt, &args("prepared"), &line, Ok(&s)).is_err());
    assert!(check(rt, &args("committed"), &line, Ok(&s)).is_ok());
    let push = format!("refs/heads/x {A} refs/heads/main {Z}\n");
    let (what, r) = check("pre-push", &args("origin"), &push, Ok(&s)).unwrap_err();
    assert_eq!(
        (what.as_str(), r.code),
        ("push to refs/heads/main on origin", "protected_ref")
    );
    // An unknown line shape (a future git format) is refused, not skipped.
    let odd = format!("{A} {A} refs/heads/main extra\n");
    let (_, r) = check(rt, &args("prepared"), &odd, Ok(&s)).unwrap_err();
    assert_eq!(r.code, "hook_input");
    assert!(check("pre-push", &args("origin"), "x y\n", Ok(&s)).is_err());
    assert!(check(rt, &args("prepared"), "", Ok(&s)).is_ok());
}
