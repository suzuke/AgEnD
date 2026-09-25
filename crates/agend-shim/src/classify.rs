//! Classifies a git invocation: run it as is, route it into the bound
//! worktree, or refuse it with the exact next step. Only what the agend
//! hooks (`hook`, which see every ref git changes) cannot see is decided
//! here: where a write acts (git's `location` answer; routed from the
//! workspace and canonical checkout keeping the subdirectory, refused from
//! another worktree or with a git dir / work tree / index named elsewhere);
//! leaving the bound branch with `checkout`/`switch` and changing worktrees;
//! snapshots before destructive commands (v1 agentic-git's scope); config
//! that would skip the hooks; and repos without hooks (the team's remote
//! and its clones, `team`). Options are matched generously: a false match
//! only adds a snapshot or refuses an unusual spelling.
//!
//! Must NOT: guess a binding when the snapshot is missing.

use crate::Refusal;
use crate::binding::{Binding, Snapshot, SnapshotError};
use crate::hook::stay_hint;
use crate::location::{Location, Resolved};
use crate::protected_ref::ProtectedRefs;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

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
    /// Values of `-c` and `--config-env` (`key=value`, `key=VAR`).
    pub config: Vec<String>,
    /// `--version`, `--help` and similar: git prints something and exits.
    pub info_only: bool,
    /// argv indexes of globals that retarget git (`-C`, `--git-dir`,
    /// `--work-tree`, `--bare`); dropped when routing.
    pub retarget_indexes: Vec<usize>,
}

const INFO_GLOBALS: &str = "--version -v --help -h --html-path --man-path --info-path --exec-path";
const FLAG_GLOBALS: &str = "-p --paginate -P --no-pager --no-replace-objects --literal-pathspecs --glob-pathspecs \
    --noglob-pathspecs --icase-pathspecs --no-optional-locks --no-lazy-fetch --no-advice";
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
        if listed(INFO_GLOBALS, a) {
            g.info_only = true;
        } else if listed(FLAG_GLOBALS, a) {
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
            g.config.push(v.clone());
            i += 1;
        } else if let Some(v) = a.strip_prefix("--config-env=") {
            g.config.push(v.to_string());
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
        /// Run in this directory of the bound worktree (`-C`, retargeting
        /// globals dropped) instead of where the caller pointed git.
        route: Option<PathBuf>,
        /// Take a snapshot first; the name of the destructive operation.
        snapshot: Option<&'static str>,
        /// One line for the agent (stderr) before running.
        note: Option<String>,
    },
    Refuse(Refusal),
}

/// Run as typed.
pub const PASS: Decision = Decision::Run {
    route: None,
    snapshot: None,
    note: None,
};

/// Questions only git (or the file system) can answer, asked lazily.
pub trait Probe {
    /// A foreign repo: whether it is the team's remote itself, or one of its
    /// remotes is the team repo (a clone).
    fn is_team_repo(&self) -> bool;
    /// A foreign repo: whether push destination `dest` is the team repo.
    fn is_team_remote(&self, dest: &str) -> bool;
    /// `git rev-parse <args>` in the bound worktree: trimmed stdout, if it
    /// succeeded.
    fn rev_parse(&self, args: &[&str]) -> Option<String>;
    /// Whether the bound worktree has the agend hooks (`hook::MARKER_FILE`).
    fn hooks_installed(&self) -> bool;
}

/// The caller's git environment, as it affects where a write lands.
#[derive(Debug, Clone, Default)]
pub struct GitEnv {
    /// `GIT_DIR`/`GIT_WORK_TREE`/`GIT_COMMON_DIR`/`GIT_INDEX_FILE` are set
    /// (routing cannot drop them, so a call that needs routing is refused).
    pub retargets: bool,
    /// `GIT_INDEX_FILE`, resolved against the directory git runs in.
    pub index_file: Option<PathBuf>,
    /// Values of `GIT_CONFIG_PARAMETERS` and `GIT_CONFIG_KEY_<n>`.
    pub config: Vec<String>,
}

