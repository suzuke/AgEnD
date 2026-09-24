//! Ref destinations: `push`, `fetch`/`pull`, `update-ref`, `symbolic-ref`.
//!
//! The destination of a ref write must be visible and checkable:
//! - `push` needs explicit `src:dst` refspecs. A colon-less refspec or no
//!   refspec lets git pick the destination from `remote.<name>.push`,
//!   `push.default`, `branch.<name>.merge` or `remote.<name>.mirror`.
//! - `fetch`/`pull` destinations come from command-line refspecs, `--refmap`
//!   and every configured `remote.*.fetch` (git applies those even when
//!   refspecs are given); each must land in `refs/remotes/`, the agent's
//!   own namespace, or a non-forced tag.
//! - A symbolic ref is followed before its destination is checked, and
//!   agents cannot create one (it would make one ref name write another).
//!
//! Must NOT: allow writes to protected refs, whatever the binding.

use super::commands::{
    Guard, in_own_namespace, is_current, own_namespace, refuse_switch, stay_hint,
};
use super::opts::Parsed;
use crate::Refusal;
use crate::binding::Binding;
use crate::protected_ref::refspec_dst;

impl Guard<'_> {
    pub(crate) fn refuse_protected(&self, name: &str) -> Refusal {
        let next = match self.binding {
            Some(Binding::Work { branch, .. }) => format!(
                "commit to your branch {branch}; when the task is done run `agend done` and the daemon merges it"
            ),
            Some(Binding::Review { .. }) => {
                "finish the review with: agend review approve  or  agend review changes \"<what to fix>\""
                    .to_string()
            }
            None => "run `agend status` to see your assignment".to_string(),
        };
        Refusal::new(
            "protected_ref",
            format!(
                "writing {name} is refused: it is a protected ref and only the daemon changes it"
            ),
            next,
        )
    }

    fn refuse_not_yours(&self, dst: &str) -> Refusal {
        Refusal::new(
            "ref_not_yours",
            format!(
                "writing {dst} is refused: agents only write their own branch{}",
                self.binding
                    .and_then(Binding::branch)
                    .map(|b| format!(" {b}"))
                    .unwrap_or_default()
            ),
            stay_hint(self.binding),
        )
    }

    /// A ref name (as typed) a bound agent may write: not protected, and a
    /// branch only if it is the bound one or in the agent's namespace.
    fn check_name(&self, name: &str) -> Result<(), Refusal> {
        if self.protected.names_protected(name) {
            return Err(self.refuse_protected(name));
        }
        let is_branch = name.starts_with("refs/heads/") || !name.starts_with("refs/");
        let yours = self
            .binding
            .is_some_and(|b| is_current(name, b) || in_own_namespace(name, b));
        if is_branch && !yours {
            return Err(self.refuse_not_yours(name));
        }
        Ok(())
    }

    /// A local ref write to `dst`: never HEAD itself (checked by callers
    /// that allow it), the name must pass `check_name`, and if it is a
    /// symbolic ref, so must the ref it points to.
    pub(crate) fn check_dst(&self, dst: &str) -> Result<(), Refusal> {
        let dst = dst.trim_start_matches('+');
        if matches!(dst, "HEAD" | "@") {
            return Err(Refusal::new(
                "ref_not_yours",
                format!("writing {dst} as a destination is refused: name the branch explicitly"),
                stay_hint(self.binding),
            ));
        }
        self.check_name(dst)?;
        self.check_symref(dst)
    }

    fn check_symref(&self, dst: &str) -> Result<(), Refusal> {
        let full = if dst.starts_with("refs/") {
            dst.to_string()
        } else {
            format!("refs/heads/{dst}")
        };
        let Some(target) = self.probe.symref_target(&full) else {
            return Ok(());
        };
        self.check_name(&target).map_err(|r| {
            Refusal::new(
                "symref",
                format!(
                    "{dst} is a symbolic ref to {target}, so writing it writes {target}: {}",
                    r.reason
                ),
                r.next,
            )
        })
    }

    /// A fetch destination: `refs/remotes/`, `refs/prefetch/`, a tag
    /// (not forced, not a glob, not protected), or a branch `check_dst`
    /// allows (globs only inside the agent's namespace).
    fn check_fetch_dst(&self, dst: &str, forced: bool) -> Result<(), Refusal> {
        if dst.is_empty() || dst == "FETCH_HEAD" {
            return Ok(());
        }
        let glob = dst.contains('*');
        if dst.starts_with("refs/remotes/") || dst.starts_with("refs/prefetch/") {
            return if glob { Ok(()) } else { self.check_symref(dst) };
        }
        if let Some(tag) = dst.strip_prefix("refs/tags/") {
            if glob || forced {
                return Err(refuse_tag_scope(dst, glob));
            }
            if self.protected.is_protected(dst) {
                return Err(self.refuse_protected(tag));
            }
            return Ok(());
        }
        if glob {
            let own = self
                .binding
                .and_then(own_namespace)
                .is_some_and(|ns| dst.trim_start_matches("refs/heads/").starts_with(&ns));
            return if own {
                Ok(())
            } else {
                Err(self.refuse_not_yours(dst))
            };
        }
        self.check_dst(dst)
    }
}

