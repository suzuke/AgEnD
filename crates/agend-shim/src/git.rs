//! The `git` guard: gathers the inputs (snapshot, location), asks
//! `classify`, takes a snapshot when needed, and builds the real git
//! command (routed into the bound worktree or unchanged).
//!
//! The location comes from the real git (`resolve`): one `rev-parse` with
//! the caller's global options, cwd and `GIT_*` env, so the shim never
//! re-implements repository discovery.
//!
//! Must NOT: run the real git itself (the caller execs the returned command),
//! except for read-only probes (`rev-parse`, config) and the snapshot.

use crate::audit::{self, Record};
use crate::binding;
use crate::classify::{self, Decision, GitEnv, Input, Probe};
use crate::ctx::{Ctx, MAX_DEPTH, lossy};
use crate::hook::MARKER_FILE;
use crate::location::{self, Anchors, Location, Resolved};
use crate::team::{self, Key, Remotes};
use crate::{Action, Outcome, Refusal, snapshot};
use std::cell::OnceCell;
use std::ffi::{OsStr, OsString};
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
    let snap_ref = snap.as_ref().ok();
    let bound = snap_ref.and_then(|s| s.binding.as_ref());
    let source_repo = snap_ref.and_then(|s| s.source_repo.as_deref());
    let names_git_dir = parsed.git_dir.is_some() || ctx.git_dir.is_some();
    let explicit = names_git_dir || parsed.work_tree.is_some() || ctx.git_env_names_repo();
    let anchors = RealAnchors {
        git: &real,
        worktree: bound.and_then(|b| std::fs::canonicalize(b.worktree()).ok()),
        source_repo,
        source_repo_canonical: source_repo.and_then(|p| std::fs::canonicalize(p).ok()),
        worktree_dirs: OnceCell::new(),
        team_common_dir: OnceCell::new(),
    };
    let needs_location = classify::needs_location(&parsed);
    let resolved = needs_location
        .then(|| resolve(ctx, &real, &args[..parsed.sub_index], &dir, names_git_dir))
        .flatten();
    let location = match &resolved {
        _ if !needs_location => Location::Unknown,
        None => Location::NoRepo,
        Some(_) if snap.is_err() => Location::Unknown,
        Some(r) => location::locate(r, explicit, &anchors),
    };
    let env = GitEnv {
        retargets: ctx.git_env_retargets(),
        index_file: ctx.git_index_file.as_ref().map(|p| dir.join(p)),
        config: ctx.config_env.clone(),
    };
    let probe = RealProbe {
        git: &real,
        bound: bound
            .filter(|b| b.worktree().is_dir())
            .map(|b| b.worktree().to_path_buf()),
        // In the bound worktree git already named its git dir.
        worktree_git_dir: resolved
            .as_ref()
            .filter(|_| location == Location::Worktree)
            .map(|r| r.git_dir.clone()),
        anchors: &anchors,
        acting: resolved.clone(),
        source_repo: source_repo.map(Path::to_path_buf),
        team: OnceCell::new(),
        here_remotes: OnceCell::new(),
    };
    let decision = classify::classify(&Input {
        args: &parsed,
        snapshot: snap.as_ref(),
        location,
        resolved: resolved.as_ref(),
        env: &env,
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
                let shown = argv[parsed.sub_index..].join(" ");
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
        messages: r.render(&format!("git {}", argv.join(" "))),
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

/// Asks the real git where the call acts: `rev-parse` with the caller's
/// global options (`globals`, the argv before the subcommand: `-C`,
/// `--git-dir`, `--work-tree`, `-c`, ...), its cwd and its `GIT_*` env.
/// `None` when git finds no repo. From inside `$AGEND_HOME`, a repo at or
/// above `$AGEND_HOME` (a dotfiles repo in `$HOME`) does not own the
/// agent's workspace, so discovery stops there (`GIT_CEILING_DIRECTORIES`)
/// unless the caller named a git dir.
fn resolve(
    ctx: &Ctx,
    git: &Path,
    globals: &[OsString],
    dir: &Path,
    names_git_dir: bool,
) -> Option<Resolved> {
    let mut cmd = Command::new(git);
    cmd.args(globals)
        .args(REV_PARSE)
        .args(["--show-toplevel", "--show-prefix"])
        .current_dir(&ctx.cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    for (var, value) in [
        ("GIT_DIR", &ctx.git_dir),
        ("GIT_WORK_TREE", &ctx.git_work_tree),
        ("GIT_COMMON_DIR", &ctx.git_common_dir),
        ("GIT_INDEX_FILE", &ctx.git_index_file),
    ] {
        match value {
            Some(v) => cmd.env(var, v),
            None => cmd.env_remove(var),
        };
    }
    let home = ctx
        .home
        .as_deref()
        .and_then(|h| std::fs::canonicalize(h).ok());
    let inside_home = home.as_deref().is_some_and(|h| {
        std::fs::canonicalize(dir)
            .ok()
            .is_some_and(|d| d.starts_with(h))
    });
    if let (Some(home), true, false) = (home, inside_home, names_git_dir) {
        let inherited = ctx.git_ceiling_dirs.clone().unwrap_or_default();
        let dirs = std::iter::once(home).chain(std::env::split_paths(&inherited));
        if let Ok(v) = std::env::join_paths(dirs) {
            cmd.env("GIT_CEILING_DIRECTORIES", v);
        }
    }
    let out = cmd.output().ok()?;
    Resolved::from_lines(&lines_keeping_empty(&out.stdout), dir)
}

/// `rev-parse` arguments shared by every location question (git 2.13+;
/// `--git-common-dir` may print a path relative to where git runs).
const REV_PARSE: &[&str] = &["rev-parse", "--absolute-git-dir", "--git-common-dir"];

/// Runs the real git with the caller's retargeting env removed (the
/// question is about the repo named in `args`); stdout if it succeeded.
fn run_clean(git: &Path, args: &[&OsStr]) -> Option<Vec<u8>> {
    let mut cmd = Command::new(git);
    cmd.args(args).stdin(Stdio::null()).stderr(Stdio::null());
    for var in crate::ctx::RETARGET_ENV {
        cmd.env_remove(var);
    }
    let out = cmd.output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// Output lines as paths, empty ones kept (`--show-prefix` prints an empty
/// line at the top of the work tree); only the newline ending the output
/// is dropped.
fn lines_keeping_empty(stdout: &[u8]) -> Vec<PathBuf> {
    let body = stdout.strip_suffix(b"\n").unwrap_or(stdout);
    if body.is_empty() && stdout.is_empty() {
        return Vec::new();
    }
    body.split(|b| *b == b'\n').map(path_of).collect()
}

#[cfg(unix)]
fn path_of(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_of(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).trim_end_matches('\r'))
}

/// The binding's anchors, asked of the real git once and only if needed.
struct RealAnchors<'a> {
    git: &'a Path,
    /// The bound worktree, canonical (`None` if unbound or missing).
    worktree: Option<PathBuf>,
    source_repo: Option<&'a Path>,
    source_repo_canonical: Option<PathBuf>,
    worktree_dirs: OnceCell<Option<(PathBuf, PathBuf)>>,
    team_common_dir: OnceCell<Option<PathBuf>>,
}

impl RealAnchors<'_> {
    /// `REV_PARSE` answers for the checkout at `dir`, canonical.
    fn dirs_of(&self, dir: &Path) -> Option<(PathBuf, PathBuf)> {
        let mut args: Vec<&OsStr> = vec![OsStr::new("-C"), dir.as_os_str()];
        args.extend(REV_PARSE.iter().map(OsStr::new));
        let out = lines_keeping_empty(&run_clean(self.git, &args)?);
        let canon = |p: &PathBuf| std::fs::canonicalize(dir.join(p)).ok();
        match out.as_slice() {
            [g, c] => Some((canon(g)?, canon(c)?)),
            _ => None,
        }
    }
}

impl Anchors for RealAnchors<'_> {
    fn worktree(&self) -> Option<&Path> {
        self.worktree.as_deref()
    }

    fn source_repo(&self) -> Option<&Path> {
        self.source_repo_canonical.as_deref()
    }

    fn worktree_dirs(&self) -> Option<(PathBuf, PathBuf)> {
        self.worktree_dirs
            .get_or_init(|| self.dirs_of(self.worktree.as_deref()?))
            .clone()
    }

    fn team_common_dir(&self) -> Option<PathBuf> {
        self.team_common_dir
            .get_or_init(|| self.dirs_of(self.source_repo?).map(|(_, c)| c))
            .clone()
    }
}

