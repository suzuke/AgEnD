//! End-to-end behaviour of the git guard against real temporary repos: a
//! canonical checkout, a daemon-style worktree on `agend/t-1/fix`, and a
//! binding snapshot written with the shim's own `Snapshot` type (the same
//! producer the daemon will use).
//!
//! Local fixture helpers; switch to `agend_testkit::git_fixture` once it has
//! repo builders (see TESTING.md).

#![cfg(unix)]

use agend_core::model::work_branch;
use agend_shim::binding::{Binding, SNAPSHOT_VERSION, Snapshot, snapshot_path};
use agend_shim::ctx::Ctx;
use agend_shim::{Action, Tool, audit, plan};
use agend_testkit::tempdir::TempDir;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const INSTANCE: &str = "dev-1";

struct Fixture {
    _dir: TempDir,
    home: PathBuf,
    repo: PathBuf,
    worktree: PathBuf,
    workspace: PathBuf,
    branch: String,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

impl Fixture {
    fn new(label: &str) -> Fixture {
        let dir = TempDir::new(label).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let home = root.join("home");
        let repo = root.join("repo");
        let workspace = home.join("workspace").join(INSTANCE);
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo.join("README.md"), "hello\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        git(&repo, &["branch", "feature"]);
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
            _dir: dir,
            home,
            repo,
            worktree,
            workspace,
            branch,
        };
        f.write_snapshot(true);
        f
    }

    fn snapshot(&self, bound: bool) -> Snapshot {
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

    fn write_snapshot(&self, bound: bool) {
        let path = snapshot_path(&self.home, INSTANCE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            serde_json::to_string_pretty(&self.snapshot(bound)).unwrap(),
        )
        .unwrap();
    }

    fn ctx(&self, cwd: &Path) -> Ctx {
        Ctx {
            home: Some(self.home.clone()),
            instance: Some(INSTANCE.into()),
            cwd: cwd.to_path_buf(),
            path: std::env::var_os("PATH").unwrap(),
            ..Ctx::default()
        }
    }

    fn head(&self, dir: &Path, rev: &str) -> String {
        git(dir, &["rev-parse", rev])
    }
}

/// Result of one shim call: stderr lines, and the real git's output if run.
struct Ran {
    messages: Vec<String>,
    refused: Option<&'static str>,
    output: Option<Output>,
}

impl Ran {
    fn text(&self) -> String {
        self.messages.join("\n")
    }

