//! Per-command checks for a bound agent's git writes: branch switching and
//! creation, branch/tag edits, protected-ref writes (`push`, `fetch`,
//! `update-ref`, `symbolic-ref`) and which operations destroy working-tree
//! state.
//!
//! Must NOT: allow writes to protected refs, whatever the binding.

use crate::Refusal;
use crate::binding::Binding;
use crate::protected_ref::{ProtectedRefs, refspec_dst};
use agend_core::model::BRANCH_NAMESPACE;

// ── branch switching ────────────────────────────────────────────────────

fn short_branch(name: &str) -> &str {
    name.strip_prefix("refs/heads/").unwrap_or(name)
}

fn is_current(target: &str, binding: &Binding) -> bool {
    matches!(target, "HEAD" | "@") || binding.branch() == Some(short_branch(target))
}

fn own_namespace(binding: &Binding) -> Option<String> {
    match binding {
        Binding::Work { task_id, .. } => Some(format!("{BRANCH_NAMESPACE}{task_id}/")),
        Binding::Review { .. } => None,
    }
}

fn in_own_namespace(name: &str, binding: &Binding) -> bool {
    own_namespace(binding).is_some_and(|ns| short_branch(name).starts_with(&ns))
}

fn stay_hint(binding: &Binding) -> String {
    match binding {
        Binding::Work {
            branch, worktree, ..
        } => format!(
            "stay on {branch} in {}. To read another branch without switching: git log <branch>, git show <branch>:<path>, git diff <branch>...HEAD. For other work: agend task create \"<title>\"",
            worktree.display()
        ),
        Binding::Review { head, .. } => format!(
            "this is a review of {head}; inspect with git log / git show / git diff, then run: agend review approve  or  agend review changes \"<what to fix>\""
        ),
    }
}

fn refuse_switch(target: &str, binding: &Binding, p: &ProtectedRefs) -> Refusal {
    let bound = binding.branch().unwrap_or("a detached review head");
    let protected = if p.names_protected(target) {
        format!(" ({target} is protected: only the daemon changes it)")
    } else {
        String::new()
    };
    Refusal::new(
        "branch_switch",
        format!("switching to {target} is refused: you are bound to {bound}{protected}"),
        stay_hint(binding),
    )
}

fn refuse_create(name: &str, binding: &Binding) -> Refusal {
    let reason = match own_namespace(binding) {
        Some(ns) => format!(
            "creating branch {name} is refused: agents may only create branches under {ns} (the daemon owns branch names)"
        ),
        None => format!("creating branch {name} is refused: review bindings are read-only"),
    };
    Refusal::new("branch_create", reason, stay_hint(binding))
}

pub(super) fn checkout(
    rest: &[String],
    binding: &Binding,
    p: &ProtectedRefs,
    is_commit: &dyn Fn(&str) -> bool,
) -> Result<Option<&'static str>, Refusal> {
    let mut force = false;
    let mut paths_mode = false;
    let mut positionals: Vec<&str> = Vec::new();
    let mut iter = rest.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--" => {
                paths_mode = true;
                break;
            }
            "-b" | "-B" | "--orphan" => {
                let name = iter.next().map(String::as_str).unwrap_or("");
                return Err(refuse_create(name, binding));
            }
            "--detach" => return Err(refuse_switch("a detached HEAD", binding, p)),
            "-f" | "--force" => force = true,
            "-p" | "--patch" => paths_mode = true,
            "-" => positionals.push("-"),
            s if s.starts_with("--pathspec-from-file") => paths_mode = true,
            s if s.starts_with('-') => {}
            s => positionals.push(s),
        }
    }
    let Some(&target) = positionals.first() else {
        return Ok((force || paths_mode).then_some("checkout"));
    };
    if paths_mode || positionals.len() > 1 {
        return Ok(Some("checkout")); // `<tree-ish> -- <paths>` or several paths
    }
    if target == "-" {
        return Err(refuse_switch("the previous branch (-)", binding, p));
    }
    if is_current(target, binding) {
        return Ok(force.then_some("checkout"));
    }
    if p.names_protected(target) || is_commit(target) {
        return Err(refuse_switch(target, binding, p));
    }
    Ok(Some("checkout")) // `git checkout <path>`: restores the path
}

