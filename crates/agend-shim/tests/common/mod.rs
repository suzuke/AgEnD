//! Shared fixture for the shim's integration tests: real temporary repos made
//! with `git init` / `git clone` (never a copied worktree), a bare `origin`,
//! a daemon-style worktree on `agend/t-1/fix`, and a binding snapshot written
//! with the shim's own `Snapshot` type (the producer the daemon will use).
//!
//! Every git call here is pinned to the temp dir: `-C <dir>` plus
//! `GIT_CEILING_DIRECTORIES` so a missing `.git` can never make git fall
//! through to a repo outside the fixture.
//!
//! Switch to `agend_testkit::git_fixture` once it has repo builders.

#![allow(dead_code)]

use agend_core::model::work_branch;
use agend_shim::binding::{Binding, SNAPSHOT_VERSION, Snapshot, snapshot_path};
use agend_shim::ctx::Ctx;
use agend_shim::{Action, Tool, plan};
use agend_testkit::tempdir::TempDir;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub const INSTANCE: &str = "dev-1";

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

/// Env that isolates a git child from the host: no system config, and no
/// repo discovery above the temp dir.
pub fn isolate(cmd: &mut Command) -> &mut Command {
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
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

pub fn try_git(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new("git");
    isolate(&mut cmd)
        .arg("-C")
        .arg(dir)
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
            cwd: cwd.to_path_buf(),
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
    pub fn text(&self) -> String {
        self.messages.join("\n")
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
    let args: Vec<OsString> = cmd.iter().map(OsString::from).collect();
    let outcome = plan(ctx, tool, &args);
    match outcome.action {
        Action::Exec(mut c) => {
            isolate(&mut c);
            if let Some(v) = &ctx.git_work_tree {
                c.env("GIT_WORK_TREE", v);
            }
            if let Some(v) = &ctx.git_dir {
                c.env("GIT_DIR", v);
            }
            if let Some(v) = &ctx.git_index_file {
                c.env("GIT_INDEX_FILE", v);
            }
            Ran {
                messages: outcome.messages,
                refused: None,
                output: Some(c.stdin(std::process::Stdio::null()).output().unwrap()),
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