    fn ok(&self) -> &Output {
        assert_eq!(self.refused, None, "refused:\n{}", self.text());
        let out = self.output.as_ref().unwrap();
        assert!(
            out.status.success(),
            "real git failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }
}

fn shim(ctx: &Ctx, tool: Tool, cmd: &[&str]) -> Ran {
    let args: Vec<OsString> = cmd.iter().map(OsString::from).collect();
    let outcome = plan(ctx, tool, &args);
    match outcome.action {
        Action::Exec(mut c) => Ran {
            messages: outcome.messages,
            refused: None,
            output: Some(c.env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap()),
        },
        Action::Refuse(r) => Ran {
            messages: outcome.messages,
            refused: Some(r.code),
            output: None,
        },
        Action::NotFound => panic!("real tool not found: {:?}", outcome.messages),
    }
}

fn gitshim(ctx: &Ctx, cmd: &[&str]) -> Ran {
    shim(ctx, Tool::Git, cmd)
}

#[test]
fn bound_commit_from_workspace_lands_on_the_task_branch() {
    let f = Fixture::new("route");
    let main_before = f.head(&f.repo, "main");
    std::fs::write(f.worktree.join("fix.txt"), "fixed\n").unwrap();
    let ctx = f.ctx(&f.workspace);
    gitshim(&ctx, &["add", "fix.txt"]).ok();
    gitshim(&ctx, &["commit", "-q", "-m", "fix"]).ok();
    assert_eq!(
        git(&f.worktree, &["log", "-1", "--format=%s", &f.branch]),
        "fix"
    );
    assert_eq!(f.head(&f.repo, "main"), main_before, "main unchanged");
    assert_eq!(git(&f.repo, &["status", "--porcelain"]), "");
    let status = gitshim(&ctx, &["status", "--short", "--branch"]);
    let out = String::from_utf8_lossy(&status.ok().stdout).to_string();
    assert!(out.contains(&f.branch), "read routed to worktree: {out}");
}

#[test]
fn bound_commit_in_canonical_is_routed_with_a_note() {
    let f = Fixture::new("canon");
    std::fs::write(f.worktree.join("a.txt"), "a\n").unwrap();
    let ctx = f.ctx(&f.repo);
    let ran = gitshim(&ctx, &["add", "a.txt"]);
    ran.ok();
    assert!(
        ran.text().contains("running in your bound worktree"),
        "{}",
        ran.text()
    );
    gitshim(&ctx, &["commit", "-q", "-m", "a"]).ok();
    let via_c = f.ctx(&f.workspace);
    std::fs::write(f.worktree.join("b.txt"), "b\n").unwrap();
    gitshim(
        &via_c,
        &["-C", f.repo.to_str().unwrap(), "commit", "-q", "-am", "x"],
    );
    assert_eq!(git(&f.repo, &["branch", "--show-current"]), "main");
    assert_eq!(git(&f.repo, &["log", "-1", "--format=%s", "main"]), "init");
    assert_eq!(git(&f.worktree, &["log", "-1", "--format=%s"]), "a");
}

#[test]
fn checkout_main_is_refused_with_next_step_and_audited() {
    let f = Fixture::new("refuse");
    let ctx = f.ctx(&f.worktree);
    let ran = gitshim(&ctx, &["checkout", "main"]);
    assert_eq!(ran.refused, Some("branch_switch"));
    let text = ran.text();
    assert!(text.contains("refused `git checkout main`"), "{text}");
    assert!(text.contains("next step:"), "{text}");
    assert!(text.contains("agend task create"), "{text}");
    assert_eq!(
        git(&f.worktree, &["branch", "--show-current"]),
        f.branch,
        "still on the task branch"
    );
    assert_eq!(
        gitshim(&ctx, &["checkout", "feature"]).refused,
        Some("branch_switch")
    );
    let records = audit::read(&f.home);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event, "refuse");
    assert_eq!(records[0].code.as_deref(), Some("branch_switch"));
    assert_eq!(records[0].argv, ["checkout", "main"]);
    assert_eq!(records[0].instance.as_deref(), Some(INSTANCE));
}

#[test]
fn worktree_and_branch_creation_are_refused() {
    let f = Fixture::new("create");
    let ctx = f.ctx(&f.worktree);
    for (cmd, code) in [
        (&["worktree", "add", "../x"][..], "worktree_managed"),
        (&["checkout", "-b", "feat/x"][..], "branch_create"),
        (&["switch", "-c", "feat/x"][..], "branch_create"),
        (&["branch", "feat/x"][..], "branch_create"),
    ] {
        assert_eq!(gitshim(&ctx, cmd).refused, Some(code), "{cmd:?}");
    }
    gitshim(&ctx, &["branch", "agend/t-1/scratch"]).ok();
    let branches = git(&f.repo, &["branch", "--format=%(refname:short)"]);
    assert!(!branches.contains("feat/x"), "{branches}");
    assert!(branches.contains("agend/t-1/scratch"), "{branches}");
    assert_eq!(git(&f.repo, &["worktree", "list"]).lines().count(), 2);
}

#[test]
fn unbound_and_missing_snapshot_refuse_mutations_but_allow_reads() {
    let f = Fixture::new("unbound");
    f.write_snapshot(false);
    let ctx = f.ctx(&f.repo);
    std::fs::write(f.repo.join("x.txt"), "x\n").unwrap();
    assert_eq!(
        gitshim(&ctx, &["add", "x.txt"]).refused,
        Some("canonical_checkout")
    );
    gitshim(&ctx, &["status", "--short"]).ok();

    std::fs::remove_file(snapshot_path(&f.home, INSTANCE)).unwrap();
    let ran = gitshim(&ctx, &["commit", "-m", "x"]);
    assert_eq!(ran.refused, Some("no_binding"));
    assert!(ran.text().contains("is missing"), "{}", ran.text());
    gitshim(&ctx, &["log", "--oneline"]).ok();

    std::fs::write(snapshot_path(&f.home, INSTANCE), "{ not json").unwrap();
    let ran = gitshim(&ctx, &["reset", "--hard"]);
    assert_eq!(ran.refused, Some("no_binding"));
    assert!(ran.text().contains("malformed"), "{}", ran.text());
    assert!(f.repo.join("x.txt").exists(), "nothing was reset");

    let no_env = Ctx {
        instance: None,
        ..f.ctx(&f.repo)
    };
    assert_eq!(
        gitshim(&no_env, &["add", "x.txt"]).refused,
        Some("no_binding")
    );
}

#[test]
fn protected_refs_are_refused_and_main_does_not_move() {
    let f = Fixture::new("protect");
    let ctx = f.ctx(&f.worktree);
    std::fs::write(f.worktree.join("w.txt"), "w\n").unwrap();
    gitshim(&ctx, &["add", "w.txt"]).ok();
    gitshim(&ctx, &["commit", "-q", "-m", "w"]).ok();
    let main_before = f.head(&f.repo, "main");
    let head = f.head(&f.worktree, "HEAD");
    for cmd in [
        vec!["update-ref", "refs/heads/main", head.as_str()],
        vec!["update-ref", "refs/heads/release", head.as_str()],
        vec!["push", ".", "HEAD:main"],
        vec!["push", ".", "HEAD:refs/heads/master"],
        vec!["branch", "-f", "main", head.as_str()],
    ] {
        assert_eq!(
            gitshim(&ctx, &cmd).refused,
            Some("protected_ref"),
            "{cmd:?}"
        );
    }
    assert_eq!(f.head(&f.repo, "main"), main_before);
    assert_eq!(
        git(
            &f.repo,
            &["for-each-ref", "--format=%(refname)", "refs/heads/release"]
        ),
        ""
    );
    // The agent's own branch may be updated.
    gitshim(
        &ctx,
        &["update-ref", &format!("refs/heads/{}", f.branch), &head],
    )
    .ok();
}

#[test]
fn reset_hard_is_snapshotted_and_restorable() {
    let f = Fixture::new("snap");
    let ctx = f.ctx(&f.workspace);
    std::fs::write(f.worktree.join("keep.txt"), "committed\n").unwrap();
    gitshim(&ctx, &["add", "keep.txt"]).ok();
    gitshim(&ctx, &["commit", "-q", "-m", "keep"]).ok();
    std::fs::write(f.worktree.join("keep.txt"), "uncommitted edit\n").unwrap();
    std::fs::write(f.worktree.join("new.txt"), "untracked\n").unwrap();

    let ran = gitshim(&ctx, &["reset", "--hard", "HEAD~1"]);
    ran.ok();
    let text = ran.text();
    assert!(text.contains("snapshot "), "{text}");
    assert!(!f.worktree.join("keep.txt").exists(), "reset removed it");

    let undo = text
        .lines()
        .find_map(|l| l.strip_prefix("agend-shim: to undo: "))
        .expect("undo line");
    for part in undo.split(" && ") {
        let words: Vec<&str> = part.split_whitespace().skip(1).collect();
        gitshim(&ctx, &words).ok();
    }
    assert_eq!(
        std::fs::read_to_string(f.worktree.join("keep.txt")).unwrap(),
        "uncommitted edit\n"
    );
    assert_eq!(
        std::fs::read_to_string(f.worktree.join("new.txt")).unwrap(),
        "untracked\n"
    );
    assert_eq!(git(&f.worktree, &["log", "-1", "--format=%s"]), "keep");
    let records = audit::read(&f.home);
    assert!(
        records
            .iter()
            .any(|r| r.event == "snapshot" && r.code.as_deref() == Some("reset")),
        "{records:?}"
    );
}

#[test]
fn clean_and_path_checkout_are_snapshotted() {
    let f = Fixture::new("clean");
    let ctx = f.ctx(&f.worktree);
    std::fs::write(f.worktree.join("scratch.txt"), "notes\n").unwrap();
    std::fs::write(f.worktree.join("README.md"), "edited\n").unwrap();
    let ran = gitshim(&ctx, &["clean", "-fd"]);
    ran.ok();
    assert!(
        ran.text().contains("before `git clean -fd`"),
        "{}",
        ran.text()
    );
    assert!(!f.worktree.join("scratch.txt").exists());
    let ran = gitshim(&ctx, &["checkout", "--", "."]);
    ran.ok();
    assert!(ran.text().contains("snapshot "), "{}", ran.text());
    assert_eq!(
        std::fs::read_to_string(f.worktree.join("README.md")).unwrap(),
        "hello\n"
    );
    let snaps = git(
        &f.repo,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/agend/snapshots/",
        ],
    );
    assert_eq!(snaps.lines().count(), 2, "{snaps}");
    let first = snaps.lines().next().unwrap();
    assert_eq!(
        git(&f.repo, &["show", &format!("{first}:scratch.txt")]),
        "notes"
    );
}

#[test]
fn bypass_runs_unchecked_and_is_audited() {
    let f = Fixture::new("bypass");
    let ctx = Ctx {
        bypass: true,
        ..f.ctx(&f.worktree)
    };
    gitshim(&ctx, &["branch", "feat/escape"]).ok();
    let records = audit::read(&f.home);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event, "bypass");
}