pub struct Input<'a> {
    pub args: &'a GitArgs,
    pub snapshot: Result<&'a Snapshot, &'a SnapshotError>,
    pub location: Location,
    /// Git's answer for the call (`None`: not a repo, or not resolved).
    pub resolved: Option<&'a Resolved>,
    pub env: &'a GitEnv,
    /// Where the caller pointed git (cwd with `-C` applied), for messages.
    pub dir: &'a Path,
    pub probe: &'a dyn Probe,
}

/// Read-only, or creating a new repo elsewhere (`init`, `clone`): no
/// binding needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Read,
    Write,
}

/// Whether the decision depends on where the call acts. Calls that print
/// and exit, or create a repo, do not need git asked first.
pub fn needs_location(args: &GitArgs) -> bool {
    !args.info_only && !matches!(args.sub.as_deref(), None | Some("init" | "clone"))
}

pub fn classify(input: &Input) -> Decision {
    let args = input.args;
    let Some(sub) = args.sub.as_deref() else {
        return PASS;
    };
    if args.info_only {
        return PASS;
    }
    let rest = args.rest.as_slice();
    if input.location == Location::Foreign {
        return foreign(input, sub, rest);
    }
    if let Err(r) = check_hooks_kept(sub, rest, input) {
        return Decision::Refuse(r);
    }
    let binding = input.snapshot.ok().and_then(|s| s.binding.as_ref());
    if sub == "worktree" && first_positional(rest) != Some("list") {
        return Decision::Refuse(refuse_worktree(rest, binding));
    }
    match kind(sub, rest) {
        None => Decision::Refuse(Refusal::new(
            "unknown_command",
            format!(
                "`git {sub}` is not a git command the agend shim knows (aliases and external git-* commands are not allowed for agents)"
            ),
            "run the underlying built-in git command directly; list them with: git help -a",
        )),
        Some(Kind::Read) => route_read(input, binding),
        Some(Kind::Write) => write(input, sub, rest),
    }
}

fn write(input: &Input, sub: &str, rest: &[String]) -> Decision {
    let refuse = Decision::Refuse;
    let snapshot = match input.snapshot {
        Ok(s) => s,
        Err(e) => return refuse(refuse_no_binding(sub, e)),
    };
    let Some(binding) = snapshot.binding.as_ref() else {
        return refuse(refuse_unbound(sub, input.location, input.dir));
    };
    let wt = binding.worktree();
    if !wt.is_dir() {
        return refuse(Refusal::new(
            "worktree_missing",
            format!("your bound worktree {} does not exist", wt.display()),
            "run `agend status`; the daemon re-creates or re-assigns the worktree",
        ));
    }
    let named = input.args.git_dir.is_some() || input.args.work_tree.is_some();
    let no_work_tree = input.resolved.is_some_and(|r| r.work_tree.is_none());
    if input.location != Location::Worktree && no_work_tree && !named {
        // e.g. cwd inside the canonical `.git`: git itself would refuse a
        // work-tree write there; routing must not make it act anyway.
        return refuse(Refusal::new(
            "route_dir_missing",
            format!(
                "you ran `git {sub}` in {}, inside a git directory (git sees no work tree there), so the shim will not run it in your bound worktree",
                input.dir.display()
            ),
            format!("cd {} and run it there", wt.display()),
        ));
    }
    let route = match input.location {
        Location::Worktree => None,
        _ => match route_dir(sub, input, binding) {
            Ok(dir) => Some(dir),
            Err(r) => return refuse(r),
        },
    };
    if route.is_some() && input.env.retargets {
        return refuse(Refusal::new(
            "git_env_retarget",
            "GIT_DIR / GIT_WORK_TREE / GIT_COMMON_DIR / GIT_INDEX_FILE make git act outside your bound worktree",
            format!(
                "unset GIT_DIR GIT_WORK_TREE GIT_COMMON_DIR GIT_INDEX_FILE and run plain `git {sub} ...`; it runs in {}",
                wt.display()
            ),
        ));
    }
    if let Err(r) = check_work_tree(sub, input, binding) {
        return refuse(r);
    }
    if !input.probe.hooks_installed() {
        return refuse(Refusal::new(
            "hooks_missing",
            format!(
                "your bound worktree {} has no agend git hooks, which guard protected refs, so `git {sub}` is refused",
                wt.display()
            ),
            "run `agend status`; the daemon installs the hooks when it binds your worktree",
        ));
    }
    let protected = ProtectedRefs::new(&snapshot.protected_refs);
    if let Some(target) = leaves_branch(sub, rest, binding, input.probe) {
        return refuse(refuse_switch(sub, &target, binding, &protected));
    }
    if let Some(flag) = copy_or_rename(sub, rest) {
        return refuse(refuse_copy_or_rename(flag, binding));
    }
    if let Some(r) = untracked_ref_write(sub, rest, binding) {
        return refuse(r);
    }
    let note = match (&route, input.location) {
        (Some(to), Location::Canonical) => Some(routed_note(to, input.dir)),
        _ => None,
    };
    Decision::Run {
        route,
        snapshot: destructive(sub, rest),
        note,
    }
}

