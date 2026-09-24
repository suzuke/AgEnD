//! Classifies a git invocation: run it as is, route it into the bound
//! worktree, or refuse it with the exact next step. Refuses agent-created
//! worktrees and branches (`git worktree`, `checkout -b`, `switch -c`,
//! `branch <new>` outside the agent's own `agend/<task-id>/` namespace),
//! branch switches, protected-ref writes, and every mutation while unbound or
//! without a usable binding snapshot.
//!
//! Pure: the caller supplies the parsed argv, the snapshot, the location and
//! a commit-ish resolver (the only question that needs git itself).
//!
//! Must NOT: guess a binding when the snapshot is missing.

use crate::Refusal;
use crate::binding::{Binding, Snapshot, SnapshotError};
use crate::location::Location;
use crate::protected_ref::ProtectedRefs;
use std::path::{Path, PathBuf};

mod commands;
use commands::{
    BranchOp, branch, branch_op, checkout, clean_is_forced, fetch, fetch_refspecs, push,
    restore_touches_worktree, switch, symbolic_ref, symbolic_ref_writes, tag, tag_is_list,
    update_ref,
};

/// A git argv split into leading global options and the subcommand.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitArgs {
    /// Index of the subcommand in the argv (argv length if none).
    pub sub_index: usize,
    pub sub: Option<String>,
    pub rest: Vec<String>,
    /// `-C <dir>` values, in order.
    pub chdirs: Vec<String>,
    /// `--git-dir` (or `--bare`, which means `.`).
    pub git_dir: Option<String>,
    pub work_tree: Option<String>,
    /// `--version`, `--help` and similar: git prints something and exits.
    pub info_only: bool,
    /// argv indexes of globals that retarget git (`-C`, `--git-dir`,
    /// `--work-tree`, `--bare`); dropped when routing.
    pub retarget_indexes: Vec<usize>,
}

const INFO_GLOBALS: &[&str] = &[
    "--version",
    "-v",
    "--help",
    "-h",
    "--html-path",
    "--man-path",
    "--info-path",
    "--exec-path",
];
const FLAG_GLOBALS: &[&str] = &[
    "-p",
    "--paginate",
    "-P",
    "--no-pager",
    "--no-replace-objects",
    "--literal-pathspecs",
    "--glob-pathspecs",
    "--noglob-pathspecs",
    "--icase-pathspecs",
    "--no-optional-locks",
    "--no-lazy-fetch",
    "--no-advice",
];
const VALUE_GLOBALS: &[&str] = &[
    "-c",
    "--namespace",
    "--super-prefix",
    "--config-env",
    "--attr-source",
];

/// Splits leading global options from the subcommand. An unrecognised
/// option ends the globals and becomes the "subcommand", so it is refused
/// as unknown instead of hiding the real one.
pub fn parse(args: &[String]) -> GitArgs {
    let mut g = GitArgs::default();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if !a.starts_with('-') {
            break;
        }
        if INFO_GLOBALS.contains(&a) {
            g.info_only = true;
        } else if FLAG_GLOBALS.contains(&a) {
        } else if a == "-C" || a == "--git-dir" || a == "--work-tree" {
            let Some(v) = args.get(i + 1) else { break };
            g.retarget_indexes.extend([i, i + 1]);
            match a {
                "-C" => g.chdirs.push(v.clone()),
                "--git-dir" => g.git_dir = Some(v.clone()),
                _ => g.work_tree = Some(v.clone()),
            }
            i += 1;
        } else if let Some(v) = a.strip_prefix("--git-dir=") {
            g.retarget_indexes.push(i);
            g.git_dir = Some(v.to_string());
        } else if let Some(v) = a.strip_prefix("--work-tree=") {
            g.retarget_indexes.push(i);
            g.work_tree = Some(v.to_string());
        } else if a == "--bare" {
            g.retarget_indexes.push(i);
            g.git_dir = Some(".".into());
        } else if VALUE_GLOBALS.contains(&a) {
            if i + 1 >= args.len() {
                break;
            }
            i += 1;
        } else if !VALUE_GLOBALS
            .iter()
            .chain(["--exec-path", "--list-cmds"].iter())
            .any(|v| a.starts_with(&format!("{v}=")))
        {
            break;
        }
        i += 1;
    }
    g.sub_index = i;
    g.sub = args.get(i).cloned();
    g.rest = args.get(i + 1..).unwrap_or_default().to_vec();
    g
}

