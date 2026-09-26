//! The agend git hooks: git itself reports every ref a command changes.
//! - `reference-transaction` (`prepared`): refuses a protected ref, a branch
//!   outside the agent's `agend/<task>/` namespace, and deleting or renaming
//!   the bound branch (a write through a symbolic ref shows its target),
//!   and `refs/stash` (shared by every worktree; owner decision 2026-09-25).
//!   Symbolic-ref updates (HEAD moved to a branch) are not read, so
//!   `classify` refuses `checkout`/`switch` away from the bound branch.
//! - `pre-push`: refuses unless every remote ref is the bound branch.
//! - Every hook then chains to the project's hook of the same name (same
//!   arguments and stdin), so project hooks keep running. Its directory is
//!   looked up at run time: the shared `core.hooksPath` (set later, e.g. by
//!   husky, still counts), else `<common dir>/hooks`.
//!
//! Installed per agent worktree (`install_hooks`): `core.hooksPath` in its
//! own `config.worktree`, pointing at `$AGEND_HOME/hooks/` (symlinks to
//! `agend`, argv[0] dispatch). Canonical checkout and `~/.gitconfig` untouched.
//!
//! Must NOT: honour `AGEND_SHIM_BYPASS` (the hook is the hard guarantee),
//! spawn git while deciding, or allow a protected ref when the binding
//! cannot be read (fail closed).

use crate::audit::{self, Record};
use crate::binding::{self, Binding, Snapshot, SnapshotError};
use crate::ctx::{Ctx, lossy};
use crate::protected_ref::ProtectedRefs;
use crate::{REFUSED_EXIT, Refusal};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// Hook names installed (client side). Receive-side hooks, `push-to-checkout`
/// and `proc-receive` are left out: their presence changes what git does.
pub const NAMES: &str = "applypatch-msg pre-applypatch post-applypatch pre-commit pre-merge-commit \
    prepare-commit-msg commit-msg post-commit pre-rebase post-checkout post-merge pre-push \
    pre-auto-gc post-rewrite sendemail-validate post-index-change reference-transaction";

/// Empty file in the worktree's own git dir marking it as having the agend
/// hooks (the shim refuses writes without it).
pub const MARKER_FILE: &str = "agend-hooks-installed";

/// The shared hooks directory: `$AGEND_HOME/hooks`.
pub fn hooks_dir(home: &Path) -> PathBuf {
    home.join("hooks")
}

/// Hook entry point (`agend` invoked as `<hooks dir>/<name>` by git).
pub fn run(name: &'static str) -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let ctx = Ctx::from_env();
    let chained = chain_target(&ctx, name).map(|hook| {
        let mut cmd = Command::new(hook);
        cmd.args(&args);
        cmd
    });
    if !matches!(name, "reference-transaction" | "pre-push") {
        return match chained {
            Some(cmd) => crate::exec(cmd),
            None => ExitCode::SUCCESS,
        };
    }
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut input) {
        eprintln!("agend-shim: refused: the {name} hook cannot read what git sent: {e}");
        return ExitCode::from(REFUSED_EXIT);
    }
    let argv = lossy(&args);
    let snap = binding::load(ctx.home.as_deref(), ctx.instance.as_deref());
    let input_text = String::from_utf8_lossy(&input);
    if let Err((what, r)) = check(name, &argv, &input_text, snap.as_ref()) {
        for line in r.render(&format!("{what} (agend {name} hook)")) {
            eprintln!("{line}");
        }
        let record = Record {
            ts: audit::now(),
            instance: ctx.instance.clone(),
            tool: "git".into(),
            event: "refuse".into(),
            code: Some(r.code.into()),
            argv: std::iter::once(name.to_string()).chain(argv).collect(),
            cwd: ctx.cwd.clone().unwrap_or_default(),
            detail: Some(r.reason),
        };
        audit::append(ctx.home.as_deref(), &record);
        return ExitCode::from(REFUSED_EXIT);
    }
    match chained {
        Some(cmd) => run_with_input(cmd, &input),
        None => ExitCode::SUCCESS,
    }
}

