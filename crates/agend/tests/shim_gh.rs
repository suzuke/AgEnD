//! Native argv[0] and holder PATH checks. The real-tool stand-in only logs
//! arguments in the fixture; no GitHub, credentials or network are used.
#![cfg(unix)]

#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;

use agend_core::model::Backend;
use agend_core::traits::HolderLaunch;
use agend_daemon::runtime::{HolderRuntime, shims};
use agend_shim::audit;
use agend_testkit::{block_on, tempdir::TempDir};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_agend");

fn fake_gh(root: &Path) -> PathBuf {
    let dir = root.join("real");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("gh");
    fs::write(&path, "#!/bin/sh\npwd > \"$AGEND_HOME/real-gh-cwd\"\nfor arg do printf '%s\\0' \"$arg\"; done >> \"$AGEND_HOME/real-gh-argv\"\nprintf 'fixture gh\\n'\nexit 23\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

struct Fixture {
    dir: TempDir,
    real: PathBuf,
    shim: PathBuf,
    path: OsString,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new("gh-shim").unwrap();
        let real = fake_gh(dir.path());
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let shim = bin.join("gh");
        symlink(BIN, &shim).unwrap();
        // A second link (including a hard link) must not recurse into agend.
        let loop_dir = dir.path().join("loop");
        fs::create_dir(&loop_dir).unwrap();
        fs::hard_link(BIN, loop_dir.join("gh")).unwrap();
        let path =
            std::env::join_paths([bin.as_path(), loop_dir.as_path(), real.parent().unwrap()])
                .unwrap();
        Self {
            dir,
            real,
            shim,
            path,
        }
    }
    fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("PATH", &self.path)
            .env("AGEND_HOME", self.dir.path())
            .env("AGEND_INSTANCE", "g12-gh")
            .current_dir(self.dir.path());
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command(&self.shim).args(args).output().unwrap()
    }
    fn received(&self) -> Vec<u8> {
        fs::read(self.dir.path().join("real-gh-argv")).unwrap_or_default()
    }
}

#[test]
fn forbidden_commands_never_execute_the_tool_or_publish_payloads() {
    let f = Fixture::new();
    for args in [
        vec!["pr", "merge", "42", "--auto"],
        vec![
            "-Rowner/repo",
            "pr",
            "review",
            "-ca",
            "--body",
            "private-notes",
        ],
        vec![
            "api",
            "--header",
            "Authorization: Bearer fixture-secret",
            "repos/o/r/pulls/42/merge",
        ],
        vec!["api", "repos/o/r/pulls/42/reviews", "-fevent=APPROVE"],
        vec![
            "api",
            "graphql",
            "-fquery=mutation { mergePullRequest(input: {}) { clientMutationId } }",
        ],
        vec!["api", "graphql", "--input=-"],
        vec!["auth", "token"],
    ] {
        let out = f.run(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains("agend-shim: refused"));
        assert!(f.received().is_empty(), "a refused call executed gh");
    }
    let records = audit::read(f.dir.path());
    assert_eq!(records.len(), 7);
    assert!(
        records.iter().all(|r| r.tool == "gh"
            && r.event == "refuse"
            && r.instance.as_deref() == Some("g12-gh"))
    );
    let log = fs::read_to_string(audit::log_path(f.dir.path())).unwrap();
    assert!(
        !log.contains("fixture-secret")
            && !log.contains("private-notes")
            && !log.contains("mutation")
    );
    assert!(
        !f.run(&[
            "api",
            "-HAuthorization: Bearer fixture-secret",
            "repos/o/r/merges"
        ])
        .stderr
        .windows(14)
        .any(|s| s == b"fixture-secret")
    );
}

#[test]
fn allowed_calls_preserve_arguments_working_directory_and_exit_status() {
    let f = Fixture::new();
    let args = ["pr", "review", "42", "--comment", "--body", "--approve"];
    let out = f.run(&args);
    assert_eq!(out.status.code(), Some(23));
    assert_eq!(out.stdout, b"fixture gh\n");
    assert!(out.stderr.is_empty());
    let expected: Vec<u8> = args.iter().flat_map(|a| a.bytes().chain([0])).collect();
    assert_eq!(f.received(), expected);
    assert_eq!(
        fs::read_to_string(f.dir.path().join("real-gh-cwd"))
            .unwrap()
            .trim(),
        fs::canonicalize(f.dir.path())
            .unwrap()
            .display()
            .to_string()
    );
    let raw = OsStr::from_bytes(b"notes-\xff");
    let out = f
        .command(&f.shim)
        .args([OsStr::new("issue"), OsStr::new("comment"), raw])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(23));
    assert!(f.received().ends_with(b"issue\0comment\0notes-\xff\0"));
    assert!(audit::read(f.dir.path()).is_empty());
}

