//! The `git` guard: gathers the inputs (snapshot, location, protected refs),
//! asks `classify`, takes a snapshot when needed, and builds the real git
//! command (routed into the bound worktree or unchanged).
//!
//! Must NOT: run the real git itself (the caller execs the returned command),
//! except for the read-only commit-ish probe and the snapshot.

use crate::audit::{self, Record};
use crate::binding::{self, Snapshot};
use crate::classify::{self, Decision, Input};
use crate::ctx::{Ctx, MAX_DEPTH, lossy};
use crate::location::{self, Anchors};
use crate::protected_ref::ProtectedRefs;
use crate::{Action, Outcome, Refusal, snapshot};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn plan(ctx: &Ctx, args: &[OsString]) -> Outcome {
    let argv = lossy(args);
    let Some(real) = ctx.find_real("git") else {
        return Outcome::fail(
            "agend-shim: cannot find the real git on PATH (the shim directory is excluded); install git or fix PATH",
        );
    };
    let record = |event: &str, code: &str, detail: Option<String>| Record {
        ts: audit::now(),
        instance: ctx.instance.clone(),
        tool: "git".into(),
        event: event.into(),
        code: (!code.is_empty()).then(|| code.to_string()),
        argv: argv.clone(),
        cwd: ctx.cwd.clone(),
        detail,
    };
    if ctx.bypass {
        audit::append(ctx.home.as_deref(), &record("bypass", "", None));
        return Outcome::exec(ctx.real_command(&real, args));
    }
    if ctx.depth >= MAX_DEPTH {
        return refuse(ctx, &argv, Refusal::shim_loop("git"), &record);
    }

    let snap = binding::load(ctx.home.as_deref(), ctx.instance.as_deref());
    let parsed = classify::parse(&argv);
    let dir = parsed.chdirs.iter().fold(ctx.cwd.clone(), |d, c| d.join(c));
    let git_dir = parsed
        .git_dir
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| ctx.git_dir.clone());
    let snap_ref = snap.as_ref().ok();
    let bound = snap_ref.and_then(|s| s.binding.as_ref());
    let anchors = Anchors {
        source_repo: snap_ref.and_then(|s| s.source_repo.as_deref()),
        worktree: bound.map(|b| b.worktree()),
        snapshot_ok: snap.is_ok(),
    };
    let location = location::locate(&dir, git_dir.as_deref(), &anchors);
    let protected = ProtectedRefs::new(snap_ref.map_or(&[][..], |s: &Snapshot| &s.protected_refs));
    let probe_dir = bound.map(|b| b.worktree().to_path_buf());
    let is_commit = |name: &str| {
        probe_dir
            .as_deref()
            .is_some_and(|wt| resolves_to_commit(&real, wt, name))
    };
    let decision = classify::classify(&Input {
        args: &parsed,
        snapshot: snap.as_ref(),
        location,
        env_retargets: ctx.git_env_retargets(),
        protected: &protected,
        dir: &dir,
        is_commit: &is_commit,
    });

    let (route, snapshot_op, note) = match decision {
        Decision::Refuse(r) => return refuse(ctx, &argv, r, &record),
        Decision::Run {
            route,
            snapshot,
            note,
        } => (route, snapshot, note),
    };
    let mut messages: Vec<String> = note.into_iter().collect();
    if let (Some(op), Some(b)) = (snapshot_op, bound) {
        let instance = ctx.instance.as_deref().unwrap_or("unknown");
        let id = format!("{}-{}", audit::now(), std::process::id());
        match snapshot::take(&real, b.worktree(), instance, &id) {
            Ok(saved) => {
                let shown = display_argv(&argv[parsed.sub_index..]);
                messages.extend(saved.report(&shown));
                audit::append(
                    ctx.home.as_deref(),
                    &record("snapshot", op, Some(saved.reference.clone())),
                );
            }
            Err(e) => {
                let r = Refusal::new(
                    "snapshot_failed",
                    format!("could not snapshot your worktree before `git {op}`: {e}"),
                    "fix the error above and retry; to run without a snapshot (audited): AGEND_SHIM_BYPASS=1 git ...",
                );
                return refuse(ctx, &argv, r, &record);
            }
        }
    }

    let final_args = match &route {
        Some(wt) => routed_args(args, wt, &parsed.retarget_indexes),
        None => args.to_vec(),
    };
    Outcome {
        messages,
        action: Action::Exec(ctx.real_command(&real, &final_args)),
    }
}

fn refuse(
    ctx: &Ctx,
    argv: &[String],
    r: Refusal,
    record: &dyn Fn(&str, &str, Option<String>) -> Record,
) -> Outcome {
    audit::append(
        ctx.home.as_deref(),
        &record("refuse", r.code, Some(r.reason.clone())),
    );
    Outcome {
        messages: r.render(&format!("git {}", display_argv(argv))),
        action: Action::Refuse(r),
    }
}

/// `-C <worktree>` + the caller's argv without its retargeting globals.
fn routed_args(args: &[OsString], worktree: &Path, drop: &[usize]) -> Vec<OsString> {
    let mut out = vec![OsString::from("-C"), worktree.as_os_str().to_os_string()];
    out.extend(
        args.iter()
            .enumerate()
            .filter(|(i, _)| !drop.contains(i))
            .map(|(_, a)| a.clone()),
    );
    out
}

/// Whether `name` names a commit in `worktree` (so `checkout <name>` would
/// switch branches rather than restore a path).
fn resolves_to_commit(git: &Path, worktree: &Path, name: &str) -> bool {
    let mut cmd = Command::new(git);
    cmd.arg("-C")
        .arg(worktree)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("{name}^{{commit}}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for var in ["GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR"] {
        cmd.env_remove(var);
    }
    cmd.status().is_ok_and(|s| s.success())
}

fn display_argv(argv: &[String]) -> String {
    argv.join(" ")
}
