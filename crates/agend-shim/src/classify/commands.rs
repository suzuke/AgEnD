//! Per-command checks for a bound agent's git writes, on the exact-spelling
//! parse (`opts`): branch switching and creation, branch/tag edits, config
//! writes, rebase, stash, remote, and which operations destroy working-tree
//! state. Ref destinations (push/fetch/update-ref/symbolic-ref) are in
//! `refs`.
//!
//! Rule for anything that can leave the bound branch: allowed only when its
//! target is the bound branch itself; there is no guessing whether an
//! argument "is probably a path".
//!
//! Must NOT: allow writes to protected refs, whatever the binding.

use super::opts::Parsed;
use super::{Probe, first_positional};
use crate::Refusal;
use crate::binding::Binding;
use crate::config_keys;
use crate::protected_ref::ProtectedRefs;
use agend_core::model::BRANCH_NAMESPACE;

/// What every check needs: the binding (if any), the protected refs, and
/// git for symbolic refs and config.
pub(crate) struct Guard<'a> {
    pub(crate) binding: Option<&'a Binding>,
    pub(crate) protected: &'a ProtectedRefs,
    pub(crate) probe: &'a dyn Probe,
}

impl Guard<'_> {
    /// The bound branch must be a real branch: if it were a symbolic ref,
    /// every commit on it would move the ref it points to.
    pub(crate) fn own_branch_is_real(&self, sub: &str) -> Result<(), Refusal> {
        let Some(branch) = self.binding.and_then(Binding::branch) else {
            return Ok(());
        };
        let full = format!("refs/heads/{branch}");
        match self.probe.symref_target(&full) {
            None => Ok(()),
            Some(target) => Err(Refusal::new(
                "symref",
                format!(
                    "your branch {branch} is a symbolic ref to {target}: `git {sub}` would write {target}"
                ),
                "ask a human to restore it: agend ask \"my task branch became a symbolic ref\"",
            )),
        }
    }

    /// Whether config `key` (a boolean) is on in the repo.
    pub(crate) fn config_on(&self, key: &str) -> bool {
        let regex = format!("^{}$", key.replace('.', r"\."));
        self.probe
            .config(&regex)
            .iter()
            .next_back()
            .is_some_and(|(_, v)| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "" | "true" | "yes" | "on" | "1"
                )
            })
    }
}

// ── names ───────────────────────────────────────────────────────────────

pub(crate) fn short_branch(name: &str) -> &str {
    name.strip_prefix("refs/heads/").unwrap_or(name)
}

pub(crate) fn is_current(target: &str, binding: &Binding) -> bool {
    matches!(target, "HEAD" | "@") || binding.branch() == Some(short_branch(target))
}

pub(crate) fn own_namespace(binding: &Binding) -> Option<String> {
    match binding {
        Binding::Work { task_id, .. } => Some(format!("{BRANCH_NAMESPACE}{task_id}/")),
        Binding::Review { .. } => None,
    }
}

pub(crate) fn in_own_namespace(name: &str, binding: &Binding) -> bool {
    own_namespace(binding).is_some_and(|ns| short_branch(name).starts_with(&ns))
}

pub(crate) fn stay_hint(binding: Option<&Binding>) -> String {
    match binding {
        Some(Binding::Work {
            branch, worktree, ..
        }) => format!(
            "stay on {branch} in {}. To read another branch without switching: git log <branch>, git show <branch>:<path>, git diff <branch>...HEAD. For other work: agend task create \"<title>\"",
            worktree.display()
        ),
        Some(Binding::Review { head, .. }) => format!(
            "this is a review of {head}; inspect with git log / git show / git diff, then run: agend review approve  or  agend review changes \"<what to fix>\""
        ),
        None => "run `agend status` to see your assignment".to_string(),
    }
}

pub(crate) fn refuse_switch(target: &str, binding: &Binding, p: &ProtectedRefs) -> Refusal {
    let bound = binding.branch().unwrap_or("a detached review head");
    let protected = if p.names_protected(target) {
        format!(" ({target} is protected: only the daemon changes it)")
    } else {
        String::new()
    };
    Refusal::new(
        "branch_switch",
        format!("switching to {target} is refused: you are bound to {bound}{protected}"),
        stay_hint(Some(binding)),
    )
}

