//! Decision tests: argv (+ a fake `Probe`) → run / route / snapshot /
//! refuse. Real-git behaviour is in `tests/`.

use super::*;
use crate::binding::SNAPSHOT_VERSION;
use agend_core::model::work_branch;
use std::cell::RefCell;

fn argv(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

fn work() -> Snapshot {
    Snapshot {
        version: SNAPSHOT_VERSION,
        instance: "dev-1".into(),
        source_repo: Some("/repo".into()),
        protected_refs: vec!["release".into()],
        binding: Some(Binding::Work {
            task_id: "t-1".into(),
            branch: work_branch("t-1", "fix"),
            // An existing directory, so `worktree_missing` does not fire.
            worktree: std::env::temp_dir(),
        }),
    }
}

fn review() -> Snapshot {
    Snapshot {
        binding: Some(Binding::Review {
            task_id: "t-2".into(),
            head: "abc123".into(),
            worktree: std::env::temp_dir(),
        }),
        ..work()
    }
}

fn unbound() -> Snapshot {
    Snapshot {
        binding: None,
        ..work()
    }
}

/// Answers from fixed tables; records config questions.
#[derive(Default)]
struct Fake {
    symrefs: Vec<(&'static str, &'static str)>,
    config: Vec<(&'static str, &'static str)>,
    team_repo: bool,
    team_remotes: Vec<&'static str>,
    asked: RefCell<Vec<String>>,
}

impl Probe for Fake {
    fn symref_target(&self, full_ref: &str) -> Option<String> {
        self.symrefs
            .iter()
            .find(|(n, _)| *n == full_ref)
            .map(|(_, t)| t.to_string())
    }
    fn config(&self, regex: &str) -> Vec<(String, String)> {
        self.asked.borrow_mut().push(regex.to_string());
        let key_part = regex.trim_start_matches('^').trim_end_matches('$');
        self.config
            .iter()
            .filter(|(k, _)| match key_part {
                r"remote\..*\.fetch" => k.starts_with("remote.") && k.ends_with(".fetch"),
                other => k.eq_ignore_ascii_case(&other.replace('\\', "")),
            })
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }
    fn is_team_repo(&self) -> bool {
        self.team_repo
    }
    fn is_team_remote(&self, dest: &str) -> bool {
        self.team_remotes.contains(&dest)
    }
}

/// The bound worktree of `work()`, as git reports it (canonical).
fn bound_wt() -> PathBuf {
    std::fs::canonicalize(std::env::temp_dir()).unwrap()
}

/// What git would answer at `loc`; at `Worktree`, the bound worktree's own
/// git dir and work tree.
fn resolved_at(loc: Location) -> Option<Resolved> {
    let r = |g: PathBuf, c: &str, w: Option<PathBuf>| {
        Some(Resolved {
            git_dir: g,
            common_dir: c.into(),
            work_tree: w,
        })
    };
    match loc {
        Location::Worktree => r(
            bound_wt().join("fake-gitdir"),
            "/repo/.git",
            Some(bound_wt()),
        ),
        Location::Canonical => r("/repo/.git".into(), "/repo/.git", Some("/repo".into())),
        Location::OtherWorktree => r(
            "/repo/.git/worktrees/t-9".into(),
            "/repo/.git",
            Some("/wt/t-9".into()),
        ),
        Location::Foreign => r(
            "/scratch/.git".into(),
            "/scratch/.git",
            Some("/scratch".into()),
        ),
        Location::NoRepo | Location::Unknown => None,
    }
}

fn run_full(
    snap: Result<&Snapshot, &SnapshotError>,
    loc: Location,
    resolved: Option<&Resolved>,
    env: &GitEnv,
    probe: &Fake,
    cmd: &str,
) -> Decision {
    let args = argv(cmd);
    let parsed = parse(&args);
    let protected = ProtectedRefs::new(snap.map(|s| s.protected_refs.as_slice()).unwrap_or(&[]));
    classify(&Input {
        args: &parsed,
        snapshot: snap,
        location: loc,
        resolved,
        env,
        protected: &protected,
        dir: Path::new("/somewhere"),
        probe,
    })
}

fn run_with(
    snap: Result<&Snapshot, &SnapshotError>,
    loc: Location,
    env: &GitEnv,
    probe: &Fake,
    cmd: &str,
) -> Decision {
    run_full(snap, loc, resolved_at(loc).as_ref(), env, probe, cmd)
}

fn decide_at(snap: Result<&Snapshot, &SnapshotError>, loc: Location, cmd: &str) -> Decision {
    run_with(snap, loc, &GitEnv::default(), &Fake::default(), cmd)
}

fn decide(snap: &Snapshot, cmd: &str) -> Decision {
    decide_at(Ok(snap), Location::Worktree, cmd)
}

fn with_probe(probe: &Fake, cmd: &str) -> Decision {
    run_with(
        Ok(&work()),
        Location::Worktree,
        &GitEnv::default(),
        probe,
        cmd,
    )
}

fn code(d: &Decision) -> &'static str {
    match d {
        Decision::Refuse(r) => r.code,
        Decision::Run { .. } => "run",
    }
}