/// The real git behind `classify::Probe`. Revision questions go to the
/// bound worktree; team questions compare remotes (`team`).
struct RealProbe<'a> {
    git: &'a Path,
    bound: Option<PathBuf>,
    /// The bound worktree's git dir when git resolved the call there.
    worktree_git_dir: Option<PathBuf>,
    anchors: &'a RealAnchors<'a>,
    /// Git's answer for the call.
    acting: Option<Resolved>,
    source_repo: Option<PathBuf>,
    team: OnceCell<Vec<Key>>,
    here_remotes: OnceCell<Remotes>,
}

impl RealProbe<'_> {
    /// `git config --get-regexp` of the remote settings `team` reads, in the
    /// checkout at `dir` (`-C`) or the git dir `git_dir` (`--git-dir`).
    fn remotes(&self, flag: &str, path: &Path) -> Remotes {
        let args = [
            OsStr::new(flag),
            path.as_os_str(),
            OsStr::new("config"),
            OsStr::new("-z"),
            OsStr::new("--get-regexp"),
            OsStr::new(team::CONFIG_REGEX),
        ];
        let out = run_clean(self.git, &args).unwrap_or_default();
        let entries: Vec<(String, String)> = String::from_utf8_lossy(&out)
            .split('\0')
            .filter(|e| !e.is_empty())
            .map(|e| match e.split_once('\n') {
                Some((k, v)) => (k.to_string(), v.to_string()),
                None => (e.to_string(), String::new()),
            })
            .collect();
        Remotes::from_config(&entries)
    }

    fn team(&self) -> &[Key] {
        self.team.get_or_init(|| match &self.source_repo {
            Some(src) => team::team_keys(src, &self.remotes("-C", src)),
            None => Vec::new(),
        })
    }

    /// Remotes of the repo the call acts on (git resolves relative remote
    /// paths from the top of its work tree).
    fn here_remotes(&self) -> &Remotes {
        self.here_remotes.get_or_init(|| match &self.acting {
            Some(r) => self.remotes("--git-dir", &r.git_dir),
            None => Remotes::default(),
        })
    }

    fn base(&self) -> &Path {
        self.acting.as_ref().map_or(Path::new("."), Resolved::root)
    }
}

