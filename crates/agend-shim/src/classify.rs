//! Classifies a git invocation: run it as is, route it into the bound
//! worktree, or refuse it with the exact next step.
//!
//! Structure (each closes a class of bypass, not one spelling):
//! - Options of every subcommand whose verdict depends on them are parsed
//!   with exact spellings only (`opts`, `specs`); abbreviations and
//!   unlisted options are refused.
//! - Ref destinations must be explicit and checkable: pushes need
//!   `src:dst` refspecs, fetch refmaps (command line and config) must land in
//!   `refs/remotes/` or the agent's namespace, config that picks destinations
//!   cannot be set (`config_keys`), and symbolic refs are followed before a
//!   destination is checked.
//! - A write acts on the bound worktree only: `--work-tree`,
//!   `GIT_WORK_TREE` and `GIT_INDEX_FILE` pointing elsewhere are refused, and
//!   so is a git dir (`--git-dir`, `GIT_DIR`) without a work tree when git
//!   runs outside the bound worktree (git would use that directory).
//! - Anything that could leave the bound branch (DWIM checkout, `<x> --`,
//!   rebase of another branch, `stash branch`) is refused unless its target
//!   is the bound branch itself.
//! - Foreign repos stay the agent's business, except clones of the team repo
//!   and pushes whose destination is the team repo (`team`).
//!
//! Pure apart from `Probe` (the questions that need git itself) and
//! existence checks on the bound worktree.
//!
//! Must NOT: guess a binding when the snapshot is missing.

use crate::Refusal;
use crate::binding::{Binding, Snapshot, SnapshotError};
use crate::config_keys;
use crate::location::Location;
use crate::protected_ref::ProtectedRefs;
use std::path::{Path, PathBuf};

mod commands;
mod opts;
mod refs;
mod specs;
#[cfg(test)]
mod tests;

use commands::{BranchOp, Guard};
pub use opts::Parsed;

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
    /// Config keys set with `-c key[=value]` or `--config-env key=VAR`.
    pub config_keys: Vec<String>,
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
const VALUE_GLOBALS: &[&str] = &["--namespace", "--super-prefix", "--attr-source"];