fn refuse_create(name: &str, binding: &Binding) -> Refusal {
    let reason = match own_namespace(binding) {
        Some(ns) => format!(
            "creating branch {name} is refused: agents may only create branches under {ns} (the daemon owns branch names)"
        ),
        None => format!("creating branch {name} is refused: review bindings are read-only"),
    };
    Refusal::new("branch_create", reason, stay_hint(Some(binding)))
}

// ── checkout / switch ───────────────────────────────────────────────────

/// Options that create a branch; the value is its name (`--track` derives
/// the name from the remote branch).
const CREATE: &[&str] = &[
    "-b",
    "-B",
    "--orphan",
    "--create",
    "--force-create",
    "--track",
];

fn created(p: &Parsed) -> Option<String> {
    let opt = p.first_of(CREATE)?;
    let name = match opt {
        "--track" => p.pos.first().map(String::as_str),
        _ => p.value(opt),
    };
    Some(name.unwrap_or("").to_string())
}

/// `git checkout`, by git's own cases (builtin/checkout.c):
/// - `-- <paths>`, `<tree-ish> -- <paths>`, `<a> <b>...`, `-p`,
///   `--pathspec-from-file`: restore paths (snapshot), never a switch;
/// - `<x>` alone or `<x> --`: a switch, or DWIM-create from a remote
///   branch; allowed only when `<x>` is the bound branch (or `<x>` is `.`,
///   `./…`, `../…`, which cannot name a ref: a path restore).
pub(crate) fn checkout(
    p: &Parsed,
    g: &Guard,
    binding: &Binding,
) -> Result<Option<&'static str>, Refusal> {
    if let Some(name) = created(p) {
        return Err(refuse_create(&name, binding));
    }
    if p.has("--detach") {
        return Err(refuse_switch("a detached HEAD", binding, g.protected));
    }
    let force = p.any(&["--force", "--merge"]);
    let restores_paths = p.any(&["--patch", "--pathspec-from-file"])
        || p.pos.len() > 1
        || (p.dashdash && !p.paths.is_empty())
        || (!p.dashdash && p.pos.len() == 1 && cannot_be_a_ref(&p.pos[0]));
    if restores_paths {
        return Ok(Some("checkout"));
    }
    match p.pos.first() {
        None => Ok(force.then_some("checkout")),
        Some(target) if is_current(target, binding) => Ok(force.then_some("checkout")),
        Some(target) => {
            let shown = if target == "-" {
                "the previous branch (-)"
            } else {
                target
            };
            let mut r = refuse_switch(shown, binding, g.protected);
            r.next = format!(
                "{}. To restore a file instead: git checkout -- <path>  or  git restore <path>",
                r.next
            );
            Err(r)
        }
    }
}

/// `.`, `./x`, `../x`: no ref name or revision can look like this (a ref
/// name component cannot start with `.`; revisions need one of `:~^@{`), so
/// git can only read it as a path.
fn cannot_be_a_ref(arg: &str) -> bool {
    (arg == "." || arg.starts_with("./") || arg.starts_with("../"))
        && !arg.contains([':', '~', '^', '@', '{'])
}

pub(crate) fn switch(
    p: &Parsed,
    g: &Guard,
    binding: &Binding,
) -> Result<Option<&'static str>, Refusal> {
    if let Some(name) = created(p) {
        return Err(refuse_create(&name, binding));
    }
    if p.has("--detach") {
        return Err(refuse_switch("a detached HEAD", binding, g.protected));
    }
    let force = p.any(&["--force", "--discard-changes", "--merge"]);
    match p.pos.first() {
        Some(t) if !is_current(t, binding) => Err(refuse_switch(t, binding, g.protected)),
        _ => Ok(force.then_some("switch")),
    }
}

// ── branch / tag ────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BranchOp {
    List,
    Upstream,
    Delete(Vec<String>),
    Rename,
    Create { name: String },
}

/// What `git branch` does (builtin/branch.c: delete, move/copy, upstream
/// edits, list when filtering or without names, else create).
pub(crate) fn branch_op(p: &Parsed) -> BranchOp {
    if p.any(&["--delete", "-D"]) {
        BranchOp::Delete(p.pos.clone())
    } else if p.any(&["--move", "-M", "--copy", "-C"]) {
        BranchOp::Rename
    } else if p.any(&[
        "--set-upstream-to",
        "--unset-upstream",
        "--edit-description",
    ]) {
        BranchOp::Upstream
    } else if p.pos.is_empty()
        || p.any(&[
            "--list",
            "--all",
            "--remotes",
            "--show-current",
            "--contains",
            "--no-contains",
            "--merged",
            "--no-merged",
            "--points-at",
            "--verbose",
        ])
    {
        BranchOp::List
    } else {
        BranchOp::Create {
            name: p.pos[0].clone(),
        }
    }
}