/// The decision for one hook call: `Err((what was refused, why))`.
/// `input` is the hook's stdin as git wrote it.
pub fn check(
    name: &str,
    args: &[String],
    input: &str,
    snap: Result<&Snapshot, &SnapshotError>,
) -> Result<(), (String, Refusal)> {
    let prepared = args.first().is_some_and(|a| a == "prepared");
    for line in input.lines() {
        match (name, line.split(' ').collect::<Vec<_>>().as_slice()) {
            ("reference-transaction", _) if !prepared => {}
            ("reference-transaction", [_, new, refname]) => {
                let verb = if is_zero(new) { "delete" } else { "update" };
                check_update(snap, new, refname).map_err(|r| (format!("{verb} {refname}"), r))?
            }
            ("pre-push", [_, local, remote_ref, _]) => {
                let remote = args.first().map_or("", String::as_str);
                check_push(snap, local, remote_ref)
                    .map_err(|r| (format!("push to {remote_ref} on {remote}"), r))?
            }
            // A format this hook does not know: refuse rather than allow.
            _ => {
                let why = format!("the agend {name} hook cannot read git's line {line:?}");
                let next = "report it: agend ask \"agend hook input format\"";
                return Err((line.to_string(), Refusal::new("hook_input", why, next)));
            }
        }
    }
    Ok(())
}

/// All-zero object name: git's "no object" (a deletion as the new value).
fn is_zero(oid: &str) -> bool {
    !oid.is_empty() && oid.bytes().all(|b| b == b'0')
}

/// One ref update in a transaction. Every listed ref counts as written:
/// git reports a deletion with old and new both zero. HEAD itself may move
/// (a rebase detaches it); leaving the branch is refused by `classify`.
pub fn check_update(
    snap: Result<&Snapshot, &SnapshotError>,
    new: &str,
    refname: &str,
) -> Result<(), Refusal> {
    if refname == "refs/stash" {
        return Err(refuse_stash(snap.ok().and_then(|s| s.binding.as_ref())));
    }
    let snap = match snap {
        Ok(s) => s,
        Err(_) if refname.starts_with("refs/remotes/") => return Ok(()),
        Err(e) => return Err(no_binding(e)),
    };
    let binding = snap.binding.as_ref();
    if ProtectedRefs::new(&snap.protected_refs).is_protected(refname) {
        return Err(refuse_protected(refname, binding));
    }
    let Some(branch) = refname.strip_prefix("refs/heads/") else {
        return Ok(());
    };
    let ns = binding.and_then(Binding::namespace);
    if !ns.as_deref().is_some_and(|ns| branch.starts_with(ns)) {
        return Err(Refusal::new(
            "ref_not_yours",
            format!(
                "writing branch {branch} is refused: agents only write branches under {}",
                ns.as_deref()
                    .unwrap_or("their own agend/<task-id>/ (you have no work binding)")
            ),
            stay_hint(binding),
        ));
    }
    if is_zero(new) && binding.and_then(Binding::branch) == Some(branch) {
        return Err(Refusal::new(
            "ref_not_yours",
            format!(
                "deleting (or renaming) your task branch {branch} is refused: the daemon owns its lifecycle"
            ),
            stay_hint(binding),
        ));
    }
    Ok(())
}

/// One pushed ref: only the bound branch, and not its deletion.
pub fn check_push(
    snap: Result<&Snapshot, &SnapshotError>,
    local_oid: &str,
    remote_ref: &str,
) -> Result<(), Refusal> {
    let snap = snap.map_err(no_binding)?;
    let binding = snap.binding.as_ref();
    let Some(branch) = binding.and_then(Binding::branch) else {
        return Err(Refusal::new(
            "push_not_yours",
            "pushing is refused: you have no work binding (review bindings are read-only)",
            stay_hint(binding),
        ));
    };
    let next = format!("push only your branch: git push origin HEAD:refs/heads/{branch}");
    if ProtectedRefs::new(&snap.protected_refs).is_protected(remote_ref) {
        let r = refuse_protected(remote_ref, binding);
        return Err(Refusal { next, ..r });
    }
    if remote_ref != format!("refs/heads/{branch}") {
        return Err(Refusal::new(
            "push_not_yours",
            format!(
                "pushing to {remote_ref} is refused: agents push only their own branch {branch}"
            ),
            next,
        ));
    }
    if is_zero(local_oid) {
        return Err(Refusal::new(
            "push_not_yours",
            format!("deleting {remote_ref} is refused: the daemon owns the task branch lifecycle"),
            next,
        ));
    }
    Ok(())
}