#[test]
fn foreign_repos_are_not_guarded() {
    let f = Fixture::new("foreign");
    let scratch = f.workspace.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let ctx = f.ctx(&scratch);
    gitshim(&ctx, &["init", "-q", "-b", "trunk"]).ok();
    gitshim(&ctx, &["checkout", "-q", "-b", "anything"]).ok();
    assert_eq!(git(&scratch, &["branch", "--show-current"]), "anything");
    assert!(audit::read(&f.home).is_empty());
}

#[test]
fn git_dir_of_the_bound_worktree_is_trusted() {
    // Hooks run git with GIT_DIR set to the worktree's git dir.
    let f = Fixture::new("gitdir");
    let gitdir = git(&f.worktree, &["rev-parse", "--absolute-git-dir"]);
    let ctx = Ctx {
        git_dir: Some(gitdir.into()),
        ..f.ctx(&f.worktree)
    };
    std::fs::write(f.worktree.join("h.txt"), "h\n").unwrap();
    gitshim(&ctx, &["add", "h.txt"]).ok();
    let canon = Ctx {
        git_dir: Some(f.repo.join(".git")),
        ..f.ctx(&f.workspace)
    };
    assert_eq!(
        gitshim(&canon, &["add", "h.txt"]).refused,
        Some("git_env_retarget")
    );
}