fn snapshot_of(d: &Decision) -> Option<&'static str> {
    match d {
        Decision::Run { snapshot, .. } => *snapshot,
        Decision::Refuse(_) => None,
    }
}

fn assert_codes(snap: &Snapshot, want: &str, cmds: &[&str]) {
    for cmd in cmds {
        assert_eq!(code(&decide(snap, cmd)), want, "git {cmd}");
    }
}

#[test]
fn globals_are_split_from_the_subcommand() {
    let g = parse(&argv("-C /a -c x=y --no-pager -C b commit -m hi"));
    assert_eq!(g.sub.as_deref(), Some("commit"));
    assert_eq!(g.rest, argv("-m hi"));
    assert_eq!(g.chdirs, argv("/a b"));
    assert_eq!(g.retarget_indexes, vec![0, 1, 5, 6]);
    assert_eq!(g.config_keys, ["x"]);
    let g = parse(&argv("--git-dir=/x/.git --work-tree /y status"));
    assert_eq!(g.git_dir.as_deref(), Some("/x/.git"));
    assert_eq!(g.work_tree.as_deref(), Some("/y"));
    assert_eq!(g.sub.as_deref(), Some("status"));
    let g = parse(&argv(
        "--config-env=core.worktree=WT --config-env a.b=V -c flag.only log",
    ));
    assert_eq!(g.config_keys, ["core.worktree", "a.b", "flag.only"]);
    assert!(parse(&argv("--version")).info_only);
    assert_eq!(parse(&argv("")).sub, None);
    // An unknown global becomes the "subcommand" and is refused.
    assert_eq!(parse(&argv("--frob push")).sub.as_deref(), Some("--frob"));
}

#[test]
fn bound_writes_route_into_the_worktree() {
    let s = work();
    for loc in [
        Location::NoRepo,
        Location::Canonical,
        Location::OtherWorktree,
    ] {
        match decide_at(Ok(&s), loc, "commit -m x") {
            Decision::Run { route, .. } => {
                assert_eq!(route.as_deref(), Some(std::env::temp_dir().as_path()))
            }
            other => panic!("{loc:?}: {other:?}"),
        }
    }
    assert_eq!(decide(&s, "commit -m x"), Decision::pass());
}

#[test]
fn reads_route_only_when_bound_and_outside() {
    let s = work();
    let d = decide_at(Ok(&s), Location::Canonical, "status");
    assert!(matches!(
        d,
        Decision::Run {
            route: Some(_),
            note: Some(_),
            ..
        }
    ));
    let d = decide_at(Ok(&unbound()), Location::Canonical, "status");
    assert_eq!(d, Decision::pass());
    let err = SnapshotError::NotAnAgent;
    assert_eq!(
        decide_at(Err(&err), Location::Unknown, "log"),
        Decision::pass()
    );
}

#[test]
fn mutations_need_a_binding() {
    let err = SnapshotError::Missing("/h/bindings/dev-1.json".into());
    assert_eq!(
        code(&decide_at(Err(&err), Location::Unknown, "commit -m x")),
        "no_binding"
    );
    assert_eq!(
        code(&decide_at(Ok(&unbound()), Location::NoRepo, "commit -m x")),
        "unbound"
    );
    assert_eq!(
        code(&decide_at(Ok(&unbound()), Location::Canonical, "add .")),
        "canonical_checkout"
    );
}

