//! Decision tests: argv (+ a fake `Probe`) → run / route / snapshot /
//! refuse. Real-git behaviour, including the hooks, is in
//! `crates/agend/tests/shim_*.rs`.

use super::*;
use crate::binding::SNAPSHOT_VERSION;
use agend_core::model::work_branch;

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

/// Answers from fixed tables.
struct Fake {
    team_repo: bool,
    team_remotes: Vec<&'static str>,
    /// `rev-parse --symbolic-full-name` answers.
    names: Vec<(&'static str, &'static str)>,
    /// Revisions that name a commit.
    commits: Vec<&'static str>,
    hooks: bool,
}

impl Default for Fake {
    fn default() -> Fake {
        Fake {
            team_repo: false,
            team_remotes: Vec::new(),
            names: Vec::new(),
            commits: vec![
                "main",
                "master",
                "feature",
                "abc123",
                "HEAD",
                "HEAD~1",
                "agend/t-1/fix",
                "refs/heads/agend/t-1/fix",
                "origin/other-feature",
            ],
            hooks: true,
        }
    }
}

impl Probe for Fake {
    fn is_team_repo(&self) -> bool {
        self.team_repo
    }
    fn is_team_remote(&self, dest: &str) -> bool {
        self.team_remotes.contains(&dest)
    }
    fn rev_parse(&self, args: &[&str]) -> Option<String> {
        match args {
            ["--symbolic-full-name", rev] => self
                .names
                .iter()
                .find(|(r, _)| r == rev)
                .map(|(_, n)| n.to_string()),
            ["--verify", "-q", rev] => {
                let rev = rev.strip_suffix("^{commit}")?;
                self.commits.contains(&rev).then(|| "c0ffee".to_string())
            }
            other => panic!("unexpected rev-parse {other:?}"),
        }
    }
    fn hooks_installed(&self) -> bool {
        self.hooks
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
            prefix: PathBuf::new(),
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
    classify(&Input {
        args: &parsed,
        snapshot: snap,
        location: loc,
        resolved,
        env,
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
    assert_eq!(g.config, ["x=y"]);
    let g = parse(&argv("--git-dir=/x/.git --work-tree /y status"));
    assert_eq!(g.git_dir.as_deref(), Some("/x/.git"));
    assert_eq!(g.work_tree.as_deref(), Some("/y"));
    assert_eq!(g.sub.as_deref(), Some("status"));
    let g = parse(&argv(
        "--config-env=core.hooksPath=H --config-env a.b=V log",
    ));
    assert_eq!(g.config, ["core.hooksPath=H", "a.b=V"]);
    assert!(parse(&argv("--version")).info_only);
    assert_eq!(parse(&argv("")).sub, None);
    // An unknown global becomes the "subcommand" and is refused.
    assert_eq!(parse(&argv("--frob push")).sub.as_deref(), Some("--frob"));
}

#[test]
fn bound_writes_route_into_the_worktree() {
    let s = work();
    for loc in [Location::NoRepo, Location::Canonical] {
        match decide_at(Ok(&s), loc, "commit -m x") {
            Decision::Run { route, .. } => {
                assert_eq!(route.as_deref(), Some(std::env::temp_dir().as_path()))
            }
            other => panic!("{loc:?}: {other:?}"),
        }
    }
    assert_eq!(decide(&s, "commit -m x"), PASS);
    // Round 5: another agent's worktree is the wrong directory, not a
    // place to route from.
    assert_eq!(
        code(&decide_at(Ok(&s), Location::OtherWorktree, "commit -m x")),
        "other_worktree"
    );
}

/// Round 5, finding 1: routing keeps the caller's directory inside its
/// checkout (git's `--show-prefix`), so `git rm -rf .` from
/// `<canonical>/src/sub` acts on `<worktree>/src/sub`, never the whole
/// worktree; a directory the worktree lacks, and another worktree, refuse.
#[test]
fn routing_keeps_the_callers_subdirectory() {
    let dir = agend_testkit::tempdir::TempDir::new("classify-prefix").unwrap();
    let wt = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::create_dir_all(wt.join("src/sub")).unwrap();
    let s = Snapshot {
        binding: Some(Binding::Work {
            task_id: "t-1".into(),
            branch: work_branch("t-1", "fix"),
            worktree: wt.clone(),
        }),
        ..work()
    };
    let at = |loc: Location, prefix: &str, cmd: &str| {
        let mut r = resolved_at(loc).unwrap();
        r.prefix = prefix.into();
        run_full(
            Ok(&s),
            loc,
            Some(&r),
            &GitEnv::default(),
            &Fake::default(),
            cmd,
        )
    };
    let route = |d: &Decision| match d {
        Decision::Run { route, .. } => route.clone(),
        Decision::Refuse(r) => panic!("refused: {r:?}"),
    };
    for cmd in [
        "rm -rf .",
        "clean -fd .",
        "checkout -- .",
        "add .",
        "restore .",
        "status",
        "log -- .",
    ] {
        let d = at(Location::Canonical, "src/sub/", cmd);
        assert_eq!(route(&d), Some(wt.join("src/sub")), "{cmd}");
        if let Decision::Run { note, .. } = &d {
            let note = note.as_deref().unwrap_or_default();
            assert!(
                note.contains(&wt.join("src/sub").display().to_string()),
                "{note}"
            );
        }
        assert_eq!(
            route(&at(Location::Canonical, "", cmd)),
            Some(wt.clone()),
            "{cmd}"
        );
        let missing = at(Location::Canonical, "only/in/canonical/", cmd);
        let Decision::Refuse(r) = &missing else {
            panic!("{cmd}: {missing:?}")
        };
        assert_eq!(r.code, "route_dir_missing", "{cmd}");
        assert!(r.reason.contains("only/in/canonical/"), "{}", r.reason);
        assert!(
            r.next.contains(&format!("cd {}", wt.display())),
            "{}",
            r.next
        );
        for bad in ["../x/", "/abs/"] {
            assert_eq!(
                code(&at(Location::Canonical, bad, cmd)),
                "route_dir_missing"
            );
        }
    }
    // Writes from another worktree are refused, with the directory to use.
    for cmd in [
        "rm -rf .",
        "clean -fdx .",
        "checkout -- .",
        "add .",
        "restore .",
        "commit -m x",
    ] {
        let d = at(Location::OtherWorktree, "src/sub/", cmd);
        let Decision::Refuse(r) = &d else {
            panic!("{cmd}: {d:?}")
        };
        assert_eq!(r.code, "other_worktree", "{cmd}");
        assert!(r.reason.contains("another worktree"), "{}", r.reason);
        assert_eq!(
            r.next,
            format!("cd {} and run it there", wt.join("src/sub").display())
        );
    }
    // Inside the canonical git dir git sees no work tree: writes refuse.
    let mut in_git_dir = resolved_at(Location::Canonical).unwrap();
    in_git_dir.work_tree = None;
    let d = run_full(
        Ok(&s),
        Location::Canonical,
        Some(&in_git_dir),
        &GitEnv::default(),
        &Fake::default(),
        "clean -fdx .",
    );
    assert_eq!(code(&d), "route_dir_missing");
    // Reads there run where they were typed (not routed, not refused).
    for cmd in ["status", "log", "diff", "branch --show-current"] {
        assert_eq!(at(Location::OtherWorktree, "src/sub/", cmd), PASS, "{cmd}");
    }
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
    assert_eq!(d, PASS);
    let err = SnapshotError::NotAnAgent;
    assert_eq!(decide_at(Err(&err), Location::Unknown, "log"), PASS);
}

#[test]
fn mutations_need_a_binding() {
    let err = SnapshotError::Missing("/h/bindings/dev-1.json".into());
    for cmd in ["commit -m x", "fetch origin", "branch x", "config a.b c"] {
        assert_eq!(
            code(&decide_at(Err(&err), Location::Unknown, cmd)),
            "no_binding",
            "{cmd}"
        );
    }
    assert_eq!(
        code(&decide_at(Ok(&unbound()), Location::NoRepo, "commit -m x")),
        "unbound"
    );
    assert_eq!(
        code(&decide_at(Ok(&unbound()), Location::Canonical, "add .")),
        "canonical_checkout"
    );
}

/// The shim no longer checks refs: a bound worktree without the agend hooks
/// would leave protected refs unguarded, so writes there are refused.
#[test]
fn writes_need_the_hooks_installed() {
    let none = Fake {
        hooks: false,
        ..Fake::default()
    };
    for cmd in [
        "commit -m x",
        "push origin HEAD",
        "update-ref refs/heads/main HEAD",
    ] {
        assert_eq!(code(&with_probe(&none, cmd)), "hooks_missing", "{cmd}");
    }
    assert_eq!(code(&with_probe(&none, "status")), "run");
}

#[test]
fn skipping_the_hooks_is_refused() {
    let s = work();
    assert_codes(
        &s,
        "hooks_skipped",
        &[
            "-c core.hooksPath=/dev/null commit -m x",
            "-c CORE.HOOKSPATH=x status",
            "--config-env=core.hooksPath=H push",
            "push --no-verify origin HEAD",
            "push --no-veri origin HEAD",
        ],
    );
    let env = |v: &str| GitEnv {
        config: vec![v.to_string()],
        ..GitEnv::default()
    };
    for v in ["core.hooksPath", "'core.hookspath'='/x'"] {
        let d = run_with(
            Ok(&s),
            Location::Worktree,
            &env(v),
            &Fake::default(),
            "commit -m x",
        );
        assert_eq!(code(&d), "hooks_skipped", "{v}");
    }
    // Other config, and `--no-verify` elsewhere, are the agent's business.
    assert_codes(
        &s,
        "run",
        &[
            "-c core.editor=true commit --amend",
            "-c remote.origin.push=HEAD:refs/heads/main push origin",
            "commit --no-verify -m x",
        ],
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
fn leaving_the_bound_branch_is_refused() {
    let s = work();
    assert_codes(
        &s,
        "branch_switch",
        &[
            "checkout main",
            "checkout feature",
            "checkout --detach",
            "checkout --deta",
            // Round 7, finding 5: git takes any unambiguous prefix.
            "checkout --de",
            "checkout --d",
            "checkout --orp o",
            "checkout abc123",
            "checkout HEAD~1",
            "checkout -b feat/x",
            "checkout -bfoo",
            "checkout -B agend/t-1/x",
            "checkout --orphan o",
            "checkout --orph o",
            "checkout other-feature --",
            "checkout master --",
            "checkout --track origin/other-feature",
            "switch main",
            "switch --detach HEAD~1",
            "switch --deta",
            "switch --de",
            "switch --de HEAD~1",
            "switch --cr x",
            "switch -c agend/t-1/x",
            "switch -cfoo",
            "switch --create x",
            "switch --force-create x",
            "switch --orphan x",
            // Round 8, finding 5: a full ref name detaches HEAD.
            "checkout refs/heads/agend/t-1/fix",
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
            "switch --force agend/t-1/fix",
            "checkout",
            // Paths: not a commit, several arguments, or after `--`. A
            // remote-only branch git would create here is the hook's.
            "checkout src/lib.rs",
            "checkout other-feature",
            "checkout main src/lib.rs",
            "checkout main -- src",
            "checkout -- .",
        ],
    );
    assert_codes(
        &review(),
        "branch_switch",
        &["checkout abc123", "switch agend/t-1/fix"],
    );
}

/// Round 5, finding 3: `checkout -` / `switch -` resolve `@{-1}` with git;
/// back to the bound branch runs, anywhere else is refused.
/// Round 6, finding 1: git 2.39 copies / renames branches outside a ref
/// transaction (the hook never sees the new name), so the shim refuses
/// every spelling; other `branch` calls are the hook's.
#[test]
fn branch_copy_and_rename_are_refused() {
    let s = work();
    assert_codes(
        &s,
        "branch_copy",
        &[
            "branch -C master",
            "branch -c agend/t-1/side release/9",
            "branch -m agend/t-1/better-name",
            "branch -M agend/t-1/side master",
            "branch -fm a b",
            "branch -Mf a b",
            "branch -vC a",
            "branch --copy a b",
            "branch --move a b",
            "branch --mo a b",
            "branch --cop a b",
            "branch --force --move=x a b",
            "branch a -m b",
        ],
    );
    assert_codes(
        &s,
        "run",
        &[
            "branch agend/t-1/x",
            "branch -f agend/t-1/x HEAD",
            "branch -D agend/t-1/x",
            "branch -uorigin/main agend/t-1/x",
            "branch --set-upstream-to=origin/main",
            "branch --merged main",
            "branch --color --contains HEAD",
            "branch -tmerge agend/t-1/x",
            "branch agend/t-1/x -- -m",
        ],
    );
    let Decision::Refuse(r) = decide(&s, "branch -m agend/t-1/y") else {
        panic!()
    };
    assert!(
        r.next.contains("git branch agend/t-1/<name> <old>"),
        "{}",
        r.next
    );
    assert!(r.next.contains("git branch -D <old>"), "{}", r.next);
}

/// Round 7, findings 1–2 and 4: `symbolic-ref` writes and `reflog
/// delete|expire` change refs outside a ref transaction (the hook never
/// sees them); refused in every spelling. Reads still run.
#[test]
fn ref_writes_outside_the_hook_are_refused() {
    let s = work();
    assert_codes(
        &s,
        "ref_outside_hook",
        &[
            "symbolic-ref refs/heads/main refs/heads/agend/t-1/fix",
            "symbolic-ref refs/heads/master refs/heads/agend/t-1/fix",
            "symbolic-ref HEAD refs/heads/agend/t-1/fix",
            "symbolic-ref -m why HEAD refs/heads/main",
            "symbolic-ref -d refs/heads/agend/t-1/alias",
            "symbolic-ref --delete HEAD",
            "symbolic-ref -qd HEAD",
            "symbolic-ref -- HEAD refs/heads/main",
            "symbolic-ref",
            "reflog delete --updateref main@{0}",
            "reflog delete --updateref --rewrite master@{0}",
            "reflog expire --expire=now --all",
            "reflog expire --expire-unreachable=now refs/heads/agend/t-1/fix",
        ],
    );
    assert_codes(
        &s,
        "run",
        &[
            "symbolic-ref HEAD",
            "symbolic-ref -q HEAD",
            "symbolic-ref --short HEAD",
            "symbolic-ref --quiet --no-recurse refs/heads/agend/t-1/alias",
            "reflog",
            "reflog show main",
            "reflog -n 5",
            "reflog exists main",
        ],
    );
    let Decision::Refuse(r) = decide(&s, "reflog expire --expire=now --all") else {
        panic!()
    };
    assert!(r.next.contains("git reflog show"), "{}", r.next);
}

/// Round 8, findings 1–3 (owner decision 2026-09-25): `refs/stash` is one
/// list shared with the canonical checkout, so every stash write is refused
/// (reads run); `clean -x|-X` deletes ignored files no snapshot keeps.
#[test]
fn what_a_snapshot_cannot_undo_is_refused() {
    let s = work();
    let stash = [
        "stash",
        "stash -q",
        "stash -u",
        "stash -p",
        "stash push -m wip",
        "stash save wip",
        "stash pop",
        "stash apply",
        "stash apply stash@{1}",
        "stash drop",
        "stash clear",
        "stash branch agend/t-1/x",
        "stash create",
        "stash store abc123",
    ];
    assert_codes(&s, "stash_shared", &stash);
    assert_eq!(
        code(&decide_at(Ok(&s), Location::Canonical, "stash pop")),
        "stash_shared"
    );
    let Decision::Refuse(r) = decide(&s, "stash pop") else {
        unreachable!()
    };
    assert!(r.next.contains("git commit -m \"wip: "), "{}", r.next);
    assert!(r.next.contains("agend/t-1/fix"), "{}", r.next);
    assert_codes(
        &s,
        "run",
        &["stash list", "stash show", "stash show -p stash@{1}"],
    );
    let clean = [
        "clean -fdx",
        "clean -fX",
        "clean -xdf",
        "clean -f -d -x",
        "clean -Xn",
        "clean -fdx -- src",
    ];
    assert_codes(&s, "clean_ignored", &clean);
    let Decision::Refuse(r) = decide(&s, "clean -fdx") else {
        unreachable!()
    };
    assert!(r.next.contains("git clean -fd"), "{}", r.next);
    // `-e` takes the rest of its cluster as the pattern.
    for cmd in [
        "clean -fd",
        "clean -fd -exyz",
        "clean -n",
        "clean -fd -- -x",
    ] {
        assert_eq!(code(&decide(&s, cmd)), "run", "{cmd}");
    }
}

#[test]
fn previous_branch_is_resolved() {
    let prev = |name: &'static str| Fake {
        names: vec![("@{-1}", name), ("@{-2}", "refs/heads/main")],
        ..Fake::default()
    };
    for cmd in ["checkout -", "switch -", "checkout @{-1}", "checkout - --"] {
        assert_eq!(
            code(&with_probe(&prev("refs/heads/agend/t-1/fix"), cmd)),
            "run",
            "{cmd}"
        );
    }
    assert_eq!(
        snapshot_of(&with_probe(
            &prev("refs/heads/agend/t-1/fix"),
            "switch --discard-changes -"
        )),
        Some("switch")
    );
    for (probe, cmd) in [
        (prev("refs/heads/main"), "checkout -"),
        (prev("refs/heads/main"), "switch -"),
        (prev("refs/heads/agend/t-1/fix"), "checkout @{-2}"),
        (prev(""), "checkout -"),
        (Fake::default(), "switch -"),
    ] {
        assert_eq!(code(&with_probe(&probe, cmd)), "branch_switch", "{cmd}");
    }
}

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
    for cmd in ["--git-dir=/repo/.git commit -m x", "-C /repo commit -m x"] {
        assert_eq!(
            code(&run_with(Ok(&s), Location::OtherWorktree, &env, &fake, cmd)),
            "other_worktree",
            "{cmd}"
        );
    }
    for loc in [Location::Canonical, Location::NoRepo] {
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

/// v1 agentic-git's scope; options matched generously (an extra snapshot
/// is harmless, a missing one loses work).
#[test]
fn destructive_operations_take_a_snapshot() {
    let s = work();
    for (cmd, op) in [
        ("rm -rf .", Some("rm")),
        ("rm -r --force src", Some("rm")),
        ("rm --forc x", Some("rm")),
        ("rm x", None),
        ("rm --cached -f x", None),
        ("rm -n -rf .", None),
        ("rm --dry-run --force .", None),
        // Round 7, finding 3: overwrites a destination with edits.
        ("mv -f a.txt b.txt", Some("mv")),
        ("mv --force a b", Some("mv")),
        ("mv --f a b", Some("mv")),
        ("mv -vf a b", Some("mv")),
        ("mv a b", None),
        ("mv -n -f a b", None),
        ("mv -k a b", None),
        ("reset --hard HEAD~1", Some("reset")),
        ("reset --har", Some("reset")),
        ("reset --keep HEAD~1", Some("reset")),
        ("reset --merge", Some("reset")),
        ("reset HEAD~1", None),
        ("reset --soft HEAD~1", None),
        ("clean -fd", Some("clean")),
        ("clean --forc -d", Some("clean")),
        ("clean -fd -enone", Some("clean")),
        ("clean -d", Some("clean")),
        ("checkout -- .", Some("checkout")),
        ("checkout .", Some("checkout")),
        ("checkout main -- src", Some("checkout")),
        ("checkout -f", Some("checkout")),
        ("checkout --forc", Some("checkout")),
        ("restore .", Some("restore")),
        ("restore --staged .", None),
        ("restore -S .", None),
        ("restore --staged --worktree .", Some("restore")),
        ("restore -SW .", Some("restore")),
        ("switch --discard-changes agend/t-1/fix", Some("switch")),
        ("switch --discard agend/t-1/fix", Some("switch")),
        ("switch -f agend/t-1/fix", Some("switch")),
        ("switch agend/t-1/fix", None),
        ("merge origin/main", Some("merge")),
        ("merge --abort", Some("merge")),
        ("rebase origin/main", Some("rebase")),
        ("pull --rebase", Some("pull")),
        ("cherry-pick abc", Some("cherry-pick")),
        ("revert abc", Some("revert")),
        ("am x.patch", Some("am")),
        ("commit -am x", None),
        ("read-tree -u --reset HEAD", None),
    ] {
        assert_eq!(snapshot_of(&decide(&s, cmd)), op, "{cmd}");
    }
}

#[test]
fn unknown_commands_are_refused() {
    let s = work();
    assert_eq!(code(&decide(&s, "co main")), "unknown_command");
    assert_eq!(code(&decide(&s, "--frob push")), "unknown_command");
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

/// T5: foreign repos have no agend hooks; they are free, except the team's
/// remote, clones of the team repo, and pushes to it.
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
        "-c core.hooksPath=x commit",
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

/// Everyday agent work runs (refs are the hooks' business).
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
            "pull --rebase",
            "pull --ff-only origin main",
            "rebase origin/main",
            "rebase -i HEAD~3",
            "rebase --onto origin/main main",
            "merge --no-ff origin/main",
            "cherry-pick abc123",
            "stash list",
            "stash show -p stash@{0}",
            "checkout -- src/lib.rs",
            "restore --staged src/lib.rs",
            "reset --soft HEAD~1",
            "branch -vv",
            "branch agend/t-1/x",
            "tag",
            "tag v1",
            "clean -n",
            "push",
            "push -u origin agend/t-1/fix",
            "push --force-with-lease origin HEAD:refs/heads/agend/t-1/fix",
            "-c user.name=Bot -c sequence.editor=true rebase -i HEAD~2",
            "config user.email bot@x",
            "remote -v",
            "worktree list",
        ],
    );
}
