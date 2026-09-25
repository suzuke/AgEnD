//! Shared fixture for the shim's integration tests (they live in the `agend`
//! crate because git runs the agend hooks as the real `agend` binary): real
//! temporary repos made with `git init` / `git clone` (never a copied
//! worktree), a bare `origin`, a daemon-style worktree on `agend/t-1/fix`
//! with the agend hooks installed (`agend_shim::install_hooks`, hooks dir
//! `<home>/hooks`), and a binding snapshot written with the shim's own
//! `Snapshot` type (the producer the daemon will use).
//!
//! Every git call here is pinned to the temp dir: `-C <dir>` plus
//! `GIT_CEILING_DIRECTORIES` so a missing `.git` can never make git fall
//! through to a repo outside the fixture; hooks are only ever installed in
//! the fixture's own worktrees.
//!
//! Switch to `agend_testkit::git_fixture` once it has repo builders.

#![allow(dead_code)]

use agend_core::model::work_branch;
use agend_shim::binding::{Binding, SNAPSHOT_VERSION, Snapshot, snapshot_path};
use agend_shim::ctx::Ctx;
use agend_shim::{Action, Tool, plan};
use agend_testkit::tempdir::TempDir;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub const INSTANCE: &str = "dev-1";

/// The `agend` binary: git runs it as every agend hook (argv[0] dispatch).
pub const AGEND: &str = env!("CARGO_BIN_EXE_agend");

pub struct Fixture {
    pub dir: TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    pub repo: PathBuf,
    pub origin: PathBuf,
    pub worktree: PathBuf,
    pub workspace: PathBuf,
    pub branch: String,
}

/// Env that isolates a git child from the host: no system or global config
/// (the developer's `~/.gitconfig` must not change results), and no repo
/// discovery above the temp dir. Run from an agent shell, the child must not
/// see that agent's own binding (`AGEND_*`) either.
pub fn isolate(cmd: &mut Command) -> &mut Command {
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("AGEND_HOME")
        .env_remove("AGEND_INSTANCE")
        .env_remove("AGEND_SHIM_BYPASS")
        .env("GIT_CEILING_DIRECTORIES", ceiling())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
}

fn ceiling() -> PathBuf {
    let tmp = std::env::temp_dir();
    std::fs::canonicalize(&tmp).unwrap_or(tmp)
}

/// Runs the real git in `dir`; panics on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = try_git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The base command for the real git, isolated like every git call here.
pub fn git_base() -> Command {
    let mut cmd = Command::new("git");
    isolate(&mut cmd);
    cmd
}

/// Installs the agend hooks into `worktree` (hooks dir `<home>/hooks`).
pub fn install_hooks(home: &Path, worktree: &Path) {
    agend_shim::install_hooks(
        &git_base,
        &agend_shim::hooks_dir(home),
        Path::new(AGEND),
        worktree,
    )
    .unwrap();
}

/// The harness's own git (setup and checks), not the agent's: it acts like
/// the daemon, so it skips the agend hooks (`-c core.hooksPath=/dev/null`),
/// which would otherwise refuse git run without the agent's binding.
pub fn try_git(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new("git");
    isolate(&mut cmd)
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args);
    cmd.output().unwrap()
}

impl Fixture {
    pub fn new(label: &str) -> Fixture {
        let dir = TempDir::new(label).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let home = root.join("home");
        let repo = root.join("repo");
        let origin = root.join("origin.git");
        let workspace = home.join("workspace").join(INSTANCE);
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        git(&root, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        for b in ["feature", "master", "release", "other-feature"] {
            git(&repo, &["branch", b]);
        }
        git(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(
            &repo,
            &[
                "push",
                "-q",
                "origin",
                "main",
                "master",
                "release",
                "other-feature",
            ],
        );
        // `other-feature` exists only on the remote (checkout DWIM bait).
        git(&repo, &["branch", "-q", "-D", "other-feature"]);
        git(&repo, &["fetch", "-q", "origin"]);
        let branch = work_branch("t-1", "fix");
        let worktree = home.join("worktrees").join("t-1");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                &branch,
                worktree.to_str().unwrap(),
                "main",
            ],
        );
        install_hooks(&home, &worktree);
        let f = Fixture {
            dir,
            root,
            home,
            repo,
            origin,
            worktree,
            workspace,
            branch,
        };
        f.write_snapshot(true);
        f
    }