#[test]
fn worktree_lifecycle_is_refused_even_when_bound() {
    for cmd in ["worktree add ../x", "worktree remove x", "worktree prune"] {
        assert_eq!(code(&decide(&work(), cmd)), "worktree_managed", "{cmd}");
        assert_eq!(code(&decide(&unbound(), cmd)), "worktree_managed", "{cmd}");
    }
    assert_eq!(code(&decide(&work(), "worktree list")), "run");
}

#[test]
fn branch_switches_are_refused() {
    let s = work();
    assert_codes(
        &s,
        "branch_switch",
        &[
            "checkout main",
            "checkout feature",
            "checkout -",
            "checkout --detach",
            "checkout abc123",
            "checkout HEAD~1",
            "checkout src/lib.rs",
            "switch main",
            "switch -",
            "switch --detach HEAD~1",
            "symbolic-ref HEAD refs/heads/main",
            "update-ref --no-deref HEAD abc123",
            "rebase main feature",
            "rebase --root feature",
        ],
    );
    let Decision::Refuse(r) = decide(&s, "checkout main") else {
        unreachable!()
    };
    assert!(r.reason.contains("main is protected"), "{}", r.reason);
    assert!(r.next.contains("agend task create"), "{}", r.next);
    assert!(r.next.contains("git checkout -- <path>"), "{}", r.next);
    assert_codes(
        &s,
        "run",
        &[
            "checkout agend/t-1/fix",
            "checkout HEAD",
            "switch agend/t-1/fix",
            "checkout",
            "rebase main",
            "rebase main agend/t-1/fix",
            "rebase --continue",
        ],
    );
    assert_codes(
        &review(),
        "branch_switch",
        &["checkout abc123", "switch agend/t-1/fix"],
    );
}

/// Round 1, class 5: DWIM creation from a remote-only branch, `<x> --`
/// (which git also reads as a switch), and other side doors.
#[test]
fn leaving_the_branch_by_dwim_or_side_doors_is_refused() {
    let s = work();
    assert_codes(
        &s,
        "branch_switch",
        &[
            "checkout other-feature",
            "checkout other-feature --",
            "checkout master --",
        ],
    );
    assert_codes(
        &s,
        "branch_create",
        &[
            "checkout --track origin/other-feature",
            "checkout -t origin/x",
            "switch --track origin/x",
            "stash branch newb",
        ],
    );
    assert_eq!(
        code(&decide(&s, "rebase --update-refs main")),
        "rebase_update_refs"
    );
    let on = Fake {
        config: vec![("rebase.updaterefs", "true")],
        ..Fake::default()
    };
    assert_eq!(code(&with_probe(&on, "rebase main")), "rebase_update_refs");
    assert_eq!(
        code(&with_probe(&on, "rebase --no-update-refs main")),
        "run"
    );
    assert_eq!(code(&with_probe(&on, "rebase --continue")), "run");
    assert_eq!(code(&with_probe(&on, "pull origin")), "rebase_update_refs");
    assert_eq!(code(&with_probe(&on, "pull --no-rebase origin")), "run");
}

#[test]
fn branch_creation_is_limited_to_the_own_namespace() {
    let s = work();
    assert_codes(
        &s,
        "branch_create",
        &[
            "checkout -b feat/x",
            "checkout -bfeat/x",
            "checkout -b agend/t-1/x",
            "checkout --orphan x",
            "switch -c x",
            "switch -cx",
            "switch --create=x",
            "switch --create x",
            "branch feat/x",
            "branch agend/t-2/x",
        ],
    );
    assert_eq!(code(&decide(&s, "branch agend/t-1/scratch")), "run");
    assert_eq!(
        code(&decide(&review(), "branch agend/t-2/x")),
        "branch_create"
    );
    for cmd in [
        "branch",
        "branch -a",
        "branch --list agend/*",
        "branch -vv",
        "branch --contains HEAD",
        "branch --sort=-committerdate",
    ] {
        assert_eq!(code(&decide(&unbound(), cmd)), "run", "{cmd}");
    }
    assert_codes(
        &s,
        "branch_rename",
        &["branch -m other", "branch --move other", "branch -c x"],
    );
    assert_eq!(code(&decide(&s, "branch -D feat/x")), "branch_delete");
    assert_eq!(
        code(&decide(&s, "branch -d agend/t-1/fix")),
        "branch_delete"
    );
    assert_eq!(code(&decide(&s, "branch -d agend/t-1/scratch")), "run");
    assert_eq!(code(&decide(&s, "branch -u origin/agend/t-1/fix")), "run");
}