#[test]
fn kill_guard_protects_agend_processes() {
    let dir = TempDir::new("kill").unwrap();
    // A stand-in holder: `sleep` exec'd under the name `agend`.
    let fake = dir.path().join("agend");
    std::os::unix::fs::symlink("/bin/sleep", &fake).unwrap();
    let mut holder = Command::new(&fake).arg("60").spawn().unwrap();
    let mut other = Command::new("/bin/sleep").arg("60").spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let ctx = Ctx {
        home: Some(dir.path().to_path_buf()),
        instance: Some(INSTANCE.into()),
        cwd: dir.path().to_path_buf(),
        path: std::env::var_os("PATH").unwrap(),
        ..Ctx::default()
    };
    let hpid = holder.id().to_string();
    assert_eq!(
        shim(&ctx, Tool::Kill, &[&hpid]).refused,
        Some("kill_protected")
    );
    assert_eq!(
        shim(&ctx, Tool::Pkill, &["-f", "sleep 60"]).refused,
        Some("pattern_kill")
    );
    assert_eq!(
        shim(&ctx, Tool::Killall, &["sleep"]).refused,
        Some("pattern_kill")
    );
    assert!(holder.try_wait().unwrap().is_none(), "holder still alive");
    assert!(other.try_wait().unwrap().is_none(), "other still alive");

    let opid = other.id().to_string();
    shim(&ctx, Tool::Kill, &[&opid]).ok();
    let status = other.wait().unwrap();
    assert!(
        !status.success(),
        "explicit pid of a normal process is killed"
    );
    holder.kill().unwrap();
    holder.wait().unwrap();
    assert_eq!(audit::read(dir.path()).len(), 3);
}
