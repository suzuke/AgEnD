//! The `git` guard: gathers the inputs (snapshot, location, protected refs),
//! asks `classify`, takes a snapshot when needed, and builds the real git
//! command (routed into the bound worktree or unchanged).
//!
//! Must NOT: run the real git itself (the caller execs the returned command),
//! except for the read-only probes (symbolic refs, config) and the snapshot.

use crate::audit::{self, Record};
use crate::binding::{self, Snapshot};
use crate::classify::{self, Decision, GitEnv, Input, Probe};
use crate::ctx::{Ctx, MAX_DEPTH, lossy};
use crate::location::{self, Anchors};
use crate::protected_ref::ProtectedRefs;
use crate::team::{self, Key, Remotes};
use crate::{Action, Outcome, Refusal, snapshot};
use std::cell::OnceCell;
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
    let source_repo = snap_ref.and_then(|s| s.source_repo.as_deref());
    let anchors = Anchors {
        source_repo,
        worktree: bound.map(|b| b.worktree()),
        home: ctx.home.as_deref(),
        snapshot_ok: snap.is_ok(),
    };
    let location = location::locate(
        &dir,
        git_dir.as_deref(),
        ctx.git_common_dir.as_deref(),
        &anchors,
    );
    let protected = ProtectedRefs::new(snap_ref.map_or(&[][..], |s: &Snapshot| &s.protected_refs));
    let env = GitEnv {
        retargets: ctx.git_env_retargets(),
        work_tree: parsed
            .work_tree
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| ctx.git_work_tree.clone())
            .map(|p| dir.join(p)),
        index_file: ctx.git_index_file.as_ref().map(|p| dir.join(p)),
        config_keys: match &ctx.config_env_error {
            Some(e) => Err(e.clone()),
            None => Ok(ctx.config_env_keys.clone()),
        },
    };
    let probe = RealProbe {
        git: &real,
        bound: bound
            .filter(|b| b.worktree().is_dir())
            .map(|b| b.worktree().to_path_buf()),
        here: dir.clone(),
        source_repo: source_repo.map(Path::to_path_buf),
        team: OnceCell::new(),
        here_remotes: OnceCell::new(),
    };
    let decision = classify::classify(&Input {
        args: &parsed,
        snapshot: snap.as_ref(),
        location,
        env: &env,
        protected: &protected,
        dir: &dir,
        probe: &probe,
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

/// The real git behind `classify::Probe`. Questions about refs and config
/// go to the bound worktree when there is one (env retargeting stripped),
/// else to where git would run; team questions compare remotes (`team`).
struct RealProbe<'a> {
    git: &'a Path,
    bound: Option<PathBuf>,
    here: PathBuf,
    source_repo: Option<PathBuf>,
    team: OnceCell<Vec<Key>>,
    here_remotes: OnceCell<Remotes>,
}

impl RealProbe<'_> {
    /// stdout of `git -C <dir> <args>` if it succeeded. `clean` strips the
    /// caller's retargeting env (the question is about `dir` itself).
    fn run(&self, dir: &Path, args: &[&str], clean: bool) -> Option<String> {
        let mut cmd = Command::new(self.git);
        cmd.arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        if clean {
            for var in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_COMMON_DIR",
                "GIT_INDEX_FILE",
            ] {
                cmd.env_remove(var);
            }
        }
        let out = cmd.output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn repo(&self) -> (&Path, bool) {
        match &self.bound {
            Some(wt) => (wt, true),
            None => (&self.here, false),
        }
    }

    fn config_in(&self, dir: &Path, regex: &str, clean: bool) -> Vec<(String, String)> {
        let out = self
            .run(dir, &["config", "-z", "--get-regexp", regex], clean)
            .unwrap_or_default();
        out.split('\0')
            .filter(|e| !e.is_empty())
            .map(|e| match e.split_once('\n') {
                Some((k, v)) => (k.to_string(), v.to_string()),
                None => (e.to_string(), String::new()),
            })
            .collect()
    }

    fn team(&self) -> &[Key] {
        self.team.get_or_init(|| match &self.source_repo {
            Some(src) => team::team_keys(
                src,
                &Remotes::from_config(&self.config_in(src, team::CONFIG_REGEX, true)),
            ),
            None => Vec::new(),
        })
    }

    fn here_remotes(&self) -> &Remotes {
        self.here_remotes.get_or_init(|| {
            Remotes::from_config(&self.config_in(&self.here, team::CONFIG_REGEX, false))
        })
    }
}

impl Probe for RealProbe<'_> {
    fn symref_target(&self, full_ref: &str) -> Option<String> {
        let (dir, clean) = self.repo();
        let mut name = full_ref.to_string();
        let mut target = None;
        for _ in 0..5 {
            match self.run(dir, &["symbolic-ref", "-q", &name], clean) {
                Some(t) if !t.trim().is_empty() && t.trim() != name => {
                    name = t.trim().to_string();
                    target = Some(name.clone());
                }
                _ => break,
            }
        }
        target
    }

    fn config(&self, regex: &str) -> Vec<(String, String)> {
        let (dir, clean) = self.repo();
        self.config_in(dir, regex, clean)
    }

    fn is_team_clone(&self) -> bool {
        let team = self.team();
        self.here_remotes()
            .keys(&self.here)
            .iter()
            .any(|k| team.contains(k))
    }

    fn is_team_remote(&self, dest: &str) -> bool {
        let team = self.team();
        self.here_remotes()
            .dest_keys(dest, &self.here)
            .iter()
            .any(|k| team.contains(k))
    }
}

fn display_argv(argv: &[String]) -> String {
    argv.join(" ")
}