fn refuse_tag_scope(dst: &str, glob: bool) -> Refusal {
    Refusal::new(
        "fetch_scope",
        format!(
            "fetching into {dst} {} can overwrite or prune protected tags",
            if glob { "(a tag glob)" } else { "with force" }
        ),
        "fetch tags without force: git fetch --tags <remote>",
    )
}

// ── push ────────────────────────────────────────────────────────────────

fn refuse_implicit(remote: &str, binding: &Binding) -> Refusal {
    let branch = binding.branch().unwrap_or("<your-branch>");
    Refusal::new(
        "push_explicit",
        "this push has no explicit destination: git would take it from config (remote.<name>.push, push.default, branch.<name>.merge), which the shim cannot check".to_string(),
        format!("name source and destination: git push {remote} HEAD:refs/heads/{branch}"),
    )
}

pub(crate) fn push(p: &Parsed, g: &Guard, binding: &Binding) -> Result<(), Refusal> {
    if matches!(binding, Binding::Review { .. }) {
        return Err(Refusal::new(
            "review_readonly",
            "pushing is refused: review bindings are read-only".to_string(),
            stay_hint(Some(binding)),
        ));
    }
    let branch = binding.branch().unwrap_or("<your-branch>");
    if let Some(flag) = p.first_of(&[
        "--all",
        "--branches",
        "--mirror",
        "--prune",
        "--tags",
        "--follow-tags",
    ]) {
        return Err(Refusal::new(
            "push_scope",
            format!("`git push {flag}` is refused: it can write refs other than yours"),
            format!("push only your branch: git push origin HEAD:refs/heads/{branch}"),
        ));
    }
    if let Some(flag) = p.first_of(&["--repo", "--receive-pack", "--exec"]) {
        return Err(Refusal::new(
            "push_scope",
            format!("`git push {flag}` is refused: it changes where or how git pushes"),
            format!("push only your branch: git push origin HEAD:refs/heads/{branch}"),
        ));
    }
    let Some((remote, specs)) = p.pos.split_first() else {
        return Err(refuse_implicit("origin", binding));
    };
    if specs.is_empty() {
        return Err(refuse_implicit(remote, binding));
    }
    let delete = p.has("--delete");
    for spec in specs {
        let (dst, deleting) = if delete {
            (spec.as_str(), true)
        } else {
            match spec.trim_start_matches('+').split_once(':') {
                Some((src, dst)) if !dst.is_empty() => (dst, src.is_empty()),
                _ => return Err(refuse_implicit(remote, binding)),
            }
        };
        if dst.starts_with("refs/tags/") {
            if dst.contains('*') {
                return Err(g.refuse_not_yours(dst));
            }
            if g.protected.is_protected(dst) {
                return Err(g.refuse_protected(dst));
            }
            continue;
        }
        if dst.contains('*') {
            let own = own_namespace(binding)
                .is_some_and(|ns| dst.trim_start_matches("refs/heads/").starts_with(&ns));
            if !own {
                return Err(g.refuse_not_yours(dst));
            }
            continue;
        }
        g.check_dst(dst)?;
        if deleting && is_current(dst, binding) {
            return Err(Refusal::new(
                "ref_not_yours",
                format!("deleting {dst} is refused: the daemon owns the task branch lifecycle"),
                stay_hint(Some(binding)),
            ));
        }
    }
    Ok(())
}

// ── fetch / pull ────────────────────────────────────────────────────────

pub(crate) fn refuse_stdin(sub: &str) -> Refusal {
    Refusal::new(
        "stdin_refs",
        format!(
            "`git {sub}` reading ref updates from stdin is refused: the shim cannot check which refs it writes"
        ),
        "name the refs on the command line instead",
    )
}