#[test]
fn protected_refs_are_refused_for_bound_agents() {
    let s = work();
    assert_codes(
        &s,
        "protected_ref",
        &[
            "update-ref refs/heads/main abc123",
            "update-ref main abc123",
            "update-ref -d refs/heads/master",
            "update-ref -m msg refs/heads/release abc123",
            "push . HEAD:main",
            "push origin HEAD:main",
            "push origin +agend/t-1/fix:refs/heads/master",
            "push origin --delete main",
            "push origin :master",
            "branch -f main abc123",
            "branch -D main",
            "fetch origin main:main",
            "fetch . HEAD:release",
            "pull . HEAD:master",
        ],
    );
    assert_eq!(code(&decide(&s, "update-ref --stdin")), "update_ref_stdin");
    assert_codes(
        &s,
        "push_scope",
        &[
            "push --all origin",
            "push --mirror origin",
            "push --tags origin",
            "push --repo=x HEAD:refs/heads/agend/t-1/fix",
        ],
    );
    assert_codes(
        &s,
        "ref_not_yours",
        &[
            "push origin HEAD:feat/x",
            "push origin HEAD:HEAD",
            "push origin refs/heads/*:refs/heads/*",
            "update-ref refs/heads/agend/t-2/x abc",
            "fetch origin main:feat/x",
            "push -fd origin agend/t-1/fix",
        ],
    );
    assert_codes(
        &s,
        "run",
        &[
            "push origin HEAD:agend/t-1/fix",
            "push origin HEAD:refs/heads/agend/t-1/fix",
            "push -u origin HEAD:refs/heads/agend/t-1/fix",
            "push --force-with-lease origin HEAD:refs/heads/agend/t-1/fix",
            "push origin HEAD:refs/heads/agend/t-1/fix --no-verify",
            "push origin :refs/heads/agend/t-1/scratch",
            "update-ref refs/heads/agend/t-1/fix abc123",
            "update-ref HEAD abc123",
            "fetch origin",
            "fetch --all --prune",
            "fetch origin main",
            "fetch origin main:refs/remotes/origin/main",
            "fetch origin tag v9",
            "pull",
            "pull origin main",
        ],
    );
    assert_eq!(code(&decide(&review(), "push")), "review_readonly");
}

/// Round 1, class 1: destinations git takes from config.
#[test]
fn implicit_push_destinations_are_refused() {
    let s = work();
    assert_codes(
        &s,
        "push_explicit",
        &[
            "push",
            "push origin",
            "push origin HEAD",
            "push -u origin agend/t-1/fix",
            "push origin agend/t-1/fix",
            "push origin agend/t-1/fix:",
        ],
    );
    let Decision::Refuse(r) = decide(&s, "push -u upstream") else {
        unreachable!()
    };
    assert!(
        r.next
            .contains("git push upstream HEAD:refs/heads/agend/t-1/fix"),
        "{}",
        r.next
    );
    let cfg = |k: &'static str, v: &'static str| Fake {
        config: vec![(k, v)],
        ..Fake::default()
    };
    // Config refmaps apply to every fetch, with or without refspecs.
    for cmd in ["fetch origin", "fetch origin main", "fetch --all", "pull"] {
        let bad = cfg("remote.origin.fetch", "+refs/heads/main:refs/heads/release");
        assert_eq!(code(&with_probe(&bad, cmd)), "fetch_refmap", "{cmd}");
        let glob = cfg("remote.origin.fetch", "+refs/heads/*:refs/heads/*");
        assert_eq!(code(&with_probe(&glob, cmd)), "fetch_refmap", "{cmd}");
        let ok = cfg("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*");
        assert_eq!(code(&with_probe(&ok, cmd)), "run", "{cmd}");
    }
    let bad = cfg("remote.m.fetch", "+refs/*:refs/*");
    assert_eq!(code(&with_probe(&bad, "remote update")), "fetch_refmap");
    assert_codes(
        &s,
        "protected_ref",
        &[
            "fetch origin master --refmap +refs/heads/master:refs/heads/release",
            "fetch origin main --refmap=+refs/heads/main:refs/heads/main",
        ],
    );
    assert_codes(
        &s,
        "ref_not_yours",
        &["fetch origin main --refmap=+refs/heads/*:refs/heads/*"],
    );
    assert_codes(
        &s,
        "fetch_scope",
        &[
            "fetch --tags --force origin",
            "fetch origin +refs/tags/*:refs/tags/*",
            "fetch -u origin",
            "fetch --prune-tags origin",
            "fetch --upload-pack=x origin",
        ],
    );
    assert_eq!(code(&decide(&s, "fetch --stdin origin")), "stdin_refs");
    assert_eq!(code(&decide(&s, "fast-import")), "stdin_refs");
    // Unbound agents may fetch into remote-tracking refs only.
    assert_eq!(code(&decide(&unbound(), "fetch origin")), "run");
    assert_eq!(
        code(&decide(&unbound(), "fetch origin main:agend/t-1/x")),
        "ref_not_yours"
    );
}