impl Probe for RealProbe<'_> {
    fn is_team_repo(&self) -> bool {
        let team = self.team();
        let own = self
            .acting
            .as_ref()
            .is_some_and(|r| team.contains(&Key::Local(r.common_dir.clone())));
        own || self
            .here_remotes()
            .keys(self.base())
            .iter()
            .any(|k| team.contains(k))
    }

    fn is_team_remote(&self, dest: &str) -> bool {
        let team = self.team();
        self.here_remotes()
            .dest_keys(dest, self.base())
            .iter()
            .any(|k| team.contains(k))
    }

    fn git(&self, args: &[&str]) -> Option<String> {
        let wt = self.bound.as_deref()?;
        let mut full: Vec<&OsStr> = vec![OsStr::new("-C"), wt.as_os_str()];
        full.extend(args.iter().map(OsStr::new));
        let out = run_clean(self.git, &full)?;
        Some(String::from_utf8_lossy(&out).trim().to_string())
    }

    fn hooks_installed(&self) -> bool {
        let git_dir = match &self.worktree_git_dir {
            Some(g) => Some(g.clone()),
            None => self.anchors.worktree_dirs().map(|(g, _)| g),
        };
        git_dir.is_some_and(|g| g.join(MARKER_FILE).is_file())
    }
}