pub(super) fn switch(
    rest: &[String],
    binding: &Binding,
    p: &ProtectedRefs,
) -> Result<Option<&'static str>, Refusal> {
    let mut force = false;
    let mut target = None;
    let mut iter = rest.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "-c" | "-C" | "--create" | "--force-create" | "--orphan" => {
                let name = iter.next().map(String::as_str).unwrap_or("");
                return Err(refuse_create(name, binding));
            }
            s if s.starts_with("--create=")
                || s.starts_with("--force-create=")
                || s.starts_with("--orphan=") =>
            {
                return Err(refuse_create(
                    s.split_once('=').map_or("", |x| x.1),
                    binding,
                ));
            }
            "-d" | "--detach" => return Err(refuse_switch("a detached HEAD", binding, p)),
            "-f" | "--force" | "--discard-changes" => force = true,
            "-" => {
                target.get_or_insert("-");
            }
            s if s.starts_with('-') => {}
            s => {
                target.get_or_insert(s);
            }
        }
    }
    match target {
        Some(t) if !is_current(t, binding) => Err(refuse_switch(t, binding, p)),
        _ => Ok(force.then_some("switch")),
    }
}

// ── branch / tag ────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BranchOp {
    List,
    Upstream,
    Delete(Vec<String>),
    Rename,
    Create { name: String },
}

pub(super) fn branch_op(rest: &[String]) -> BranchOp {
    const VALUE_FLAGS: &[&str] = &[
        "--contains",
        "--no-contains",
        "--merged",
        "--no-merged",
        "--points-at",
        "--sort",
        "--format",
    ];
    let (mut delete, mut rename, mut list, mut upstream) = (false, false, false, false);
    let mut positionals = Vec::new();
    let mut iter = rest.iter();
    while let Some(a) = iter.next() {
        let s = a.as_str();
        if VALUE_FLAGS.contains(&s) {
            list = true;
            iter.next();
        } else if s == "-u" || s == "--set-upstream-to" {
            upstream = true;
            iter.next();
        } else if let Some(long) = s.strip_prefix("--") {
            match long.split('=').next().unwrap_or("") {
                "delete" => delete = true,
                "move" | "copy" => rename = true,
                "list" | "all" | "remotes" | "show-current" | "contains" | "no-contains"
                | "merged" | "no-merged" | "points-at" => list = true,
                "set-upstream-to" | "unset-upstream" | "edit-description" => upstream = true,
                _ => {}
            }
        } else if let Some(shorts) = s.strip_prefix('-') {
            for c in shorts.chars() {
                match c {
                    'd' | 'D' => delete = true,
                    'm' | 'M' | 'c' | 'C' => rename = true,
                    'l' | 'a' | 'r' => list = true,
                    'u' => upstream = true,
                    _ => {}
                }
            }
        } else {
            positionals.push(s.to_string());
        }
    }
    if delete {
        BranchOp::Delete(positionals)
    } else if rename {
        BranchOp::Rename
    } else if upstream {
        BranchOp::Upstream
    } else if list || positionals.is_empty() {
        BranchOp::List
    } else {
        BranchOp::Create {
            name: positionals.swap_remove(0),
        }
    }
}