#[test]
fn config_that_redirects_refs_cannot_be_set() {
    let s = work();
    assert_codes(
        &s,
        "config_override",
        &[
            "-c remote.origin.push=HEAD:refs/heads/main push origin HEAD:refs/heads/agend/t-1/fix",
            "-c push.default=matching commit -m x",
            "-c core.worktree=/repo reset --hard",
            "-c core.hooksPath=/x commit -m x",
            "--config-env=core.worktree=WT restore .",
            "-c alias.co=checkout fetch origin",
        ],
    );
    assert_eq!(
        code(&decide(&s, "-c user.name=A -c user.email=a@b commit -m x")),
        "run"
    );
    let env = |keys: Result<Vec<String>, String>| GitEnv {
        config_keys: keys,
        ..GitEnv::default()
    };
    let fake = Fake::default();
    let run = |e: &GitEnv, cmd| code(&run_with(Ok(&s), Location::Worktree, e, &fake, cmd));
    assert_eq!(
        run(&env(Ok(vec!["core.worktree".into()])), "checkout -- ."),
        "config_override"
    );
    assert_eq!(
        run(&env(Err("bad".into())), "commit -m x"),
        "config_override"
    );
    assert_eq!(
        run(&env(Ok(vec!["user.name".into()])), "commit -m x"),
        "run"
    );
    assert_eq!(run(&env(Ok(vec!["core.worktree".into()])), "status"), "run");
    assert_codes(
        &s,
        "config_write",
        &[
            "config remote.origin.push +HEAD:refs/heads/master",
            "config --add remote.origin.push x",
            "config branch.agend/t-1/fix.merge refs/heads/main",
            "config push.default upstream",
            "config core.hooksPath hooks",
            "config alias.co checkout",
            "config --global core.worktree /x",
            "config set push.default upstream",
            "config --rename-section remote.origin remote.x",
            "config -e",
        ],
    );
    assert_codes(
        &s,
        "run",
        &[
            "config user.name x",
            "config --global user.email a@b",
            "config remote.origin.push",
            "config --get-all remote.origin.fetch",
            "config -l",
            "config get user.name",
        ],
    );
}

/// Round 1, class 2: abbreviations and attached values never skip a check.
#[test]
fn abbreviated_or_unknown_options_are_refused() {
    let s = work();
    assert_codes(
        &s,
        "option_unknown",
        &[
            "push --mirr origin",
            "push --al origin",
            "push --bran origin",
            "push --prun origin",
            "push --delet origin agend/t-1/fix",
            "update-ref --stdi",
            "update-ref --std",
            "branch --mov agend/t-1/renamed",
            "branch --cop main agend/t-1/x",
            "checkout --orph orphan1",
            "checkout --deta",
            "checkout --forc",
            "switch --deta",
            "switch --discard agend/t-1/fix",
            "reset --har",
            "clean --forc -d",
            "restore --wor .",
            "config --ad remote.origin.push x",
            "fetch --refm=+refs/heads/*:refs/heads/* origin",
            "rebase --update-ref main",
            "remote add --mirr=fetch m /x",
            "checkout --force=yes",
        ],
    );
    let Decision::Refuse(r) = decide(&s, "push --mirr origin") else {
        unreachable!()
    };
    assert!(r.reason.contains("`--mirror`"), "{}", r.reason);
    // Unknown options are refused on reads of guarded commands too, and on
    // unbound agents: the parse runs before any other decision.
    assert_eq!(code(&decide(&unbound(), "branch --lis")), "option_unknown");
}