/// What to do with the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Run {
        /// Run in this worktree (`-C`, retargeting globals dropped) instead
        /// of where the caller pointed git.
        route: Option<PathBuf>,
        /// Take a snapshot first; the name of the destructive operation.
        snapshot: Option<&'static str>,
        /// One line for the agent (stderr) before running.
        note: Option<String>,
    },
    Refuse(Refusal),
}

impl Decision {
    fn pass() -> Decision {
        Decision::Run {
            route: None,
            snapshot: None,
            note: None,
        }
    }
}

pub struct Input<'a> {
    pub args: &'a GitArgs,
    pub snapshot: Result<&'a Snapshot, &'a SnapshotError>,
    pub location: Location,
    /// `GIT_DIR`/`GIT_WORK_TREE`/`GIT_COMMON_DIR` are set in the env.
    pub env_retargets: bool,
    pub protected: &'a ProtectedRefs,
    /// Where the caller pointed git (cwd with `-C` applied), for messages.
    pub dir: &'a Path,
    /// Whether a name resolves to a commit in the bound worktree.
    pub is_commit: &'a dyn Fn(&str) -> bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Read,
    Write,
    NewRepo,
}

pub fn classify(input: &Input) -> Decision {
    let args = input.args;
    let Some(sub) = args.sub.as_deref() else {
        return Decision::pass();
    };
    if args.info_only {
        return Decision::pass();
    }
    if input.location == Location::Foreign {
        return Decision::pass();
    }
    let rest = args.rest.as_slice();
    let binding = input.snapshot.ok().and_then(|s| s.binding.as_ref());

    if sub == "worktree" && !matches!(first_positional(rest), Some("list")) {
        return Decision::Refuse(refuse_worktree(sub, rest, binding));
    }
    if matches!(sub, "filter-branch" | "filter-repo" | "replace") {
        return Decision::Refuse(Refusal::new(
            "history_rewrite",
            format!("`git {sub}` rewrites shared history and is not allowed for agents"),
            "commit fixes on your branch instead; if history must change, ask a human: agend ask \"<question>\"",
        ));
    }
    let Some(kind) = kind(sub, rest) else {
        return Decision::Refuse(Refusal::new(
            "unknown_command",
            format!(
                "`git {sub}` is not a git command the agend shim knows (aliases and external git-* commands are not allowed for agents)"
            ),
            "run the underlying built-in git command directly; list them with: git help -a",
        ));
    };
    match kind {
        Kind::NewRepo => return Decision::pass(),
        Kind::Read => return route_read(input, binding),
        Kind::Write => {}
    }

    let snapshot = match input.snapshot {
        Ok(s) => s,
        Err(e) => return Decision::Refuse(refuse_no_binding(sub, e)),
    };
    let Some(binding) = snapshot.binding.as_ref() else {
        return Decision::Refuse(refuse_unbound(sub, input.location, input.dir));
    };
    if !binding.worktree().is_dir() {
        return Decision::Refuse(Refusal::new(
            "worktree_missing",
            format!(
                "your bound worktree {} does not exist",
                binding.worktree().display()
            ),
            "run `agend status`; the daemon re-creates or re-assigns the worktree",
        ));
    }
    let route = (input.location != Location::Worktree).then(|| binding.worktree().to_path_buf());
    if route.is_some() && input.env_retargets {
        return Decision::Refuse(Refusal::new(
            "git_env_retarget",
            "GIT_DIR / GIT_WORK_TREE / GIT_COMMON_DIR point git outside your bound worktree"
                .to_string(),
            format!(
                "unset GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR and run plain `git {sub} ...`; it runs in {}",
                binding.worktree().display()
            ),
        ));
    }
    let snapshot_op = match check_write(sub, rest, binding, input) {
        Ok(op) => op,
        Err(r) => return Decision::Refuse(r),
    };
    let note = matches!(
        input.location,
        Location::Canonical | Location::OtherWorktree
    )
    .then(|| routed_note(binding, input.dir));
    Decision::Run {
        route,
        snapshot: snapshot_op,
        note,
    }
}