    pub fn snapshot(&self, bound: bool) -> Snapshot {
        Snapshot {
            version: SNAPSHOT_VERSION,
            instance: INSTANCE.into(),
            source_repo: Some(self.repo.clone()),
            protected_refs: vec!["release".into()],
            binding: bound.then(|| Binding::Work {
                task_id: "t-1".into(),
                branch: self.branch.clone(),
                worktree: self.worktree.clone(),
            }),
        }
    }

    pub fn write_snapshot(&self, bound: bool) {
        let path = snapshot_path(&self.home, INSTANCE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            serde_json::to_string_pretty(&self.snapshot(bound)).unwrap(),
        )
        .unwrap();
    }

    pub fn ctx(&self, cwd: &Path) -> Ctx {
        Ctx {
            home: Some(self.home.clone()),
            instance: Some(INSTANCE.into()),
            cwd: Some(cwd.to_path_buf()),
            path: std::env::var_os("PATH").unwrap(),
            ..Ctx::default()
        }
    }

    pub fn head(&self, dir: &Path, rev: &str) -> String {
        git(dir, &["rev-parse", rev])
    }

    /// Every protected ref, locally and on origin, with its value.
    pub fn protected_state(&self) -> String {
        let local = git(
            &self.repo,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/heads/main",
                "refs/heads/master",
                "refs/heads/release",
            ],
        );
        let remote = git(
            &self.origin,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        );
        format!("{local}\n--origin--\n{remote}")
    }
}

/// Result of one shim call: stderr lines, and the real tool's output if run.
pub struct Ran {
    pub messages: Vec<String>,
    pub refused: Option<&'static str>,
    pub output: Option<Output>,
}

impl Ran {
    /// The shim's lines, then git's stderr (where a hook's refusal is).
    pub fn text(&self) -> String {
        let stderr = self
            .output
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
            .unwrap_or_default();
        format!("{}\n{stderr}", self.messages.join("\n"))
    }

    /// git ran and failed because an agend hook refused.
    pub fn hook_refused(&self) -> bool {
        self.output.as_ref().is_some_and(|o| {
            !o.status.success()
                && String::from_utf8_lossy(&o.stderr).contains("agend-shim: refused")
        })
    }

    /// Refused by the shim or by an agend hook.
    pub fn is_refused(&self) -> bool {
        self.refused.is_some() || self.hook_refused()
    }

    pub fn ok(&self) -> &Output {
        assert_eq!(self.refused, None, "refused:\n{}", self.text());
        let out = self.output.as_ref().unwrap();
        assert!(
            out.status.success(),
            "real tool failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }
}

pub fn shim(ctx: &Ctx, tool: Tool, cmd: &[&str]) -> Ran {
    shim_input(ctx, tool, cmd, b"")
}

/// Like `shim`, with `input` on the real tool's stdin.
pub fn shim_input(ctx: &Ctx, tool: Tool, cmd: &[&str], input: &[u8]) -> Ran {
    let args: Vec<OsString> = cmd.iter().map(OsString::from).collect();
    let outcome = plan(ctx, tool, &args);
    match outcome.action {
        Action::Exec(mut c) => {
            isolate(&mut c);
            // What the holder gives the agent; git passes it to the hooks.
            if let (Some(home), Some(instance)) = (&ctx.home, &ctx.instance) {
                c.env("AGEND_HOME", home).env("AGEND_INSTANCE", instance);
            }
            for (var, value) in [
                ("GIT_WORK_TREE", &ctx.git_work_tree),
                ("GIT_DIR", &ctx.git_dir),
                ("GIT_INDEX_FILE", &ctx.git_index_file),
            ] {
                if let Some(v) = value {
                    c.env(var, v);
                }
            }
            let mut child = c
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let _ = child.stdin.take().unwrap().write_all(input);
            Ran {
                messages: outcome.messages,
                refused: None,
                output: Some(child.wait_with_output().unwrap()),
            }
        }
        Action::Refuse(r) => Ran {
            messages: outcome.messages,
            refused: Some(r.code),
            output: None,
        },
        Action::NotFound => panic!("real tool not found: {:?}", outcome.messages),
    }
}

pub fn gitshim(ctx: &Ctx, cmd: &[&str]) -> Ran {
    shim(ctx, Tool::Git, cmd)
}