/// Read-only commands: run in the bound worktree when the caller is in the
/// workspace (no repo) or the canonical checkout, else as is. In another
/// worktree a read runs where it was typed: it shows that worktree, which
/// is what the agent asked for, and routing it would be a surprise.
fn route_read(input: &Input, binding: Option<&Binding>) -> Decision {
    let Some(binding) = binding else {
        return PASS;
    };
    let outside = matches!(input.location, Location::Canonical | Location::NoRepo);
    if !outside || input.env.retargets || !binding.worktree().is_dir() {
        return PASS;
    }
    let sub = input.args.sub.as_deref().unwrap_or("");
    let to = match route_dir(sub, input, binding) {
        Ok(to) => to,
        Err(r) => return Decision::Refuse(r),
    };
    let note = (input.location != Location::NoRepo).then(|| routed_note(&to, input.dir));
    Decision::Run {
        route: Some(to),
        snapshot: None,
        note,
    }
}

/// Where a call from outside the bound worktree runs: the same directory
/// inside the bound worktree as the caller's inside its checkout (git's
/// `--show-prefix`), so `.` and other relative pathspecs keep their scope.
/// From the workspace (no repo) that is the worktree's top.
///
/// Refused instead of routed: a call from another worktree of the team
/// repo (the agent is in the wrong directory; acting on its own worktree
/// would surprise it), and a directory the bound worktree does not have.
fn route_dir(sub: &str, input: &Input, binding: &Binding) -> Result<PathBuf, Refusal> {
    let wt = binding.worktree();
    let prefix = input
        .resolved
        .filter(|_| input.location != Location::NoRepo)
        .map_or(Path::new(""), |r| r.prefix.as_path());
    // Joined by component: `Path::join("")` would add a trailing slash.
    let target = prefix.components().fold(wt.to_path_buf(), |p, c| p.join(c));
    let target_ok = prefix
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
        && target.is_dir()
        && match (std::fs::canonicalize(&target), std::fs::canonicalize(wt)) {
            (Ok(t), Ok(w)) => t.starts_with(w),
            _ => false,
        };
    if input.location == Location::OtherWorktree {
        let there = input.resolved.map_or(input.dir, Resolved::root);
        let go = if target_ok { &target } else { wt };
        return Err(Refusal::new(
            "other_worktree",
            format!(
                "you ran `git {sub}` in {}, which is in another worktree of the team repo ({}), not in your bound worktree {}",
                input.dir.display(),
                there.display(),
                wt.display()
            ),
            format!("cd {} and run it there", go.display()),
        ));
    }
    if target_ok {
        return Ok(target);
    }
    Err(Refusal::new(
        "route_dir_missing",
        format!(
            "you ran `git {sub}` in {}, outside your bound worktree; the same directory ({}) does not exist in your bound worktree {}, so the shim will not run it there (relative paths would mean something else)",
            input.dir.display(),
            prefix.display(),
            wt.display()
        ),
        format!(
            "cd {} (your bound worktree), then cd to the directory you meant and run it there",
            wt.display()
        ),
    ))
}

fn routed_note(to: &Path, dir: &Path) -> String {
    format!(
        "agend-shim: running in your bound worktree {} (you ran git in {})",
        to.display(),
        dir.display()
    )
}