/// Read-only commands: run in the bound worktree when the caller is outside
/// it (workspace dir, canonical checkout, another worktree), else as is.
fn route_read(input: &Input, binding: Option<&Binding>) -> Decision {
    let Some(binding) = binding else {
        return Decision::pass();
    };
    let outside = matches!(
        input.location,
        Location::Canonical | Location::OtherWorktree | Location::NoRepo
    );
    if !outside || input.env_retargets || !binding.worktree().is_dir() {
        return Decision::pass();
    }
    let note = (input.location != Location::NoRepo).then(|| routed_note(binding, input.dir));
    Decision::Run {
        route: Some(binding.worktree().to_path_buf()),
        snapshot: None,
        note,
    }
}

fn routed_note(binding: &Binding, dir: &Path) -> String {
    format!(
        "agend-shim: running in your bound worktree {} (you ran git in {})",
        binding.worktree().display(),
        dir.display()
    )
}

/// Command-specific checks for a bound agent's write. Returns the name of the
/// destructive operation to snapshot, if any.
fn check_write(
    sub: &str,
    rest: &[String],
    binding: &Binding,
    input: &Input,
) -> Result<Option<&'static str>, Refusal> {
    let p = input.protected;
    match sub {
        "checkout" => checkout(rest, binding, p, input.is_commit),
        "switch" => switch(rest, binding, p),
        "branch" => branch(rest, binding, p).map(|_| None),
        "tag" => tag(rest, p).map(|_| None),
        "push" => push(rest, binding, p).map(|_| None),
        "fetch" => fetch(rest, binding, p).map(|_| None),
        "update-ref" => update_ref(rest, binding, p).map(|_| None),
        "symbolic-ref" => symbolic_ref(rest, binding, p).map(|_| None),
        "reset" => Ok(rest
            .iter()
            .any(|a| matches!(a.as_str(), "--hard" | "--merge" | "--keep"))
            .then_some("reset")),
        "clean" => Ok(clean_is_forced(rest).then_some("clean")),
        "restore" => Ok(restore_touches_worktree(rest).then_some("restore")),
        _ => Ok(None),
    }
}

// ── command kinds ───────────────────────────────────────────────────────

const READ: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "blame",
    "annotate",
    "ls-files",
    "ls-tree",
    "rev-parse",
    "rev-list",
    "cat-file",
    "describe",
    "shortlog",
    "grep",
    "for-each-ref",
    "show-ref",
    "name-rev",
    "merge-base",
    "merge-tree",
    "diff-tree",
    "diff-index",
    "diff-files",
    "difftool",
    "count-objects",
    "check-ignore",
    "check-attr",
    "check-mailmap",
    "check-ref-format",
    "var",
    "help",
    "version",
    "whatchanged",
    "format-patch",
    "range-diff",
    "cherry",
    "fsck",
    "verify-commit",
    "verify-tag",
    "verify-pack",
    "archive",
    "show-branch",
    "patch-id",
    "ls-remote",
    "hash-object",
    "request-pull",
    "interpret-trailers",
    "stripspace",
    "column",
    "get-tar-commit-id",
];

const WRITE: &[&str] = &[
    "add",
    "rm",
    "mv",
    "commit",
    "reset",
    "restore",
    "checkout",
    "switch",
    "merge",
    "rebase",
    "cherry-pick",
    "revert",
    "pull",
    "push",
    "am",
    "apply",
    "stash",
    "tag",
    "branch",
    "notes",
    "clean",
    "gc",
    "prune",
    "repack",
    "pack-refs",
    "maintenance",
    "update-index",
    "read-tree",
    "write-tree",
    "commit-tree",
    "mktree",
    "mktag",
    "update-ref",
    "symbolic-ref",
    "bisect",
    "sparse-checkout",
    "lfs",
    "rerere",
    "submodule",
    "config",
    "remote",
    "reflog",
    "fetch",
    "fast-import",
    "unpack-objects",
];