pub(crate) fn branch(p: &Parsed, g: &Guard, binding: &Binding) -> Result<(), Refusal> {
    match branch_op(p) {
        BranchOp::List | BranchOp::Upstream => Ok(()),
        BranchOp::Rename => Err(Refusal::new(
            "branch_rename",
            "renaming or copying branches is refused: the daemon owns branch names".to_string(),
            stay_hint(Some(binding)),
        )),
        BranchOp::Delete(names) => {
            for name in &names {
                if g.protected.names_protected(name) {
                    return Err(g.refuse_protected(name));
                }
                if is_current(name, binding) || !in_own_namespace(name, binding) {
                    return Err(Refusal::new(
                        "branch_delete",
                        format!(
                            "deleting branch {name} is refused: agents may only delete their own scratch branches under {}",
                            own_namespace(binding).unwrap_or_else(|| "agend/<task-id>/".into())
                        ),
                        "leave other branches alone; the daemon deletes task branches after merge or cancel",
                    ));
                }
            }
            Ok(())
        }
        BranchOp::Create { name } => {
            if g.protected.names_protected(&name) {
                return Err(g.refuse_protected(&name));
            }
            if !in_own_namespace(&name, binding) || is_current(&name, binding) {
                return Err(refuse_create(&name, binding));
            }
            g.check_dst(&format!("refs/heads/{}", short_branch(&name)))
        }
    }
}

pub(crate) fn tag_is_list(p: &Parsed) -> bool {
    p.pos.is_empty()
        || p.any(&[
            "--list",
            "--verify",
            "-n",
            "--contains",
            "--no-contains",
            "--merged",
            "--no-merged",
            "--points-at",
        ])
}

pub(crate) fn tag(p: &Parsed, g: &Guard) -> Result<(), Refusal> {
    let names: &[String] = if p.has("--delete") {
        &p.pos
    } else {
        &p.pos[..p.pos.len().min(1)]
    };
    for name in names {
        let full = format!("refs/tags/{}", name.trim_start_matches("refs/tags/"));
        if g.protected.is_protected(&full) {
            return Err(Refusal::new(
                "protected_ref",
                format!("writing protected tag {name} is refused: only the daemon changes it"),
                "leave protected tags alone; ask a human if one must change: agend ask \"<question>\"",
            ));
        }
    }
    Ok(())
}

// ── rebase / pull --rebase / stash ──────────────────────────────────────

fn refuse_update_refs(what: &str, next: &str) -> Refusal {
    Refusal::new(
        "rebase_update_refs",
        format!("{what} moves every branch that points into the rebased commits, not only yours"),
        next.to_string(),
    )
}

/// `git rebase [<upstream> [<branch>]]` / `--root [<branch>]`: `<branch>` is
/// checked out first, so it must be the bound branch; `--update-refs` (or
/// `rebase.updateRefs`) would move other branches.
pub(crate) fn rebase(p: &Parsed, g: &Guard, binding: &Binding) -> Result<(), Refusal> {
    const SEQUENCER: &[&str] = &[
        "--continue",
        "--skip",
        "--abort",
        "--quit",
        "--edit-todo",
        "--show-current-patch",
    ];
    if p.has("--update-refs") {
        return Err(refuse_update_refs(
            "`git rebase --update-refs`",
            "rebase only your branch: git rebase <upstream>",
        ));
    }
    if !p.any(SEQUENCER) && !p.has("--no-update-refs") && g.config_on("rebase.updateRefs") {
        return Err(refuse_update_refs(
            "`git rebase` with rebase.updateRefs=true",
            "add --no-update-refs: git rebase --no-update-refs <upstream>",
        ));
    }
    let branch = if p.has("--root") {
        p.pos.first()
    } else {
        p.pos.get(1)
    };
    match branch {
        Some(b) if !is_current(b, binding) => Err(refuse_switch(b, binding, g.protected)),
        _ => Ok(()),
    }
}