/// Every configured `remote.*.fetch` refspec must be a safe refmap; git
/// applies them on `fetch <remote>`, `fetch --all`, `pull`, `remote update`,
/// and (as the refmap) even when refspecs are given.
pub(crate) fn config_refmaps(g: &Guard, force: bool) -> Result<(), Refusal> {
    for (key, value) in g.probe.config(r"^remote\..*\.fetch$") {
        let spec = value.trim();
        if spec.starts_with('^') {
            continue;
        }
        let forced = force || spec.starts_with('+');
        let dst = refspec_dst(spec).filter(|_| spec.contains(':'));
        if let Some(dst) = dst {
            g.check_fetch_dst(dst, forced).map_err(|r| {
                Refusal::new(
                    "fetch_refmap",
                    format!("config {key} = {spec} maps fetched refs onto local refs: {}", r.reason),
                    "ask a human to fix that remote's fetch refspec: agend ask \"remote fetch refspec writes local branches\"",
                )
            })?;
        }
    }
    Ok(())
}

/// `git fetch` / `git pull`: every destination they can write.
pub(crate) fn fetch(sub: &str, p: &Parsed, g: &Guard) -> Result<(), Refusal> {
    if p.has("--stdin") {
        return Err(refuse_stdin(sub));
    }
    if let Some(flag) = p.first_of(&["--update-head-ok", "--prune-tags", "--upload-pack"]) {
        return Err(Refusal::new(
            "fetch_scope",
            format!(
                "`git {sub} {flag}` is refused: it can overwrite or delete refs other than remote-tracking ones"
            ),
            format!("git {sub} <remote> without {flag}"),
        ));
    }
    let force = p.has("--force");
    if force && p.has("--tags") {
        return Err(refuse_tag_scope("refs/tags/*", true));
    }
    // `fetch --multiple a b` / `--all`: positionals are remotes only.
    let refspecs: &[String] = if p.any(&["--multiple", "--all"]) {
        &[]
    } else {
        p.pos.get(1..).unwrap_or_default()
    };
    let mut i = 0;
    while i < refspecs.len() {
        let spec = refspecs[i].as_str();
        i += 1;
        if spec == "tag" {
            // `fetch <remote> tag <name>` = refs/tags/<name>:refs/tags/<name>
            if let Some(name) = refspecs.get(i) {
                g.check_fetch_dst(&format!("refs/tags/{name}"), force)?;
                i += 1;
            }
            continue;
        }
        if let Some((_, dst)) = spec.trim_start_matches('+').split_once(':') {
            g.check_fetch_dst(dst, force || spec.starts_with('+'))?;
        }
    }
    for refmap in p.values("--refmap") {
        if let Some((_, dst)) = refmap.trim_start_matches('+').split_once(':') {
            g.check_fetch_dst(dst, force || refmap.starts_with('+'))?;
        }
    }
    config_refmaps(g, force)
}

// ── update-ref / symbolic-ref ───────────────────────────────────────────

pub(crate) fn update_ref(p: &Parsed, g: &Guard, binding: &Binding) -> Result<(), Refusal> {
    if p.has("--stdin") {
        return Err(Refusal::new(
            "update_ref_stdin",
            "`git update-ref --stdin` is refused: the shim cannot check which refs it writes"
                .to_string(),
            "run one `git update-ref <ref> <new>` per ref instead",
        ));
    }
    let Some(name) = p.pos.first() else {
        return Ok(());
    };
    let no_deref = p.has("--no-deref");
    if name == "HEAD" {
        return if no_deref {
            Err(refuse_switch("a detached HEAD", binding, g.protected))
        } else {
            Ok(())
        };
    }
    if no_deref {
        g.check_name(name)
    } else {
        g.check_dst(name)
    }
}

pub(crate) fn symbolic_ref_writes(p: &Parsed) -> bool {
    p.has("--delete") || p.pos.len() >= 2
}

fn refuse_symref(name: &str, target: &str, binding: &Binding) -> Refusal {
    Refusal::new(
        "symref",
        format!(
            "making {name} a symbolic ref to {target} is refused: a symbolic ref makes one ref name write another"
        ),
        stay_hint(Some(binding)),
    )
}

/// Only `symbolic-ref HEAD <bound branch>` (a no-op) may set a symbolic ref;
/// deleting one follows the usual name rules (not protected, not another
/// agent's branch, not the bound branch).
pub(crate) fn symbolic_ref(p: &Parsed, g: &Guard, binding: &Binding) -> Result<(), Refusal> {
    if p.has("--delete") {
        for name in &p.pos {
            if name == "HEAD" {
                return Err(refuse_switch("no HEAD", binding, g.protected));
            }
            if is_current(name, binding) {
                return Err(g.refuse_not_yours(name));
            }
            g.check_name(name)?;
        }
        return Ok(());
    }
    match p.pos.as_slice() {
        [head, target, ..] if head == "HEAD" => {
            if is_current(target, binding) {
                Ok(())
            } else {
                Err(refuse_switch(target, binding, g.protected))
            }
        }
        [name, target, ..] => Err(refuse_symref(name, target, binding)),
        _ => Ok(()),
    }
}