fn kind(sub: &str, rest: &[String]) -> Option<Kind> {
    if matches!(sub, "init" | "clone") {
        return Some(Kind::NewRepo);
    }
    if READ.contains(&sub) {
        return Some(Kind::Read);
    }
    if sub == "worktree" {
        return Some(Kind::Read); // only `worktree list` gets here
    }
    if !WRITE.contains(&sub) {
        return None;
    }
    let first = first_positional(rest);
    let read = match sub {
        "branch" => matches!(branch_op(rest), BranchOp::List),
        "tag" => tag_is_list(rest),
        "stash" => matches!(first, Some("list" | "show")),
        "remote" => matches!(first, None | Some("show" | "get-url")),
        "config" => config_is_read(rest),
        "reflog" => !matches!(first, Some("expire" | "delete")),
        "notes" => matches!(first, None | Some("list" | "show")),
        "submodule" => matches!(first, None | Some("status" | "summary")),
        "sparse-checkout" => matches!(first, Some("list")),
        "symbolic-ref" => !symbolic_ref_writes(rest),
        "fetch" => !fetch_refspecs(rest).iter().any(|s| s.contains(':')),
        _ => false,
    };
    Some(if read { Kind::Read } else { Kind::Write })
}

fn first_positional(rest: &[String]) -> Option<&str> {
    rest.iter()
        .map(String::as_str)
        .find(|a| !a.starts_with('-'))
}

fn config_is_read(rest: &[String]) -> bool {
    const READ_FLAGS: &[&str] = &[
        "--get",
        "--get-all",
        "--get-regexp",
        "--get-urlmatch",
        "--list",
        "-l",
        "--get-color",
        "--get-colorbool",
        "get",
        "list",
    ];
    if rest.iter().any(|a| READ_FLAGS.contains(&a.as_str())) {
        return true;
    }
    let value_flags = ["--file", "-f", "--blob", "--type", "--default"];
    let mut positionals = 0;
    let mut skip = false;
    for a in rest {
        if skip {
            skip = false;
        } else if value_flags.contains(&a.as_str()) {
            skip = true;
        } else if a.starts_with('-') {
            if matches!(
                a.as_str(),
                "--unset" | "--unset-all" | "--add" | "--replace-all" | "--edit" | "-e"
            ) || a.starts_with("--rename-section")
                || a.starts_with("--remove-section")
            {
                return false;
            }
        } else {
            positionals += 1;
        }
    }
    positionals == 1
}

// ── worktree / unbound / no binding ─────────────────────────────────────

fn refuse_worktree(sub: &str, rest: &[String], binding: Option<&Binding>) -> Refusal {
    let what = first_positional(rest).unwrap_or("");
    let next = match binding {
        Some(b) => format!(
            "work in your bound worktree: cd {}; for separate work create a task: agend task create \"<title>\"",
            b.worktree().display()
        ),
        None => "run `agend status`; the daemon creates your worktree when it assigns you a task"
            .to_string(),
    };
    Refusal::new(
        "worktree_managed",
        format!("`git {sub} {what}` is refused: only the daemon creates and removes worktrees"),
        next,
    )
}

fn refuse_no_binding(sub: &str, err: &SnapshotError) -> Refusal {
    let next = match err {
        SnapshotError::NotAnAgent => {
            "this git is the agend agent shim; agents get AGEND_HOME and AGEND_INSTANCE from their holder. Humans: use the git outside the agent PATH (e.g. /usr/bin/git)"
        }
        _ => {
            "run `agend status` (the daemon rewrites the snapshot); read-only git commands still work"
        }
    };
    Refusal::new(
        "no_binding",
        format!("`git {sub}` changes the repo, which needs a valid binding: {err}"),
        next,
    )
}