/// `git pull` rebases with `rebase.updateRefs` honoured; refuse that combo.
pub(crate) fn pull_rebase(p: &Parsed, g: &Guard) -> Result<(), Refusal> {
    if !p.has("--no-rebase") && g.config_on("rebase.updateRefs") {
        return Err(refuse_update_refs(
            "`git pull` (which may rebase) with rebase.updateRefs=true",
            "merge instead: git pull --no-rebase ...; or: git fetch, then git rebase --no-update-refs <upstream>",
        ));
    }
    Ok(())
}

/// `git stash branch <name>` creates a branch and switches to it.
pub(crate) fn stash(rest: &[String], binding: &Binding) -> Result<(), Refusal> {
    if first_positional(rest) == Some("branch") {
        let name = rest
            .iter()
            .skip_while(|a| a.as_str() != "branch")
            .nth(1)
            .map(String::as_str)
            .unwrap_or("");
        return Err(refuse_create(name, binding));
    }
    Ok(())
}

// ── config / remote ─────────────────────────────────────────────────────

const CONFIG_READ: &[&str] = &[
    "--get",
    "--get-all",
    "--get-regexp",
    "--get-urlmatch",
    "--list",
    "--get-color",
    "--get-colorbool",
    "--blob",
];
const CONFIG_WRITE: &[&str] = &[
    "--replace-all",
    "--add",
    "--unset",
    "--unset-all",
    "--rename-section",
    "--remove-section",
    "--edit",
];

pub(crate) fn config_is_read(p: &Parsed) -> bool {
    if p.any(CONFIG_WRITE) {
        return false;
    }
    if p.any(CONFIG_READ) {
        return true;
    }
    match p.pos.first().map(String::as_str) {
        Some("get" | "list") => true,
        Some("set" | "unset" | "rename-section" | "remove-section" | "edit") => false,
        _ => p.pos.len() <= 1,
    }
}

/// Config writes: only keys an agent may set (`config_keys`); section edits
/// and the editor cannot be checked.
pub(crate) fn config(p: &Parsed) -> Result<(), Refusal> {
    let sub = p.pos.first().map(String::as_str);
    let unchecked = p.any(&["--rename-section", "--remove-section", "--edit"])
        || matches!(sub, Some("rename-section" | "remove-section" | "edit"));
    let key = match sub {
        Some("set" | "unset") => p.pos.get(1),
        _ => p.pos.first(),
    };
    let next = format!(
        "agents may set only: {}; for anything else ask a human: agend ask \"<question>\"",
        config_keys::ALLOWED_HINT
    );
    if unchecked {
        return Err(Refusal::new(
            "config_write",
            "renaming, removing or editing config sections is refused: the shim cannot check which keys change".to_string(),
            next,
        ));
    }
    match key {
        Some(k) if config_keys::allowed(k) => Ok(()),
        k => Err(Refusal::new(
            "config_write",
            format!(
                "setting {} is refused: that config can redirect refs, the work tree, hooks or command names",
                k.map(String::as_str).unwrap_or("<no key>")
            ),
            next,
        )),
    }
}

const REMOTE_ADD: &[&str] = &[
    "-f|--fetch",
    "--tags",
    "--no-tags",
    "-t|--track=",
    "-m|--master=",
    "--mirror?",
];

/// `git remote add` may not mirror; `add -f` and `update` fetch with the
/// configured refmaps, which must be safe (see `refs::config_refmaps`).
pub(crate) fn remote(rest: &[String], g: &Guard) -> Result<(), Refusal> {
    let Some(at) = rest.iter().position(|a| !a.starts_with('-')) else {
        return Ok(());
    };
    match rest[at].as_str() {
        "add" => {
            let p = super::opts::parse(&[REMOTE_ADD], &rest[at + 1..])
                .map_err(|u| super::refuse_option("remote add", &u))?;
            if p.has("--mirror") {
                return Err(Refusal::new(
                    "push_scope",
                    "`git remote add --mirror` maps every ref of the remote onto yours".to_string(),
                    "add a normal remote: git remote add <name> <url>",
                ));
            }
            if p.has("--fetch") {
                super::refs::config_refmaps(g, false)?;
            }
            Ok(())
        }
        "update" => super::refs::config_refmaps(g, false),
        _ => Ok(()),
    }
}