/// Splits leading global options from the subcommand. git matches globals
/// exactly (no abbreviations); an unrecognised option ends the globals and
/// becomes the "subcommand", so it is refused as unknown.
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
        } else if a == "-c" || a == "--config-env" {
            let Some(v) = args.get(i + 1) else { break };
            g.config_keys.push(config_key_of(v));
            i += 1;
        } else if let Some(v) = a.strip_prefix("--config-env=") {
            g.config_keys.push(config_key_of(v));
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

/// `key=value` / `key` → `key`.
fn config_key_of(v: &str) -> String {
    v.split('=').next().unwrap_or_default().to_string()
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

/// Questions only git can answer, asked lazily.
pub trait Probe {
    /// Where `full_ref` finally points if it is a symbolic ref (following
    /// chains), in the repo the command acts on.
    fn symref_target(&self, full_ref: &str) -> Option<String>;
    /// `git config --get-regexp <regex>` in the repo the command acts on.
    fn config(&self, regex: &str) -> Vec<(String, String)>;
    /// A foreign repo: whether one of its remotes is the team repo.
    fn is_team_clone(&self) -> bool;
    /// A foreign repo: whether push destination `dest` is the team repo.
    fn is_team_remote(&self, dest: &str) -> bool;
}

/// The caller's git environment, as it affects where a write lands.
#[derive(Debug, Clone)]
pub struct GitEnv {
    /// `GIT_DIR`/`GIT_WORK_TREE`/`GIT_COMMON_DIR`/`GIT_INDEX_FILE` are set.
    pub retargets: bool,
    /// The work tree the caller chose (`--work-tree` or `GIT_WORK_TREE`),
    /// resolved against the directory git runs in.
    pub work_tree: Option<PathBuf>,
    /// `GIT_INDEX_FILE`, resolved likewise.
    pub index_file: Option<PathBuf>,
    /// Whether a git dir was given (`--git-dir`, `--bare` or `GIT_DIR`).
    /// Without a work tree, git then uses the directory it runs in as the
    /// work tree (the top of it, not the checkout that owns the git dir).
    pub git_dir: bool,
    /// Keys set through `GIT_CONFIG_COUNT` / `GIT_CONFIG_PARAMETERS`, or why
    /// they could not be read.
    pub config_keys: Result<Vec<String>, String>,
}

impl Default for GitEnv {
    fn default() -> GitEnv {
        GitEnv {
            retargets: false,
            work_tree: None,
            index_file: None,
            git_dir: false,
            config_keys: Ok(Vec::new()),
        }
    }
}

pub struct Input<'a> {
    pub args: &'a GitArgs,
    pub snapshot: Result<&'a Snapshot, &'a SnapshotError>,
    pub location: Location,
    pub env: &'a GitEnv,
    pub protected: &'a ProtectedRefs,
    /// Where the caller pointed git (cwd with `-C` applied), for messages.
    pub dir: &'a Path,
    pub probe: &'a dyn Probe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Read,
    /// Writes only remote-tracking refs (after its checks): no binding
    /// needed, routed like a read.
    Fetch,
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
    let rest = args.rest.as_slice();
    if input.location == Location::Foreign {
        return foreign(input, sub, rest);
    }
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
    if sub == "fast-import" {
        return Decision::Refuse(refs::refuse_stdin(sub));
    }
    let parsed = match specs::of(sub).map(|spec| opts::parse(spec, rest)) {
        None => None,
        Some(Ok(p)) => Some(p),
        Some(Err(u)) => return Decision::Refuse(refuse_option(sub, &u)),
    };
    let Some(kind) = kind(sub, rest, parsed.as_ref()) else {
        return Decision::Refuse(Refusal::new(
            "unknown_command",
            format!(
                "`git {sub}` is not a git command the agend shim knows (aliases and external git-* commands are not allowed for agents)"
            ),
            "run the underlying built-in git command directly; list them with: git help -a",
        ));
    };
    if kind == Kind::NewRepo {
        return Decision::pass();
    }
    if kind == Kind::Read {
        return route_read(input, binding);
    }
    if let Err(r) = check_config_channel(input) {
        return Decision::Refuse(r);
    }
    let guard = Guard {
        binding,
        protected: input.protected,
        probe: input.probe,
    };
    let parsed = parsed.unwrap_or_default();
    if kind == Kind::Fetch {
        return match refs::fetch(sub, &parsed, &guard) {
            Ok(()) => route_read(input, binding),
            Err(r) => Decision::Refuse(r),
        };
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
    if route.is_some() && input.env.retargets {
        return Decision::Refuse(Refusal::new(
            "git_env_retarget",
            "GIT_DIR / GIT_WORK_TREE / GIT_COMMON_DIR / GIT_INDEX_FILE point git outside your bound worktree"
                .to_string(),
            format!(
                "unset GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GIT_INDEX_FILE and run plain `git {sub} ...`; it runs in {}",
                binding.worktree().display()
            ),
        ));
    }
    if let Err(r) = check_work_tree(sub, input, binding) {
        return Decision::Refuse(r);
    }
    if let Err(r) = guard.own_branch_is_real(sub) {
        return Decision::Refuse(r);
    }
    let snapshot_op = match check_write(sub, rest, &parsed, &guard, binding) {
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
    if !outside || input.env.retargets || !binding.worktree().is_dir() {
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

/// A foreign repo is the agent's own business, except a clone of the team
/// repo (writes refused) and a push whose destination is the team repo.
fn foreign(input: &Input, sub: &str, rest: &[String]) -> Decision {
    let parsed = specs::of(sub).and_then(|spec| opts::parse(spec, rest).ok());
    if matches!(
        kind(sub, rest, parsed.as_ref()),
        Some(Kind::Read | Kind::NewRepo)
    ) {
        return Decision::pass();
    }
    let binding = input.snapshot.ok().and_then(|s| s.binding.as_ref());
    let next = match binding {
        Some(b) => format!(
            "change the team repo only from your bound worktree: cd {} and run it there",
            b.worktree().display()
        ),
        None => {
            "run `agend status`; the daemon gives you a worktree when it assigns a task".to_string()
        }
    };
    if input.probe.is_team_clone() {
        return Decision::Refuse(Refusal::new(
            "team_clone",
            format!(
                "`git {sub}` in {}: this repo is a clone of the team repo, which agents change only through their bound worktree",
                input.dir.display()
            ),
            next,
        ));
    }
    if sub == "push" {
        let dests = rest
            .iter()
            .filter(|a| !a.starts_with('-'))
            .map(String::as_str)
            .chain(rest.iter().filter_map(|a| a.strip_prefix("--repo=")));
        for dest in dests {
            if input.probe.is_team_remote(dest) {
                return Decision::Refuse(Refusal::new(
                    "team_remote",
                    format!("pushing to {dest} is refused: it is the team repo"),
                    next,
                ));
            }
        }
    }
    Decision::pass()
}

/// Config set for this one call (`-c`, `--config-env`, `GIT_CONFIG_*`) may
/// only touch keys an agent may set at all.
fn check_config_channel(input: &Input) -> Result<(), Refusal> {
    let env_keys = match &input.env.config_keys {
        Ok(keys) => keys.as_slice(),
        Err(e) => {
            return Err(Refusal::new(
                "config_override",
                format!("cannot read the config set in the environment: {e}"),
                "unset GIT_CONFIG_PARAMETERS GIT_CONFIG_COUNT and retry",
            ));
        }
    };
    let bad = input
        .args
        .config_keys
        .iter()
        .chain(env_keys)
        .find(|k| !config_keys::allowed(k));
    match bad {
        None => Ok(()),
        Some(key) => Err(Refusal::new(
            "config_override",
            format!(
                "setting {key} for this command is refused: that config can redirect refs, the work tree, hooks or command names"
            ),
            format!(
                "drop the -c / --config-env / GIT_CONFIG_* setting; only these keys may be set: {}",
                config_keys::ALLOWED_HINT
            ),
        )),
    }
}

/// A write must act on the bound worktree: a work tree or index chosen by
/// the caller must be the bound worktree's own. A git dir given without a
/// work tree makes the directory git runs in the work tree, so that
/// directory must be the bound worktree (hooks run there).
fn check_work_tree(sub: &str, input: &Input, binding: &Binding) -> Result<(), Refusal> {
    let wt = binding.worktree();
    let bound = std::fs::canonicalize(wt).ok();
    let same = |p: &Path| {
        std::fs::canonicalize(p)
            .ok()
            .is_some_and(|c| Some(c) == bound)
    };
    let refuse = |what: &str, p: &Path| {
        Refusal::new(
            "work_tree_retarget",
            format!(
                "{what} {} is not your bound worktree {}; `git {sub}` would change another checkout",
                p.display(),
                wt.display()
            ),
            format!(
                "drop --git-dir / --work-tree, unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE, and run `git {sub} ...` in {}",
                wt.display()
            ),
        )
    };
    if let Some(p) = &input.env.work_tree
        && !same(p)
    {
        return Err(refuse("work tree", p));
    }
    // Routed calls drop `--git-dir` (and refuse `GIT_DIR`) before this.
    if input.location == Location::Worktree
        && input.env.work_tree.is_none()
        && input.env.git_dir
        && !same(input.dir)
    {
        return Err(refuse(
            "with a git dir set and no work tree, git uses the current directory as the work tree:",
            input.dir,
        ));
    }
    if let Some(p) = &input.env.index_file {
        let gitdir = crate::location::gitdir_of_checkout(wt);
        let parent = p.parent().and_then(|d| std::fs::canonicalize(d).ok());
        let inside = matches!((&gitdir, &parent), (Some(g), Some(d)) if d.starts_with(g));
        if !inside {
            return Err(refuse("index file", p));
        }
    }
    Ok(())
}

/// Command-specific checks for a bound agent's write. Returns the name of the
/// destructive operation to snapshot, if any.
fn check_write(
    sub: &str,
    rest: &[String],
    p: &Parsed,
    g: &Guard,
    binding: &Binding,
) -> Result<Option<&'static str>, Refusal> {
    match sub {
        "checkout" => commands::checkout(p, g, binding),
        "switch" => commands::switch(p, g, binding),
        "branch" => commands::branch(p, g, binding).map(|_| None),
        "tag" => commands::tag(p, g).map(|_| None),
        "rebase" => commands::rebase(p, g, binding).map(|_| None),
        "stash" => commands::stash(rest, binding).map(|_| None),
        "config" => commands::config(p).map(|_| None),
        "remote" => commands::remote(rest, g).map(|_| None),
        "push" => refs::push(p, g, binding).map(|_| None),
        "pull" => refs::fetch(sub, p, g)
            .and_then(|_| commands::pull_rebase(p, g))
            .map(|_| None),
        "update-ref" => refs::update_ref(p, g, binding).map(|_| None),
        "symbolic-ref" => refs::symbolic_ref(p, g, binding).map(|_| None),
        "reset" => Ok(p.any(&["--hard", "--merge", "--keep"]).then_some("reset")),
        "clean" => Ok((!p.has("--dry-run")).then_some("clean")),
        "restore" => Ok((!p.has("--staged") || p.has("--worktree")).then_some("restore")),
        "read-tree" => Ok((p.has("-u") && !p.has("--dry-run")).then_some("read-tree")),
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
    "unpack-objects",
];

/// `parsed` is the exact-spelling parse for subcommands that have a spec;
/// without it (foreign repos, parse failure) option-dependent reads count as
/// writes.
fn kind(sub: &str, rest: &[String], parsed: Option<&Parsed>) -> Option<Kind> {
    if matches!(sub, "init" | "clone") {
        return Some(Kind::NewRepo);
    }
    if READ.contains(&sub) || sub == "worktree" {
        return Some(Kind::Read); // only `worktree list` gets this far
    }
    if sub == "fetch" {
        return Some(Kind::Fetch);
    }
    if !WRITE.contains(&sub) {
        return None;
    }
    let first = first_positional(rest);
    let with = |f: fn(&Parsed) -> bool| parsed.is_some_and(f);
    let read = match sub {
        "branch" => with(|p| commands::branch_op(p) == BranchOp::List),
        "tag" => with(commands::tag_is_list),
        "config" => with(commands::config_is_read),
        "symbolic-ref" => with(|p| !refs::symbolic_ref_writes(p)),
        "stash" => matches!(first, Some("list" | "show")),
        "remote" => matches!(first, None | Some("show" | "get-url")),
        "reflog" => !matches!(first, Some("expire" | "delete")),
        "notes" => matches!(first, None | Some("list" | "show")),
        "submodule" => matches!(first, None | Some("status" | "summary")),
        "sparse-checkout" => matches!(first, Some("list")),
        _ => false,
    };
    Some(if read { Kind::Read } else { Kind::Write })
}

fn first_positional(rest: &[String]) -> Option<&str> {
    rest.iter()
        .map(String::as_str)
        .find(|a| !a.starts_with('-'))
}

// ── refusals shared by every command ────────────────────────────────────

fn refuse_option(sub: &str, u: &opts::Unknown) -> Refusal {
    let reason = match u.like {
        Some(full) => format!(
            "`{}` looks like an abbreviation of `--{full}`; the agend shim only accepts options spelled in full for `git {sub}`",
            u.arg
        ),
        None => format!(
            "`{}` is not an option the agend shim accepts for `git {sub}` (abbreviated, attached or unsupported spellings are refused)",
            u.arg
        ),
    };
    Refusal::new(
        "option_unknown",
        reason,
        format!(
            "spell the option in full as `git {sub} -h` lists it, or run without it; if a real option is missing, ask: agend ask \"shim option for git {sub}\""
        ),
    )
}

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