#[test]
fn the_operator_tool_is_unchanged_and_explicit_bypass_is_audited() {
    let f = Fixture::new();
    let out = f
        .command(&f.real)
        .args(["pr", "merge", "42"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(23));
    assert!(audit::read(f.dir.path()).is_empty());
    let out = f
        .command(&f.shim)
        .env("AGEND_SHIM_BYPASS", "1")
        .args(["auth", "token"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(23));
    let records = audit::read(f.dir.path());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event, "bypass");
}

#[test]
fn policy_and_loop_refusals_do_not_need_an_installed_gh() {
    let f = Fixture::new();
    let only_shim = f.shim.parent().unwrap();
    let out = f
        .command(&f.shim)
        .env("PATH", only_shim)
        .args(["pr", "merge", "42"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let out = f
        .command(&f.shim)
        .env("PATH", only_shim)
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(127));
    let out = f
        .command(&f.shim)
        .env("AGEND_SHIM_DEPTH", "8")
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("PATH loop"));
    assert!(f.received().is_empty());
}

#[test]
fn every_backend_holder_uses_the_shared_gh_guard_and_cleanup_leaves_no_holders() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g12gh");
    let home = lab.home(1);
    let real = fake_gh(&home);
    shims::ensure(&home, Path::new(BIN)).unwrap();
    let runtime = HolderRuntime::new(
        &home,
        Path::new(BIN),
        vec![(
            "PATH".into(),
            format!("{}:/usr/bin:/bin", real.parent().unwrap().display()),
        )],
        Arc::new(|_| {}),
    );
    for (backend, id) in [
        (Backend::Claude, "g12-gh-c"),
        (Backend::Codex, "g12-gh-x"),
        (Backend::Opencode, "g12-gh-o"),
    ] {
        let cwd = home.join(id);
        fs::create_dir(&cwd).unwrap();
        let launch = HolderLaunch {
            instance_id: id.into(), backend, executable: "/bin/sh".into(),
            args: vec!["-c".into(), "command -v gh > located; gh pr merge 42; printf '%s' \"$?\" > refused; gh pr view 42; printf '%s' \"$?\" > allowed.tmp; mv allowed.tmp allowed; exec sleep 600".into()],
            working_directory: cwd.display().to_string(),
        };
        block_on(runtime.start(&launch)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !cwd.join("allowed").exists() {
            assert!(
                Instant::now() < deadline,
                "{id}: holder did not run the probe"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            fs::read_to_string(cwd.join("located")).unwrap().trim(),
            home.join("bin/gh").display().to_string()
        );
        assert_eq!(fs::read(cwd.join("refused")).unwrap(), b"1");
        assert_eq!(fs::read(cwd.join("allowed")).unwrap(), b"23");
    }
    let expected: Vec<u8> = ["pr", "view", "42"]
        .iter()
        .flat_map(|a| a.bytes().chain([0]))
        .collect();
    assert_eq!(
        fs::read(home.join("real-gh-argv")).unwrap(),
        expected.repeat(3)
    );
    assert_eq!(lab.stop_all_holders(), 3);
    assert!(lab.running_holders().is_empty());
    for id in ["g12-gh-c", "g12-gh-x", "g12-gh-o"] {
        runtime.detach(id);
    }
}

#[test]
fn graphql_lexical_boundaries_and_repeated_approve_flags_match_native_arguments() {
    let refused = [
        "query=mutation { # a comment\r mergePullRequest(input:{pullRequestId:\"PR_fixture\"}){clientMutationId} }",
        r#"query=mutation($body:String="""Quote: " """){ mergePullRequest(input:{pullRequestId:"PR_fixture",commitBody:$body}){clientMutationId}}"#,
        r#"query=mutation($body:String="""Escaped: \""" more"""){ mergePullRequest(input:{pullRequestId:"PR_fixture",commitBody:$body}){clientMutationId}}"#,
    ];
    for query in refused {
        let f = Fixture::new();
        let out = f.run(&["api", "graphql", "-f", query]);
        assert_eq!(out.status.code(), Some(1), "{query}: {out:?}");
        assert!(f.received().is_empty());
        assert!(out.stdout.is_empty());
    }
    let allowed = [
        vec![
            "api",
            "graphql",
            "-f",
            r#"query={repository(owner:"o",name:"""a " mergePullRequest " b"""){id}}"#,
        ],
        vec![
            "api",
            "graphql",
            "-f",
            r#"query={repository(owner:"o",name:"""Escaped: \""" mergePullRequest"""){id}}"#,
        ],
        vec![
            "pr",
            "review",
            "42",
            "--approve",
            "--approve=false",
            "-c",
            "-b",
            "notes",
        ],
        vec![
            "pr",
            "review",
            "42",
            "-a",
            "--approve=false",
            "-c",
            "-b",
            "notes",
        ],
    ];
    for args in allowed {
        let f = Fixture::new();
        let out = f.run(&args);
        assert_eq!(out.status.code(), Some(23), "{args:?}: {out:?}");
        let expected: Vec<u8> = args.iter().flat_map(|a| a.bytes().chain([0])).collect();
        assert_eq!(f.received(), expected);
        assert!(audit::read(f.dir.path()).is_empty());
    }
    let f = Fixture::new();
    assert_eq!(
        f.run(&["pr", "review", "--approve=false", "-a"])
            .status
            .code(),
        Some(1)
    );
    assert!(f.received().is_empty());
}