pub(super) fn branch(rest: &[String], binding: &Binding, p: &ProtectedRefs) -> Result<(), Refusal> {
    match branch_op(rest) {
        BranchOp::List | BranchOp::Upstream => Ok(()),
        BranchOp::Rename => Err(Refusal::new(
            "branch_rename",
            "renaming or copying branches is refused: the daemon owns branch names".to_string(),
            stay_hint(binding),
        )),
        BranchOp::Delete(names) => {
            for name in &names {
                if p.names_protected(name) {
                    return Err(refuse_protected(name, binding));
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
            if p.names_protected(&name) {
                return Err(refuse_protected(&name, binding));
            }
            if in_own_namespace(&name, binding) && !is_current(&name, binding) {
                Ok(())
            } else {
                Err(refuse_create(&name, binding))
            }
        }
    }
}

pub(super) fn tag_is_list(rest: &[String]) -> bool {
    let value_flags = [
        "-m",
        "-F",
        "-u",
        "--contains",
        "--no-contains",
        "--points-at",
        "--merged",
        "--no-merged",
    ];
    let mut positionals = 0;
    let mut skip = false;
    for a in rest {
        if skip {
            skip = false;
            continue;
        }
        match a.as_str() {
            "-l" | "--list" | "-v" | "--verify" => return true,
            s if value_flags.contains(&s) => skip = true,
            s if s.starts_with('-') => {}
            _ => positionals += 1,
        }
    }
    positionals == 0
}

pub(super) fn tag(rest: &[String], p: &ProtectedRefs) -> Result<(), Refusal> {
    let value_flags = ["-m", "-F", "-u", "--cleanup"];
    let mut skip = false;
    for a in rest {
        if skip {
            skip = false;
        } else if value_flags.contains(&a.as_str()) {
            skip = true;
        } else if !a.starts_with('-') {
            let full = format!("refs/tags/{}", a.trim_start_matches("refs/tags/"));
            if p.is_protected(&full) {
                return Err(Refusal::new(
                    "protected_ref",
                    format!("writing protected tag {a} is refused: only the daemon changes it"),
                    "leave protected tags alone; ask a human if one must change: agend ask \"<question>\"",
                ));
            }
            break; // the tag name; later positionals are the target
        }
    }
    Ok(())
}

// ── protected refs: push / fetch / update-ref / symbolic-ref ────────────

fn refuse_protected(name: &str, binding: &Binding) -> Refusal {
    let next = match binding {
        Binding::Work { branch, .. } => format!(
            "commit to your branch {branch}; when the task is done run `agend done` and the daemon merges it"
        ),
        Binding::Review { .. } => {
            "finish the review with: agend review approve  or  agend review changes \"<what to fix>\""
                .to_string()
        }
    };
    Refusal::new(
        "protected_ref",
        format!("writing {name} is refused: it is a protected ref and only the daemon changes it"),
        next,
    )
}

/// A local branch/ref write a bound agent may make: its own branch or its
/// own namespace, and nothing protected.
fn check_local_dst(dst: &str, binding: &Binding, p: &ProtectedRefs) -> Result<(), Refusal> {
    if p.names_protected(dst) {
        return Err(refuse_protected(dst, binding));
    }
    let is_branch = dst.starts_with("refs/heads/") || !dst.starts_with("refs/");
    if is_branch && !is_current(dst, binding) && !in_own_namespace(dst, binding) {
        return Err(Refusal::new(
            "ref_not_yours",
            format!(
                "writing {dst} is refused: agents only write their own branch{}",
                binding
                    .branch()
                    .map(|b| format!(" {b}"))
                    .unwrap_or_default()
            ),
            stay_hint(binding),
        ));
    }
    Ok(())
}

fn push_positionals(rest: &[String]) -> Vec<&str> {
    let value_flags = ["-o", "--push-option", "--receive-pack", "--exec", "--repo"];
    let mut out = Vec::new();
    let mut skip = false;
    for a in rest {
        if skip {
            skip = false;
        } else if value_flags.contains(&a.as_str()) {
            skip = true;
        } else if !a.starts_with('-') {
            out.push(a.as_str());
        }
    }
    out
}

pub(super) fn push(rest: &[String], binding: &Binding, p: &ProtectedRefs) -> Result<(), Refusal> {
    if matches!(binding, Binding::Review { .. }) {
        return Err(Refusal::new(
            "review_readonly",
            "pushing is refused: review bindings are read-only".to_string(),
            stay_hint(binding),
        ));
    }
    if let Some(flag) = rest
        .iter()
        .find(|a| matches!(a.as_str(), "--all" | "--branches" | "--mirror" | "--prune"))
    {
        return Err(Refusal::new(
            "push_scope",
            format!("`git push {flag}` is refused: it can write branches other than yours"),
            "push only your branch: git push origin HEAD",
        ));
    }
    let delete = rest.iter().any(|a| a == "--delete" || a == "-d");
    let positionals = push_positionals(rest);
    for spec in positionals.iter().skip(1) {
        let dst = if delete {
            Some(*spec)
        } else if *spec == "HEAD" || *spec == "@" {
            None
        } else {
            refspec_dst(spec).map(|d| if d == "HEAD" { "@" } else { d })
        };
        let Some(dst) = dst.filter(|d| *d != "@") else {
            continue;
        };
        if dst.starts_with("refs/tags/") {
            if p.is_protected(dst) {
                return Err(refuse_protected(dst, binding));
            }
            continue;
        }
        check_local_dst(dst, binding, p)?;
        if delete && is_current(dst, binding) {
            return Err(Refusal::new(
                "ref_not_yours",
                format!("deleting {dst} is refused: the daemon owns the task branch lifecycle"),
                stay_hint(binding),
            ));
        }
    }
    Ok(())
}

pub(super) fn fetch_refspecs(rest: &[String]) -> Vec<&str> {
    let value_flags = [
        "--depth",
        "--deepen",
        "--shallow-since",
        "--shallow-exclude",
        "-j",
        "--jobs",
        "--upload-pack",
        "--refmap",
        "-o",
        "--server-option",
        "--negotiation-tip",
        "--recurse-submodules-default",
        "--submodule-prefix",
    ];
    let mut out = Vec::new();
    let mut skip = false;
    for a in rest {
        if skip {
            skip = false;
        } else if value_flags.contains(&a.as_str()) {
            skip = true;
        } else if !a.starts_with('-') {
            out.push(a.as_str());
        }
    }
    out.into_iter().skip(1).collect()
}

pub(super) fn fetch(rest: &[String], binding: &Binding, p: &ProtectedRefs) -> Result<(), Refusal> {
    for spec in fetch_refspecs(rest) {
        if let Some((_, dst)) = spec.split_once(':')
            && !dst.is_empty()
            && !dst.starts_with("refs/remotes/")
        {
            check_local_dst(dst, binding, p)?;
        }
    }
    Ok(())
}

pub(super) fn update_ref(
    rest: &[String],
    binding: &Binding,
    p: &ProtectedRefs,
) -> Result<(), Refusal> {
    if rest.iter().any(|a| a == "--stdin") {
        return Err(Refusal::new(
            "update_ref_stdin",
            "`git update-ref --stdin` is refused: the shim cannot check which refs it writes"
                .to_string(),
            "run one `git update-ref <ref> <new>` per ref instead",
        ));
    }
    let no_deref = rest.iter().any(|a| a == "--no-deref");
    let mut skip = false;
    let mut name = None;
    for a in rest {
        if skip {
            skip = false;
        } else if a == "-m" {
            skip = true;
        } else if !a.starts_with('-') {
            name = Some(a.as_str());
            break;
        }
    }
    let Some(name) = name else { return Ok(()) };
    if name == "HEAD" {
        return if no_deref {
            Err(refuse_switch("a detached HEAD", binding, p))
        } else {
            Ok(())
        };
    }
    check_local_dst(name, binding, p)
}

pub(super) fn symbolic_ref_writes(rest: &[String]) -> bool {
    rest.iter().any(|a| a == "-d" || a == "--delete")
        || rest.iter().filter(|a| !a.starts_with('-')).count() >= 2
}

pub(super) fn symbolic_ref(
    rest: &[String],
    binding: &Binding,
    p: &ProtectedRefs,
) -> Result<(), Refusal> {
    let mut skip = false;
    let mut positionals = Vec::new();
    for a in rest {
        if skip {
            skip = false;
        } else if a == "-m" {
            skip = true;
        } else if !a.starts_with('-') {
            positionals.push(a.as_str());
        }
    }
    let delete = rest.iter().any(|a| a == "-d" || a == "--delete");
    match positionals.as_slice() {
        ["HEAD", ..] if delete => Err(refuse_switch("no HEAD", binding, p)),
        ["HEAD", target, ..] if !is_current(target, binding) => {
            Err(refuse_switch(target, binding, p))
        }
        _ => Ok(()),
    }
}

// ── destructive working-tree operations ─────────────────────────────────

/// `clean -f` in any spelling (`-fd`, `-xdf`, `--force`), unless a dry run.
pub(super) fn clean_is_forced(rest: &[String]) -> bool {
    let dry = rest.iter().any(|a| {
        a == "--dry-run" || (a.starts_with('-') && !a.starts_with("--") && a.contains('n'))
    });
    let force = rest.iter().any(|a| {
        a == "--force" || (a.starts_with('-') && !a.starts_with("--") && a[1..].contains('f'))
    });
    force && !dry
}

/// `restore` overwrites the working tree unless it is `--staged` only.
pub(super) fn restore_touches_worktree(rest: &[String]) -> bool {
    let staged = rest.iter().any(|a| a == "--staged" || a == "-S");
    let worktree = rest.iter().any(|a| a == "--worktree" || a == "-W");
    !staged || worktree
}