fn no_binding(e: &SnapshotError) -> Refusal {
    Refusal::new(
        "no_binding",
        format!(
            "the agend hook cannot read your binding ({e}), so it refuses branch and protected ref changes"
        ),
        "run `agend status`; the daemon rewrites the binding snapshot",
    )
}

fn refuse_protected(refname: &str, binding: Option<&Binding>) -> Refusal {
    let next = match binding {
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
            "writing {refname} is refused: it is a protected ref and only the daemon changes it"
        ),
        next,
    )
}

/// Agents never write `refs/stash`: one list shared by the canonical
/// checkout and every worktree. Shared with `classify`.
pub fn refuse_stash(binding: Option<&Binding>) -> Refusal {
    let on = binding.and_then(Binding::branch);
    Refusal::new(
        "stash_shared",
        "agents do not write git stash: refs/stash is one list shared by the canonical checkout and every worktree, so stash pop / drop / clear take or destroy someone else's work",
        format!(
            "save your work as a commit on your own branch{} instead: git add -A && git commit -m \"wip: <what>\" (nothing is lost: one task, one branch). Reading still works: git stash list, git stash show",
            on.map_or(String::new(), |b| format!(" {b}"))
        ),
    )
}

/// How to stay on the binding; shared with `classify`.
pub fn stay_hint(binding: Option<&Binding>) -> String {
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

// ── chaining ────────────────────────────────────────────────────────────

/// The project's hook `name` to run after ours, if it is an executable file
/// that is not this binary: in the shared `core.hooksPath` (real git with
/// `GIT_DIR` set to the common dir, so this worktree's `config.worktree`,
/// which names the agend hooks, is not read), else `<common dir>/hooks`.
/// git exports `GIT_DIR` to hooks and runs them at the worktree top, which
/// a relative `core.hooksPath` is relative to.
fn chain_target(ctx: &Ctx, name: &str) -> Option<PathBuf> {
    let git_dir = PathBuf::from(std::env::var_os("GIT_DIR")?);
    let common = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(rel) => git_dir.join(rel.trim()),
        Err(_) => git_dir,
    };
    let configured = ctx.find_real("git").and_then(|git| {
        let mut cmd = Command::new(git);
        for var in crate::ctx::RETARGET_ENV {
            cmd.env_remove(var);
        }
        let out = cmd
            .args(["config", "--path", "--get", "core.hooksPath"])
            .env("GIT_DIR", &common)
            .stdin(Stdio::null())
            .output()
            .ok()?;
        let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !dir.is_empty()).then(|| PathBuf::from(dir))
    });
    let hook = configured
        .unwrap_or_else(|| common.join("hooks"))
        .join(name);
    let me = ctx.self_exe.as_deref().and_then(crate::ctx::identity);
    (crate::ctx::is_executable(&hook) && crate::ctx::identity(&hook) != me).then_some(hook)
}

/// Runs the chained hook with `input` on its stdin; its exit code.
fn run_with_input(mut cmd: Command, input: &[u8]) -> ExitCode {
    let mut child = match cmd.stdin(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "agend-shim: cannot run {}: {e}",
                cmd.get_program().to_string_lossy()
            );
            return ExitCode::from(REFUSED_EXIT);
        }
    };
    // A hook that exits without reading its input is fine (EPIPE).
    let _ = child.stdin.take().map(|mut stdin| stdin.write_all(input));
    match child.wait() {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(_) => ExitCode::from(REFUSED_EXIT),
    }
}

// ── install / uninstall ─────────────────────────────────────────────────