/// A foreign repo has no agend hooks and is the agent's own business,
/// except a clone of the team repo or the team's remote itself (writes
/// refused) and a push whose destination is the team repo (T5).
fn foreign(input: &Input, sub: &str, rest: &[String]) -> Decision {
    if kind(sub, rest) == Some(Kind::Read) {
        return PASS;
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
    if input.probe.is_team_repo() {
        let repo = input.resolved.map_or(input.dir, Resolved::root);
        return Decision::Refuse(Refusal::new(
            "team_clone",
            format!(
                "`git {sub}` in {}: this repo is the team's remote or a clone of the team repo, which agents change only through their bound worktree",
                repo.display()
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
    PASS
}

/// The agend hooks must run: no `core.hooksPath` set for one call (`-c`,
/// `--config-env`, `GIT_CONFIG_*`), no `push --no-verify` (skips pre-push).
fn check_hooks_kept(sub: &str, rest: &[String], input: &Input) -> Result<(), Refusal> {
    let mut config = input.args.config.iter().chain(&input.env.config);
    if config.any(|v| v.to_ascii_lowercase().contains("core.hookspath")) {
        return Err(Refusal::new(
            "hooks_skipped",
            format!(
                "setting core.hooksPath for `git {sub}` would skip the agend git hooks that guard protected refs"
            ),
            "drop the -c / --config-env / GIT_CONFIG_* core.hooksPath setting; your project's own hooks still run (the agend hooks chain to them)",
        ));
    }
    if sub == "push" && options(rest).any(|a| long(a, "--no-verify")) {
        return Err(Refusal::new(
            "hooks_skipped",
            "`git push --no-verify` would skip the agend pre-push hook",
            "push without --no-verify: git push origin HEAD:refs/heads/<your branch>",
        ));
    }
    Ok(())
}

/// A write must act on the bound worktree. In its git dir (`Worktree`),
/// the work tree git resolved must be the bound worktree (a git dir given
/// without a work tree makes git use the directory it runs in; hooks run in
/// the worktree) and `GIT_INDEX_FILE` must be inside its git dir. A routed
/// write drops the caller's `--git-dir` / `--work-tree`, so naming one that
/// resolves elsewhere is refused rather than silently rewritten.
fn check_work_tree(sub: &str, input: &Input, binding: &Binding) -> Result<(), Refusal> {
    let wt = binding.worktree();
    let refuse = |reason: String| {
        Refusal::new(
            "work_tree_retarget",
            reason,
            format!(
                "drop --git-dir / --work-tree, unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE, and run `git {sub} ...` in {}",
                wt.display()
            ),
        )
    };
    if input.location != Location::Worktree {
        let named = input.args.git_dir.is_some() || input.args.work_tree.is_some();
        return match (named, input.resolved) {
            (false, _) => Ok(()),
            (true, Some(r)) => Err(refuse(format!(
                "--git-dir / --work-tree point `git {sub}` at {}, which is not your bound worktree {}",
                r.root().display(),
                wt.display()
            ))),
            (true, None) => Err(refuse(format!(
                "with --git-dir / --work-tree as given, git finds no repository for `git {sub}`; your bound worktree is {}",
                wt.display()
            ))),
        };
    }
    let bound = std::fs::canonicalize(wt).ok();
    let resolved = input.resolved;
    let other = |subject: String| {
        refuse(format!(
            "{subject} is not your bound worktree {}; `git {sub}` would change another checkout",
            wt.display()
        ))
    };
    match resolved.and_then(|r| r.work_tree.as_deref()) {
        None => {
            return Err(refuse(format!(
                "git would run `git {sub}` without a work tree, not in your bound worktree {}",
                wt.display()
            )));
        }
        Some(p) if Some(p) != bound.as_deref() => {
            return Err(other(format!(
                "the work tree git would use ({})",
                p.display()
            )));
        }
        Some(_) => {}
    }
    if let Some(p) = &input.env.index_file {
        let parent = p.parent().and_then(|d| std::fs::canonicalize(d).ok());
        let inside = matches!((resolved, &parent), (Some(r), Some(d)) if d.starts_with(&r.git_dir));
        if !inside {
            return Err(other(format!("the index file {}", p.display())));
        }
    }
    Ok(())
}

// ── leaving the bound branch ────────────────────────────────────────────

/// The target named when `checkout`/`switch` would leave the bound branch.
/// `checkout` with paths (after `--`, or several arguments, or one that is
/// not a commit) restores files instead; a new branch it would create from
/// a remote one is refused by the hook.
fn leaves_branch(sub: &str, rest: &[String], b: &Binding, probe: &dyn Probe) -> Option<String> {
    // Options that create a branch or detach HEAD.
    let (longs, letters): (&[&str], &str) = match sub {
        "checkout" => (&["--detach", "--orphan"], "bBd"),
        "switch" => (
            &["--detach", "--orphan", "--create", "--force-create"],
            "cCd",
        ),
        _ => return None,
    };
    if let Some(flag) =
        options(rest).find(|a| longs.iter().any(|l| long(a, l)) || short(a, letters))
    {
        return Some(flag.to_string());
    }
    let dashdash = rest.iter().position(|a| a == "--");
    let pos: Vec<&str> = rest[..dashdash.unwrap_or(rest.len())]
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-') || *a == "-")
        .collect();
    let target = *pos.first()?;
    if sub == "checkout" {
        let paths_after = dashdash.is_some_and(|i| i + 1 < rest.len());
        if paths_after || pos.len() > 1 {
            return None;
        }
        let names_commit = previous(target).is_some()
            || probe
                .rev_parse(&["--verify", "-q", &format!("{target}^{{commit}}")])
                .is_some();
        if dashdash.is_none() && !names_commit {
            return None;
        }
    }
    (!stays(target, b, probe)).then(|| target.to_string())
}

/// `-` and `@{-<n>}` (a previously checked-out branch) as a revision.
fn previous(target: &str) -> Option<String> {
    let n = match target {
        "-" => "1",
        t => t.strip_prefix("@{-")?.strip_suffix('}')?,
    };
    (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then(|| format!("@{{-{n}}}"))
}

/// Whether switching to `target` stays on the bound branch (`-` resolved by
/// git in the bound worktree).
fn stays(target: &str, b: &Binding, probe: &dyn Probe) -> bool {
    if matches!(target, "HEAD" | "@") {
        return true;
    }
    let Some(branch) = b.branch() else {
        return false;
    };
    let full = match previous(target) {
        Some(rev) => probe.rev_parse(&["--symbolic-full-name", &rev]),
        None => Some(target.to_string()),
    };
    full.is_some_and(|f| f == branch || f.strip_prefix("refs/heads/") == Some(branch))
}

fn refuse_switch(sub: &str, target: &str, b: &Binding, p: &ProtectedRefs) -> Refusal {
    let bound = b.branch().unwrap_or("a detached review head");
    let protected = if p.is_protected(&format!("refs/heads/{target}")) {
        format!(" ({target} is protected: only the daemon changes it)")
    } else {
        String::new()
    };
    let mut next = stay_hint(Some(b));
    if sub == "checkout" {
        next.push_str(
            ". To restore a file instead: git checkout -- <path>  or  git restore <path>",
        );
    }
    Refusal::new(
        "branch_switch",
        format!(
            "`git {sub} {target}` would leave your branch: you are bound to {bound}{protected}"
        ),
        next,
    )
}

/// `git branch -c/-C/-m/-M` (`--copy`, `--move`, any prefix, in a cluster
/// like `-fm`): git 2.39 writes the new name outside a ref transaction, so
/// the hook never sees it, and a hook refusal halfway through (deleting the
/// old name) loses a branch. A cluster ends at `-u`/`-t` (their value).
fn copy_or_rename<'a>(sub: &str, rest: &'a [String]) -> Option<&'a str> {
    (sub == "branch").then_some(())?;
    options(rest).find(|a| match a.strip_prefix("--") {
        Some(n) => {
            let n = n.split('=').next().unwrap_or(n);
            !n.is_empty() && ("move".starts_with(n) || "copy".starts_with(n))
        }
        None => {
            a.starts_with('-')
                && a[1..]
                    .chars()
                    .take_while(|c| !"ut".contains(*c))
                    .any(|c| "cCmM".contains(c))
        }
    })
}

fn refuse_copy_or_rename(flag: &str, b: &Binding) -> Refusal {
    let ns = b.namespace().unwrap_or_else(|| "agend/<task-id>/".into());
    Refusal::new(
        "branch_copy",
        format!(
            "`git branch {flag}` copies or renames a branch outside git's ref transaction, so the agend hook cannot check the new name"
        ),
        format!(
            "create the new branch instead (the hook checks it): git branch {ns}<name> <old>; if <old> is your own {ns} side branch, then delete it: git branch -D <old>"
        ),
    )
}

/// `symbolic-ref` writes (a target, `-d`, `-m`) and `reflog delete|expire`:
/// git 2.39 changes the ref outside a ref transaction, so the hook never
/// sees it (round 7: `symbolic-ref refs/heads/main <own>` repointed main,
/// `reflog delete --updateref main@{0}` moved it, `reflog expire --all`
/// wiped every branch's reflog). Reads stay: `symbolic-ref [-q] [--short]
/// <name>`, `reflog [show|exists]`.
fn untracked_ref_write(sub: &str, rest: &[String], b: &Binding) -> Option<Refusal> {
    let next = match sub {
        "symbolic-ref" => {
            let (flags, names): (Vec<&String>, Vec<&String>) =
                rest.iter().partition(|a| a.starts_with('-'));
            let read_flags = "-q --quiet --short --recurse --no-recurse";
            if names.len() == 1 && flags.iter().all(|a| listed(read_flags, a)) {
                return None;
            }
            let ns = b.namespace().unwrap_or_else(|| "agend/<task-id>/".into());
            format!(
                "read it with: git symbolic-ref <name>; to start a branch: git branch {ns}<name> <commit>"
            )
        }
        "reflog" if matches!(first_positional(rest), Some("expire" | "delete")) => {
            "read it with: git reflog show <ref>; agents do not delete or expire reflog entries"
                .into()
        }
        _ => return None,
    };
    Some(Refusal::new(
        "ref_outside_hook",
        format!(
            "`git {sub} {}` changes refs outside git's ref transaction, so the agend hook cannot check it",
            rest.join(" ")
        ),
        next,
    ))
}

// ── snapshots ───────────────────────────────────────────────────────────

/// Operations that are snapshotted (v1 agentic-git's scope).
const SNAPSHOT_OPS: &str =
    "reset clean checkout restore switch stash rm mv merge rebase pull cherry-pick revert am";

/// The name of the destructive operation to snapshot before, if any:
/// `reset --hard|--merge|--keep`, `clean` (any), `checkout` (any that is
/// allowed: paths, or `-f`), `restore` of the work tree, `switch -f` /
/// `--discard-changes`, `stash drop|clear`, `rm -f` / `mv -f` (not
/// `--cached`, not `-n`), and merge / rebase / pull / cherry-pick / revert /
/// am. `mv -f` overwrites a destination with uncommitted edits.
fn destructive(sub: &str, rest: &[String]) -> Option<&'static str> {
    let has = |l: &str| options(rest).any(|a| long(a, l));
    let has_short = |c: &str| options(rest).any(|a| short(a, c));
    let yes = match sub {
        "reset" => has("--hard") || has("--merge") || has("--keep"),
        "restore" => !(has("--staged") || has_short("S")) || has("--worktree") || has_short("W"),
        "switch" => has("--force") || has("--discard-changes") || has_short("f"),
        "stash" => matches!(first_positional(rest), Some("drop" | "clear")),
        "rm" | "mv" => {
            (has("--force") || has_short("f"))
                && !has("--cached")
                && !has("--dry-run")
                && !options(rest).any(|a| a == "-n")
        }
        _ => true,
    };
    SNAPSHOT_OPS
        .split_whitespace()
        .find(|o| *o == sub)
        .filter(|_| yes)
}

// ── option matching and command kinds ───────────────────────────────────

/// Arguments before `--`.
fn options(rest: &[String]) -> impl Iterator<Item = &str> {
    rest.iter().map(String::as_str).take_while(|a| *a != "--")
}

/// `arg` is the long option `l`, or any abbreviation of it (git accepts
/// every unambiguous prefix, even `--d` for `checkout --detach`; an
/// ambiguous one is git's error, so matching it too is harmless; `=value`
/// allowed). `--force` is itself an option, so it never abbreviates
/// `--force-create`.
fn long(arg: &str, l: &str) -> bool {
    let name = arg.split('=').next().unwrap_or(arg);
    name == l || (name.len() >= 3 && l.starts_with(name) && name != "--force")
}

/// `arg` is a cluster of short options containing one of `letters`.
fn short(arg: &str, letters: &str) -> bool {
    arg.len() > 1
        && arg.starts_with('-')
        && !arg.starts_with("--")
        && arg[1..].chars().any(|c| letters.contains(c))
}

const READ: &str = "init clone status log diff show blame annotate ls-files ls-tree rev-parse rev-list cat-file describe \
    shortlog grep for-each-ref show-ref name-rev merge-base merge-tree diff-tree diff-index \
    diff-files difftool count-objects check-ignore check-attr check-mailmap check-ref-format \
    var help version whatchanged format-patch range-diff cherry fsck verify-commit verify-tag \
    verify-pack archive show-branch patch-id ls-remote hash-object request-pull \
    interpret-trailers stripspace column get-tar-commit-id";

const WRITE: &str = "add rm mv commit reset restore checkout switch merge rebase cherry-pick revert pull push \
    fetch am apply clean gc prune repack pack-refs maintenance update-index read-tree \
    write-tree commit-tree mktree mktag update-ref symbolic-ref bisect lfs rerere config \
    filter-branch fast-import replace unpack-objects";

/// Read or write, by the subcommand and (for the mixed ones) its first
/// positional; `None` for a command the shim does not know.
fn kind(sub: &str, rest: &[String]) -> Option<Kind> {
    if listed(READ, sub) {
        return Some(Kind::Read);
    }
    let first = first_positional(rest);
    let read = match sub {
        // Listing forms only; anything with a name counts as a write.
        "branch" | "tag" => first.is_none(),
        "stash" => matches!(first, Some("list" | "show")),
        "remote" => matches!(first, None | Some("show" | "get-url")),
        "reflog" => !matches!(first, Some("expire" | "delete")),
        "submodule" => matches!(first, None | Some("status" | "summary")),
        "notes" => matches!(first, None | Some("list" | "show")),
        "sparse-checkout" | "worktree" => first == Some("list"),
        s if listed(WRITE, s) => false,
        _ => return None,
    };
    Some(if read { Kind::Read } else { Kind::Write })
}

/// Whether `word` is one of the whitespace-separated words of `list`.
fn listed(list: &str, word: &str) -> bool {
    list.split_whitespace().any(|w| w == word)
}

fn first_positional(rest: &[String]) -> Option<&str> {
    rest.iter()
        .map(String::as_str)
        .find(|a| !a.starts_with('-'))
}

// ── refusals shared by every command ────────────────────────────────────

fn refuse_worktree(rest: &[String], binding: Option<&Binding>) -> Refusal {
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
        format!("`git worktree {what}` is refused: only the daemon creates and removes worktrees"),
        next,
    )
}

fn refuse_no_binding(sub: &str, err: &SnapshotError) -> Refusal {
    let next = match err {
        SnapshotError::NotAnAgent => {
            "this git is the agend agent shim; agents get AGEND_HOME and AGEND_INSTANCE from their holder. Humans: use the git outside the agent PATH (e.g. /usr/bin/git)"
        }
        _ => {
            "run `agend status` (the daemon rewrites the snapshot). Until then read-only git commands run only inside a checkout: they are not routed to your worktree, so from your workspace use git -C <your worktree> status"
        }
    };
    Refusal::new(
        "no_binding",
        format!("`git {sub}` changes the repo, which needs a valid binding: {err}"),
        next,
    )
}

fn refuse_unbound(sub: &str, location: Location, dir: &Path) -> Refusal {
    let (code, reason) = match location {
        Location::Canonical => (
            "canonical_checkout",
            format!(
                "`git {sub}` would change the canonical checkout {} and you have no task bound; agents never change the canonical checkout",
                dir.display()
            ),
        ),
        _ => (
            "unbound",
            format!("`git {sub}` changes the repo but you have no task bound"),
        ),
    };
    Refusal::new(
        code,
        reason,
        "run `agend status` to see your assignment; the daemon gives you a worktree when it assigns a task. To start new work: agend task create \"<title>\"",
    )
}