/// Round 1, class 3: symbolic refs.
#[test]
fn symbolic_refs_cannot_alias_protected_refs() {
    let s = work();
    assert_codes(
        &s,
        "symref",
        &[
            "symbolic-ref refs/heads/agend/t-1/fix refs/heads/main",
            "symbolic-ref refs/heads/agend/t-1/alias refs/heads/master",
            "symbolic-ref refs/agend/x refs/heads/agend/t-1/fix",
        ],
    );
    assert_eq!(
        code(&decide(&s, "symbolic-ref HEAD refs/heads/agend/t-1/fix")),
        "run"
    );
    assert_eq!(code(&decide(&s, "symbolic-ref HEAD")), "run");
    assert_eq!(
        code(&decide(
            &s,
            "symbolic-ref --delete refs/heads/agend/t-1/alias"
        )),
        "run"
    );
    assert_eq!(
        code(&decide(
            &s,
            "symbolic-ref --delete refs/remotes/origin/HEAD"
        )),
        "run",
        "not a branch"
    );
    assert_eq!(
        code(&decide(&s, "symbolic-ref -d refs/heads/main")),
        "protected_ref"
    );

    let alias = Fake {
        symrefs: vec![
            ("refs/heads/agend/t-1/alias", "refs/heads/master"),
            ("refs/heads/agend/t-1/mine", "refs/heads/agend/t-1/fix"),
            ("refs/remotes/origin/sneaky", "refs/heads/release"),
        ],
        ..Fake::default()
    };
    for cmd in [
        "update-ref refs/heads/agend/t-1/alias abc",
        "branch -f agend/t-1/alias abc",
        "push . HEAD:refs/heads/agend/t-1/alias",
        "fetch . HEAD:refs/heads/agend/t-1/alias",
        "fetch origin main:refs/remotes/origin/sneaky",
    ] {
        assert_eq!(code(&with_probe(&alias, cmd)), "symref", "{cmd}");
    }
    assert_eq!(
        code(&with_probe(
            &alias,
            "update-ref --no-deref refs/heads/agend/t-1/alias abc"
        )),
        "run",
        "--no-deref replaces the symref itself"
    );
    assert_eq!(
        code(&with_probe(
            &alias,
            "update-ref refs/heads/agend/t-1/mine abc"
        )),
        "run"
    );
    // The bound branch itself being a symref blocks every write.
    let own = Fake {
        symrefs: vec![("refs/heads/agend/t-1/fix", "refs/heads/main")],
        ..Fake::default()
    };
    for cmd in ["commit -m x", "merge feature", "reset --hard"] {
        assert_eq!(code(&with_probe(&own, cmd)), "symref", "{cmd}");
    }
}

/// Round 1, class 4: work tree / index retargeting. Round 3: the work tree
/// is git's answer, so every spelling (`--work-tree`, `GIT_WORK_TREE`, a git
/// dir without a work tree, a gitfile) is the same case.
#[test]
fn writes_must_use_the_bound_work_tree() {
    let s = work();
    let other = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
    let fake = Fake::default();
    let own = resolved_at(Location::Worktree).unwrap();
    let with_wt = |w: Option<PathBuf>| Resolved {
        work_tree: w,
        ..own.clone()
    };
    let at = |r: &Resolved, env: &GitEnv, cmd| {
        code(&run_full(
            Ok(&s),
            Location::Worktree,
            Some(r),
            env,
            &fake,
            cmd,
        ))
    };
    let plain = GitEnv::default();
    for cmd in [
        "reset --hard",
        "checkout -- .",
        "clean -fd",
        "commit -m x",
        "add -A",
    ] {
        assert_eq!(
            at(&with_wt(Some(other.clone())), &plain, cmd),
            "work_tree_retarget",
            "{cmd}"
        );
        assert_eq!(
            at(&with_wt(None), &plain, cmd),
            "work_tree_retarget",
            "bare: {cmd}"
        );
        assert_eq!(at(&own, &plain, cmd), "run", "{cmd}");
    }
    // Hooks run with GIT_DIR set, in the worktree: env retargeting is fine
    // when git resolves to the bound worktree.
    let hook = GitEnv {
        retargets: true,
        ..GitEnv::default()
    };
    assert_eq!(at(&own, &hook, "commit -m x"), "run");
    let idx = |p: PathBuf| GitEnv {
        retargets: true,
        index_file: Some(p),
        ..GitEnv::default()
    };
    assert_eq!(
        at(&own, &idx(other.join("index")), "add x"),
        "work_tree_retarget"
    );
    let gd = agend_testkit::tempdir::TempDir::new("classify-gitdir").unwrap();
    let own = Resolved {
        git_dir: std::fs::canonicalize(gd.path()).unwrap(),
        ..own.clone()
    };
    assert_eq!(
        at(&own, &idx(own.git_dir.join("index.lock")), "add x"),
        "run"
    );
    // Reads may look anywhere.
    assert_eq!(at(&with_wt(Some(other)), &plain, "status"), "run");
}