fn refuse_unbound(sub: &str, location: Location, dir: &Path) -> Refusal {
    let next = "run `agend status` to see your assignment; the daemon gives you a worktree when it assigns a task. To start new work: agend task create \"<title>\"";
    if location == Location::Canonical {
        Refusal::new(
            "canonical_checkout",
            format!(
                "`git {sub}` would change the canonical checkout {} and you have no task bound; agents never change the canonical checkout",
                dir.display()
            ),
            next,
        )
    } else {
        Refusal::new(
            "unbound",
            format!("`git {sub}` changes the repo but you have no task bound"),
            next,
        )
    }
}

#[cfg(test)]
mod tests {
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

    /// Commit-ish names in the fake repo.
    fn commits(name: &str) -> bool {
        matches!(name, "main" | "master" | "feature" | "HEAD~1" | "abc123")
    }

    fn decide_at(snap: Result<&Snapshot, &SnapshotError>, loc: Location, cmd: &str) -> Decision {
        let args = argv(cmd);
        let parsed = parse(&args);
        let protected =
            ProtectedRefs::new(snap.map(|s| s.protected_refs.as_slice()).unwrap_or(&[]));
        classify(&Input {
            args: &parsed,
            snapshot: snap,
            location: loc,
            env_retargets: false,
            protected: &protected,
            dir: Path::new("/somewhere"),
            is_commit: &commits,
        })
    }