/// Installs the agend hooks into one linked agent worktree (the daemon calls
/// this when it binds a worktree; tests and the demo on lab worktrees):
/// 1. `extensions.worktreeConfig=true` in the repo config (a capability
///    switch; changes nothing by itself);
/// 2. `hooks_dir` holds one symlink per hook name to `agend`;
/// 3. the worktree's git dir gets `MARKER_FILE`;
/// 4. `core.hooksPath=<hooks_dir>` and `gc.packRefs=false` in that
///    worktree's `config.worktree`.
///
/// Refuses the main worktree (the canonical checkout). Idempotent. `git`
/// makes the base command for the real git (binary and environment).
pub fn install_hooks(
    git: &dyn Fn() -> Command,
    hooks_dir: &Path,
    agend: &Path,
    worktree: &Path,
) -> Result<(), String> {
    let git_in = |args: &[&str]| git_in(git, worktree, args);
    let (git_dir, common) = worktree_dirs(git, worktree)?;
    if git_dir == common {
        return Err(format!(
            "{} is a main checkout, not a linked agent worktree; agend hooks go only into agent worktrees",
            worktree.display()
        ));
    }
    std::fs::create_dir_all(hooks_dir).map_err(|e| format!("{}: {e}", hooks_dir.display()))?;
    for name in NAMES.split_whitespace() {
        link(agend, &hooks_dir.join(name))?;
    }
    let marker = git_dir.join(MARKER_FILE);
    std::fs::write(&marker, "").map_err(|e| format!("{}: {e}", marker.display()))?;
    git_in(&["config", "extensions.worktreeConfig", "true"])?;
    let dir = hooks_dir.to_str().ok_or("hooks dir is not UTF-8")?;
    // git 2.39's pack-refs reports every ref to the hook as if it were
    // written; refs are shared, so packing them is the canonical side's job.
    for (key, value) in [("gc.packRefs", "false"), ("core.hooksPath", dir)] {
        git_in(&["config", "--worktree", key, value])?;
    }
    Ok(())
}

/// Removes the agend hooks from one worktree (the daemon calls this when it
/// releases the binding): unsets what `install_hooks` set there. Left
/// behind, harmless: the shared hooks dir, `extensions.worktreeConfig` and
/// an empty `config.worktree`.
pub fn uninstall_hooks(git: &dyn Fn() -> Command, worktree: &Path) -> Result<(), String> {
    let (git_dir, _) = worktree_dirs(git, worktree)?;
    for key in ["core.hooksPath", "gc.packRefs"] {
        let out = git_cmd(git, worktree, &["config", "--worktree", "--unset", key])
            .output()
            .map_err(|e| e.to_string())?;
        // Exit 5: the key was not set.
        if !out.status.success() && out.status.code() != Some(5) {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
    }
    match std::fs::remove_file(git_dir.join(MARKER_FILE)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// Canonical (git dir, common dir) of the checkout at `worktree`.
fn worktree_dirs(git: &dyn Fn() -> Command, worktree: &Path) -> Result<(PathBuf, PathBuf), String> {
    let out = git_in(
        git,
        worktree,
        &["rev-parse", "--absolute-git-dir", "--git-common-dir"],
    )?;
    let canon = |p: &str| std::fs::canonicalize(worktree.join(p)).map_err(|e| format!("{p}: {e}"));
    match out.lines().collect::<Vec<_>>().as_slice() {
        [g, c] => Ok((canon(g)?, canon(c)?)),
        _ => Err(format!("{} is not a git worktree", worktree.display())),
    }
}

fn git_cmd(git: &dyn Fn() -> Command, dir: &Path, args: &[&str]) -> Command {
    let mut cmd = git();
    cmd.arg("-C").arg(dir).args(args).stdin(Stdio::null());
    for var in crate::ctx::RETARGET_ENV {
        cmd.env_remove(var);
    }
    cmd
}

/// stdout of `git -C dir args`, trimmed, if it succeeded.
fn git_in(git: &dyn Fn() -> Command, dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = git_cmd(git, dir, args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!(
            "`git {}` in {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// `dest` → `agend`, replaced atomically if it points elsewhere.
#[cfg(unix)]
fn link(agend: &Path, dest: &Path) -> Result<(), String> {
    if std::fs::read_link(dest).is_ok_and(|t| t == agend) {
        return Ok(());
    }
    let mut tmp = dest.as_os_str().to_os_string();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(agend, &tmp)
        .and_then(|_| std::fs::rename(&tmp, dest))
        .map_err(|e| format!("{}: {e}", dest.display()))
}

#[cfg(not(unix))]
fn link(_: &Path, dest: &Path) -> Result<(), String> {
    Err(format!(
        "{}: agend hooks need unix symlinks",
        dest.display()
    ))
}

#[cfg(test)]
mod tests;