/// Round 3: a write that has to be routed drops the caller's `--git-dir` /
/// `--work-tree`; naming one is refused instead of silently rewritten.
/// `-C` alone still routes.
#[test]
fn routed_writes_do_not_drop_a_named_git_dir() {
    let s = work();
    let fake = Fake::default();
    let env = GitEnv::default();
    for loc in [
        Location::Canonical,
        Location::OtherWorktree,
        Location::NoRepo,
    ] {
        for cmd in [
            "--git-dir=/repo/.git commit -m x",
            "--work-tree=/repo clean -fd",
            "--git-dir /nowhere reset --hard",
            "--bare update-ref refs/heads/agend/t-1/x HEAD",
        ] {
            assert_eq!(
                code(&run_with(Ok(&s), loc, &env, &fake, cmd)),
                "work_tree_retarget",
                "{loc:?} {cmd}"
            );
        }
        let d = run_with(Ok(&s), loc, &env, &fake, "-C /repo commit -m x");
        assert!(
            matches!(&d, Decision::Run { route: Some(_), .. }),
            "{loc:?}: {d:?}"
        );
        // Reads still route, dropping the retargeting globals.
        let d = run_with(Ok(&s), loc, &env, &fake, "--git-dir=/repo/.git status");
        assert!(
            matches!(&d, Decision::Run { route: Some(_), .. }),
            "{loc:?}: {d:?}"
        );
    }
}

#[test]
fn destructive_operations_take_a_snapshot() {
    let s = work();
    for (cmd, op) in [
        ("reset --hard HEAD~1", Some("reset")),
        ("reset --keep HEAD~1", Some("reset")),
        ("reset HEAD~1", None),
        ("clean -fd", Some("clean")),
        ("clean -xdf", Some("clean")),
        ("clean --force", Some("clean")),
        ("clean -d", Some("clean")),
        ("clean -fd -enone", Some("clean")),
        ("clean -i", Some("clean")),
        ("clean -nd", None),
        ("clean -fdn", None),
        ("checkout -- .", Some("checkout")),
        ("checkout .", Some("checkout")),
        ("checkout ./src", Some("checkout")),
        ("checkout main -- src", Some("checkout")),
        ("checkout main src other", Some("checkout")),
        ("checkout -f", Some("checkout")),
        ("checkout -p", Some("checkout")),
        ("restore .", Some("restore")),
        ("restore --staged .", None),
        ("restore --staged --worktree .", Some("restore")),
        ("restore -s HEAD~1 .", Some("restore")),
        ("switch --discard-changes agend/t-1/fix", Some("switch")),
        ("read-tree -u --reset HEAD", Some("read-tree")),
        ("read-tree HEAD", None),
        ("commit -am x", None),
    ] {
        assert_eq!(snapshot_of(&decide(&s, cmd)), op, "{cmd}");
    }
}

#[test]
fn unknown_commands_and_rewrites_are_refused() {
    let s = work();
    assert_eq!(code(&decide(&s, "co main")), "unknown_command");
    assert_eq!(code(&decide(&s, "--frob push")), "unknown_command");
    assert_eq!(
        code(&decide(&s, "filter-branch -- --all")),
        "history_rewrite"
    );
    assert_eq!(code(&decide(&s, "--version")), "run");
    assert_eq!(code(&decide(&s, "")), "run");
}