    fn decide(snap: &Snapshot, cmd: &str) -> Decision {
        decide_at(Ok(snap), Location::Worktree, cmd)
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

    #[test]
    fn globals_are_split_from_the_subcommand() {
        let g = parse(&argv("-C /a -c x=y --no-pager -C b commit -m hi"));
        assert_eq!(g.sub.as_deref(), Some("commit"));
        assert_eq!(g.rest, argv("-m hi"));
        assert_eq!(g.chdirs, argv("/a b"));
        assert_eq!(g.retarget_indexes, vec![0, 1, 5, 6]);
        let g = parse(&argv("--git-dir=/x/.git --work-tree /y status"));
        assert_eq!(g.git_dir.as_deref(), Some("/x/.git"));
        assert_eq!(g.work_tree.as_deref(), Some("/y"));
        assert_eq!(g.sub.as_deref(), Some("status"));
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
            let d = decide_at(Ok(&s), loc, "commit -m x");
            match d {
                Decision::Run { route, .. } => {
                    assert_eq!(route.as_deref(), Some(std::env::temp_dir().as_path()))
                }
                other => panic!("{loc:?}: {other:?}"),
            }
        }
        assert_eq!(
            decide(&s, "commit -m x"),
            Decision::Run {
                route: None,
                snapshot: None,
                note: None
            }
        );
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
    fn foreign_repos_and_new_repos_pass() {
        assert_eq!(
            decide_at(Ok(&unbound()), Location::Foreign, "reset --hard"),
            Decision::pass()
        );
        let err = SnapshotError::NotAnAgent;
        assert_eq!(
            decide_at(Err(&err), Location::NoRepo, "clone x"),
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
        for cmd in [
            "checkout main",
            "checkout feature",
            "checkout -",
            "checkout --detach",
            "checkout abc123",
            "switch main",
            "switch -",
            "switch --detach HEAD~1",
            "symbolic-ref HEAD refs/heads/main",
            "update-ref --no-deref HEAD abc123",
        ] {
            assert_eq!(code(&decide(&s, cmd)), "branch_switch", "{cmd}");
        }
        let Decision::Refuse(r) = decide(&s, "checkout main") else {
            unreachable!()
        };
        assert!(r.reason.contains("main is protected"), "{}", r.reason);
        assert!(r.next.contains("agend task create"), "{}", r.next);
        for cmd in [
            "checkout agend/t-1/fix",
            "checkout HEAD",
            "switch agend/t-1/fix",
            "checkout",
        ] {
            assert_eq!(code(&decide(&s, cmd)), "run", "{cmd}");
        }
        for cmd in ["checkout abc123", "switch agend/t-1/fix"] {
            assert_eq!(code(&decide(&review(), cmd)), "branch_switch", "{cmd}");
        }
    }

    #[test]
    fn branch_creation_is_limited_to_the_own_namespace() {
        let s = work();
        for cmd in [
            "checkout -b feat/x",
            "checkout -b agend/t-1/x",
            "switch -c x",
            "switch --create=x",
            "branch feat/x",
            "branch agend/t-2/x",
        ] {
            assert_eq!(code(&decide(&s, cmd)), "branch_create", "{cmd}");
        }
        assert_eq!(code(&decide(&s, "branch agend/t-1/scratch")), "run");
        assert_eq!(
            code(&decide(&review(), "branch agend/t-2/x")),
            "branch_create"
        );
        for cmd in [
            "branch",
            "branch -a",
            "branch --list 'agend/*'",
            "branch -vv",
        ] {
            assert_eq!(code(&decide(&unbound(), cmd)), "run", "{cmd}");
        }
        assert_eq!(code(&decide(&s, "branch -m other")), "branch_rename");
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
        for cmd in [
            "update-ref refs/heads/main abc123",
            "update-ref main abc123",
            "update-ref -d refs/heads/master",
            "update-ref -m msg refs/heads/release abc123",
            "push . HEAD:main",
            "push origin HEAD:main",
            "push origin +agend/t-1/fix:refs/heads/master",
            "push origin --delete main",
            "push origin main",
            "branch -f main abc123",
            "branch -D main",
            "fetch origin main:main",
        ] {
            assert_eq!(code(&decide(&s, cmd)), "protected_ref", "{cmd}");
        }
        assert_eq!(code(&decide(&s, "update-ref --stdin")), "update_ref_stdin");
        assert_eq!(code(&decide(&s, "push --all origin")), "push_scope");
        assert_eq!(
            code(&decide(&s, "push origin HEAD:feat/x")),
            "ref_not_yours"
        );
        assert_eq!(
            code(&decide(&s, "update-ref refs/heads/agend/t-2/x abc")),
            "ref_not_yours"
        );
        for cmd in [
            "push",
            "push origin HEAD",
            "push -u origin agend/t-1/fix",
            "push origin HEAD:agend/t-1/fix",
            "update-ref refs/heads/agend/t-1/fix abc123",
            "update-ref HEAD abc123",
            "fetch origin",
            "fetch origin main:refs/remotes/origin/main",
        ] {
            assert_eq!(code(&decide(&s, cmd)), "run", "{cmd}");
        }
        assert_eq!(code(&decide(&review(), "push")), "review_readonly");
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
            ("clean -nd", None),
            ("clean -fdn", None),
            ("checkout -- .", Some("checkout")),
            ("checkout .", Some("checkout")),
            ("checkout src/lib.rs", Some("checkout")),
            ("checkout main -- src", Some("checkout")),
            ("checkout -f", Some("checkout")),
            ("checkout -p", Some("checkout")),
            ("restore .", Some("restore")),
            ("restore --staged .", None),
            ("restore --staged --worktree .", Some("restore")),
            ("switch --discard-changes agend/t-1/fix", Some("switch")),
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
        let args = argv("commit -m x");
        let parsed = parse(&args);
        let protected = ProtectedRefs::new(&[]);
        let mut input = Input {
            args: &parsed,
            snapshot: Ok(&s),
            location: Location::Canonical,
            env_retargets: true,
            protected: &protected,
            dir: Path::new("/repo"),
            is_commit: &commits,
        };
        assert_eq!(code(&classify(&input)), "git_env_retarget");
        input.location = Location::Worktree;
        assert_eq!(code(&classify(&input)), "run", "hooks run with GIT_DIR set");
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

    #[test]
    fn config_reads_and_writes() {
        for (cmd, read) in [
            ("config user.name", true),
            ("config --get user.name", true),
            ("config -l", true),
            ("config user.name x", false),
            ("config --unset user.name", false),
            ("config alias.co checkout", false),
        ] {
            let rest = argv(cmd)[1..].to_vec();
            assert_eq!(config_is_read(&rest), read, "{cmd}");
        }
    }
}