#[test]
fn env_retargeting_blocks_routing() {
    let s = work();
    let env = GitEnv {
        retargets: true,
        ..GitEnv::default()
    };
    let fake = Fake::default();
    let d = run_with(Ok(&s), Location::Canonical, &env, &fake, "commit -m x");
    assert_eq!(code(&d), "git_env_retarget");
    let d = run_with(Ok(&s), Location::Worktree, &env, &fake, "commit -m x");
    assert_eq!(code(&d), "run", "hooks run with GIT_DIR set");
}

#[test]
fn missing_worktree_refuses_writes() {
    let mut s = work();
    s.binding = Some(Binding::Work {
        task_id: "t-1".into(),
        branch: work_branch("t-1", "fix"),
        worktree: "/definitely/not/here".into(),
    });
    assert_eq!(code(&decide(&s, "commit -m x")), "worktree_missing");
}

/// T5: foreign repos are free, except clones of the team repo and pushes
/// to it.
#[test]
fn foreign_repos_are_guarded_only_towards_the_team() {
    let s = work();
    let free = Fake::default();
    let at = |probe: &Fake, cmd| {
        code(&run_with(
            Ok(&s),
            Location::Foreign,
            &GitEnv::default(),
            probe,
            cmd,
        ))
    };
    for cmd in [
        "reset --hard",
        "checkout -b x",
        "push origin HEAD:main",
        "co main",
    ] {
        assert_eq!(at(&free, cmd), "run", "{cmd}");
    }
    let clone = Fake {
        team_repo: true,
        ..Fake::default()
    };
    assert_eq!(at(&clone, "push origin HEAD:main"), "team_clone");
    assert_eq!(at(&clone, "commit -m x"), "team_clone");
    assert_eq!(at(&clone, "log"), "run");
    assert_eq!(at(&clone, "branch -a"), "run");
    // Round 4: only `worktree list` reads; the rest change the team repo's
    // worktrees (`add` also creates a branch) and are team_clone writes.
    assert_eq!(at(&clone, "worktree list"), "run");
    assert_eq!(at(&clone, "worktree list --porcelain"), "run");
    for cmd in [
        "worktree add ../zm main",
        "worktree add -b x ../zm",
        "worktree remove ../zm",
        "worktree move ../zm ../zn",
        "worktree prune",
        "worktree lock ../zm",
        "worktree unlock ../zm",
        "worktree repair",
        "worktree",
    ] {
        assert_eq!(at(&clone, cmd), "team_clone", "{cmd}");
        assert_eq!(at(&free, cmd), "run", "scratch repo: {cmd}");
    }
    let remote = Fake {
        team_remotes: vec!["/team.git", "team"],
        ..Fake::default()
    };
    assert_eq!(at(&remote, "push /team.git HEAD:main"), "team_remote");
    assert_eq!(at(&remote, "push --repo=team HEAD:main"), "team_remote");
    assert_eq!(at(&remote, "push /mine.git HEAD:main"), "run");
    assert_eq!(at(&remote, "fetch /team.git"), "run");
}

/// Deny-by-default must not break everyday agent work.
#[test]
fn everyday_commands_still_run() {
    assert_codes(
        &work(),
        "run",
        &[
            "status --short",
            "add -A",
            "commit -am msg",
            "commit --amend --no-edit",
            "diff --stat origin/main...HEAD",
            "log --oneline -5",
            "fetch origin",
            "fetch --prune origin",
            "pull --rebase",
            "pull --ff-only origin main",
            "rebase origin/main",
            "rebase -i HEAD~3",
            "merge --no-ff origin/main",
            "cherry-pick abc123",
            "stash",
            "stash push -m wip",
            "stash pop",
            "checkout -- src/lib.rs",
            "restore --staged src/lib.rs",
            "reset HEAD~1",
            "reset --soft HEAD~1",
            "branch -vv",
            "branch --show-current",
            "tag",
            "tag -l v*",
            "clean -n",
            "push origin HEAD:refs/heads/agend/t-1/fix",
            "push --force-with-lease origin HEAD:refs/heads/agend/t-1/fix",
            "-c user.name=Bot -c user.email=bot@x commit -m x",
            "config user.email bot@x",
            "remote -v",
            "worktree list",
        ],
    );
}
