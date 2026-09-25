//! Throwaway git repos for shim, forge and reconcile tests: a canonical repo
//! with an initial commit on `main`, a bare "team origin" it pushes to, and
//! linked worktrees and branches, all inside one fresh [`TempDir`].
//!
//! Every command is hygienic: an absolute `current_dir` (never the process
//! cwd), no global or system git config, a fixed author and committer (so
//! commit ids are reproducible), `GIT_CEILING_DIRECTORIES` at the fixture
//! root, and every inherited `GIT_*` / `AGEND_*` variable removed.
//!
//! Must NOT: run a command outside the repos it created (canonical, origin,
//! a linked worktree, or their subdirectories), so git's discovery always
//! starts in a fixture repo and never in a bare directory from which it
//! could walk up into an enclosing repo; accept a temp dir or label
//! containing the path-list separator (`:`, `;` on Windows), which would
//! silently void `GIT_CEILING_DIRECTORIES`; let [`GitFixture::git`] take a
//! global option (`-C`, `--git-dir`, `-c`, …) that picks another repo; or
//! let [`GitFixture::commit`] write through a symlink or a hard link or
//! into a `.git` path. A harness whose `cd` failed once ran destructive git
//! in a real repo. Not checked: path arguments after the subcommand, and
//! anything passed to [`GitFixture::command`] besides its directory.

use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use crate::tempdir::TempDir;

/// Branch the canonical repo and origin start on.
pub const MAIN: &str = "main";

const NULL_DEVICE: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// Separator git uses to split `GIT_CEILING_DIRECTORIES`.
const PATH_LIST_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

/// Variables removed by name even when the sweep below would not see them
/// (e.g. set later in the parent); the sweep removes every other inherited
/// `GIT_*` / `AGEND_*` variable.
const REMOVED: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "AGEND_HOME",
];

/// Layout under [`GitFixture::root`]: `canonical/` (the repo, `origin`
/// remote set, `main` pushed), `origin.git/` (bare), `worktrees/<name>/`.
/// Removed on drop.
pub struct GitFixture {
    _dir: TempDir,
    root: PathBuf,
    /// Linked worktrees made by [`GitFixture::add_worktree`].
    worktrees: Mutex<Vec<PathBuf>>,
}

impl GitFixture {
    /// Creates the canonical repo with one commit on [`MAIN`] and the bare
    /// origin it is pushed to. Fails with `InvalidInput`, creating nothing,
    /// if `label` contains `:` or `;` or the temp dir contains the
    /// path-list separator.
    pub fn new(label: &str) -> io::Result<GitFixture> {
        GitFixture::new_in(&std::env::temp_dir(), label)
    }

    fn new_in(parent: &Path, label: &str) -> io::Result<GitFixture> {
        // git splits GIT_CEILING_DIRECTORIES on the separator: a root that
        // contains it has no ceiling at all.
        let invalid = |what: String| Err(io::Error::new(io::ErrorKind::InvalidInput, what));
        if label.contains([':', ';']) {
            return invalid(format!("git fixture label {label:?} contains ':' or ';'"));
        }
        // Canonical: absolute, symlinks resolved (macOS /var -> /private/var),
        // so containment checks and git's own paths agree.
        let parent = std::fs::canonicalize(parent)?;
        if parent.to_string_lossy().contains(PATH_LIST_SEPARATOR) {
            return invalid(format!(
                "git fixture temp dir {parent:?} contains {PATH_LIST_SEPARATOR:?}"
            ));
        }
        let dir = TempDir::new_in(&parent, &format!("git-{label}"))?;
        let root = std::fs::canonicalize(dir.path())?;
        assert!(root.is_absolute(), "fixture root {root:?} is not absolute");
        assert!(
            !root.to_string_lossy().contains(PATH_LIST_SEPARATOR),
            "fixture root {root:?} contains {PATH_LIST_SEPARATOR:?}"
        );
        let fx = GitFixture {
            _dir: dir,
            root,
            worktrees: Mutex::new(Vec::new()),
        };
        std::fs::create_dir(fx.canonical())?;
        std::fs::create_dir(fx.origin())?;
        fx.git(&fx.origin(), &["init", "-q", "--bare", "-b", MAIN]);
        fx.git(&fx.canonical(), &["init", "-q", "-b", MAIN]);
        fx.commit(&fx.canonical(), "README", "initial commit");
        let origin = fx.origin();
        let origin = origin.to_str().expect("fixture path is UTF-8");
        fx.git(&fx.canonical(), &["remote", "add", "origin", origin]);
        fx.git(&fx.canonical(), &["push", "-q", "-u", "origin", MAIN]);
        Ok(fx)
    }

    /// The fixture's temp directory (absolute, canonical).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The canonical repo's work tree.
    pub fn canonical(&self) -> PathBuf {
        self.root.join("canonical")
    }

    /// The bare team origin.
    pub fn origin(&self) -> PathBuf {
        self.root.join("origin.git")
    }

    /// A hygienic command running `program` in `dir`. Panics unless `dir`
    /// is absolute and inside one of the fixture's repos (canonical,
    /// origin, a linked worktree, or a subdirectory of one). Only `dir` is
    /// checked, not the arguments the caller adds.
    pub fn command(&self, program: impl AsRef<OsStr>, dir: &Path) -> Command {
        self.assert_in_repo(dir, true);
        let mut cmd = Command::new(program);
        cmd.current_dir(dir);
        for (key, _) in std::env::vars_os() {
            let k = key.to_string_lossy();
            if k.starts_with("GIT_") || k.starts_with("AGEND_") {
                cmd.env_remove(&key);
            }
        }
        for key in REMOVED {
            cmd.env_remove(key);
        }
        cmd.env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CEILING_DIRECTORIES", &self.root)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "AgEnD Test")
            .env("GIT_AUTHOR_EMAIL", "test@agend.invalid")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
            .env("GIT_COMMITTER_NAME", "AgEnD Test")
            .env("GIT_COMMITTER_EMAIL", "test@agend.invalid")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
        cmd
    }

    /// Runs git in `dir` and returns its trimmed stdout. Panics (with
    /// stderr) if git fails, or if `args` starts with a global option
    /// instead of the subcommand (`-C`, `--git-dir`, `--work-tree`, `-c`,
    /// … could point git at another repo; pass the repo as `dir`).
    pub fn git(&self, dir: &Path, args: &[&str]) -> String {
        assert!(
            args.first().is_some_and(|a| !a.starts_with('-')),
            "git fixture: {args:?} must start with the subcommand, not a global option"
        );
        let output = self
            .command("git", dir)
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("git {args:?} in {dir:?}: {e}"));
        assert!(
            output.status.success(),
            "git {args:?} in {dir:?} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// Writes `message` to `file` (a plain relative path) in the work tree
    /// `dir`, commits it and returns the new head. Commit ids depend only on
    /// content, parent and message (dates are fixed), so give different
    /// commits different messages.
    /// Panics unless `dir` is in the canonical repo or a linked worktree
    /// (not the bare origin), or if any component of `file` is `.git` (any
    /// case) or a symlink, or if `file` exists with more than one hard link.
    pub fn commit(&self, dir: &Path, file: &str, message: &str) -> String {
        self.assert_in_repo(dir, false);
        let work_tree = std::fs::canonicalize(dir).expect("resolve work tree");
        let mut path = work_tree.clone();
        for c in Path::new(file).components() {
            assert!(
                matches!(c, Component::Normal(n) if !n.eq_ignore_ascii_case(".git")),
                "commit file {file:?} must be a plain relative path outside .git"
            );
            path.push(c);
            let is_link = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_symlink());
            assert!(!is_link, "commit file {file:?}: {path:?} is a symlink");
        }
        let parent = path.parent().expect("commit file has a parent");
        let parent = std::fs::canonicalize(parent)
            .unwrap_or_else(|e| panic!("commit file {file:?}: cannot resolve {parent:?}: {e}"));
        assert!(
            parent.starts_with(&work_tree),
            "commit file {file:?} resolves outside {work_tree:?}"
        );
        #[cfg(unix)]
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            use std::os::unix::fs::MetadataExt;
            assert!(
                meta.nlink() <= 1,
                "commit file {file:?}: {path:?} has {} hard links",
                meta.nlink()
            );
        }
        std::fs::write(&path, message).expect("write commit file");
        self.git(dir, &["add", "--", file]);
        self.git(dir, &["commit", "-q", "-m", message]);
        self.rev_parse(dir, "HEAD")
    }

    /// Creates branch `name` at `from` in the canonical repo.
    pub fn branch(&self, name: &str, from: &str) {
        self.git(&self.canonical(), &["branch", name, from]);
    }

    /// Adds a linked worktree `worktrees/<name>` of the canonical repo on a
    /// new branch `branch` starting at `from`, and returns its path.
    pub fn add_worktree(&self, name: &str, branch: &str, from: &str) -> PathBuf {
        let parent = self.root.join("worktrees");
        std::fs::create_dir_all(&parent).expect("create worktrees dir");
        let path = parent.join(name);
        assert!(
            path.parent() == Some(parent.as_path()),
            "worktree name {name:?} must be one path component"
        );
        let target = path.to_str().expect("fixture path is UTF-8");
        self.git(
            &self.canonical(),
            &["worktree", "add", "-q", "-b", branch, target, from],
        );
        self.worktrees
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(path.clone());
        path
    }

    /// Full commit id of `rev` in the repo at `dir`.
    pub fn rev_parse(&self, dir: &Path, rev: &str) -> String {
        self.git(
            dir,
            &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
        )
    }

    /// Whether `ancestor` is `commit` or one of its ancestors, in the
    /// canonical repo (`git merge-base --is-ancestor`).
    pub fn is_ancestor(&self, ancestor: &str, commit: &str) -> bool {
        let status = self
            .command("git", &self.canonical())
            .args(["merge-base", "--is-ancestor", ancestor, commit])
            .status()
            .expect("run git merge-base");
        match status.code() {
            Some(0) => true,
            Some(1) => false,
            _ => panic!("git merge-base --is-ancestor {ancestor} {commit}: {status}"),
        }
    }

    /// Panics unless `dir` is absolute and resolves into the canonical
    /// repo, a linked worktree, or (if `bare_ok`) the origin. Any other
    /// directory, even inside [`Self::root`], is not a repo, so git's
    /// discovery would start there and could walk up past the root.
    fn assert_in_repo(&self, dir: &Path, bare_ok: bool) {
        let resolved = std::fs::canonicalize(dir)
            .unwrap_or_else(|e| panic!("git fixture: cannot resolve {dir:?}: {e}"));
        let mut repos = vec![self.canonical()];
        if bare_ok {
            repos.push(self.origin());
        }
        repos.extend(
            self.worktrees
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned(),
        );
        assert!(
            dir.is_absolute() && repos.iter().any(|r| resolved.starts_with(r)),
            "git fixture: refusing to operate on {dir:?} (resolves to {resolved:?}), not inside a fixture repo {repos:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_canonical_origin_worktrees_and_branches() {
        let fx = GitFixture::new("unit").unwrap();
        let canonical = fx.canonical();
        let initial = fx.rev_parse(&canonical, MAIN);
        assert_eq!(fx.rev_parse(&fx.origin(), MAIN), initial);
        assert_eq!(
            fx.git(&canonical, &["log", "-1", "--format=%an <%ae> %s"]),
            "AgEnD Test <test@agend.invalid> initial commit"
        );

        let wt = fx.add_worktree("dev-1", "agend/T-1/fix", MAIN);
        assert!(wt.starts_with(fx.root()) && wt.join("README").is_file());
        assert_eq!(fx.git(&wt, &["branch", "--show-current"]), "agend/T-1/fix");
        let head = fx.commit(&wt, "fix.txt", "fix");
        assert_eq!(fx.rev_parse(&canonical, "agend/T-1/fix"), head);
        assert_eq!(fx.rev_parse(&canonical, MAIN), initial, "main moved");
        assert!(fx.is_ancestor(&initial, &head) && !fx.is_ancestor(&head, &initial));

        fx.branch("other", MAIN);
        assert_eq!(fx.rev_parse(&canonical, "other"), initial);

        let root = fx.root().to_path_buf();
        drop(fx);
        assert!(!root.exists());
    }

    #[test]
    fn refuses_directories_outside_its_temp_dir() {
        let fx = GitFixture::new("unit").unwrap();
        let outside = [
            PathBuf::from("/"),
            std::env::temp_dir(),
            fx.root().join(".."),
            fx.canonical().join("..").join(".."),
            PathBuf::from("canonical"),
        ];
        for dir in outside {
            let result = std::panic::catch_unwind(|| fx.command("git", &dir));
            assert!(result.is_err(), "command in {dir:?} was allowed");
        }
        let result = std::panic::catch_unwind(|| fx.commit(&fx.canonical(), "../x", "escape"));
        assert!(result.is_err(), "commit to ../x was allowed");
        assert!(!fx.root().join("x").exists());
    }

    #[test]
    fn commands_do_not_inherit_git_or_agend_environment() {
        let fx = GitFixture::new("unit").unwrap();
        let cmd = fx.command("git", &fx.canonical());
        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| {
                let v = v.map(|v| v.to_string_lossy().into_owned());
                (k.to_string_lossy().into_owned(), v)
            })
            .collect();
        let get = |key: &str| envs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        for key in REMOVED {
            assert_eq!(get(key), Some(None), "{key} is not removed");
        }
        for (key, _) in std::env::vars() {
            if key.starts_with("GIT_") || key.starts_with("AGEND_") {
                assert!(
                    get(&key).is_some(),
                    "inherited {key} is neither set nor removed"
                );
            }
        }
        assert_eq!(get("GIT_CONFIG_NOSYSTEM"), Some(Some("1".into())));
        assert_eq!(get("GIT_CONFIG_GLOBAL"), Some(Some(NULL_DEVICE.into())));
        assert_eq!(cmd.get_current_dir(), Some(fx.canonical().as_path()));
    }

    /// The observation behind Forge rule FRG-10 on real git: a proper merge
    /// keeps an earlier merge, moving the base to a branch head drops it.
    #[test]
    fn overwriting_the_base_drops_an_earlier_merge() {
        let fx = GitFixture::new("unit").unwrap();
        let canonical = fx.canonical();
        let a_dir = fx.add_worktree("a", "a", MAIN);
        let b_dir = fx.add_worktree("b", "b", MAIN);
        let a = fx.commit(&a_dir, "a.txt", "change a");
        let b = fx.commit(&b_dir, "b.txt", "change b");
        fx.git(
            &canonical,
            &["merge", "-q", "--no-ff", "--no-edit", "-m", "merge a", "a"],
        );
        fx.git(
            &canonical,
            &["merge", "-q", "--no-ff", "--no-edit", "-m", "merge b", "b"],
        );
        assert!(fx.is_ancestor(&a, MAIN) && fx.is_ancestor(&b, MAIN));
        fx.git(&canonical, &["update-ref", "refs/heads/main", &b]);
        assert!(!fx.is_ancestor(&a, MAIN));
    }

    fn plain_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} in {dir:?}: {status}");
    }

    /// F1: the fixture sits inside an enclosing repo; nothing it runs may
    /// reach that repo, and every directory it allows finds its own repo.
    #[test]
    fn no_command_reaches_an_enclosing_repo() {
        let outer_dir = TempDir::new("git-enclosing").unwrap();
        let outer = std::fs::canonicalize(outer_dir.path()).unwrap();
        plain_git(&outer, &["init", "-q"]);
        let fx = GitFixture::new_in(&outer, "herm").unwrap();
        let wt = fx.add_worktree("w", "w", MAIN);
        let container = fx.root().join("worktrees");
        for dir in [fx.root().to_path_buf(), container.clone()] {
            let _ =
                std::panic::catch_unwind(|| fx.git(&dir, &["config", "escaped.from", "fixture"]));
        }
        let config = std::fs::read_to_string(outer.join(".git/config")).unwrap();
        assert!(
            !config.contains("escaped"),
            "wrote the enclosing repo's config"
        );

        for dir in [fx.root().to_path_buf(), container] {
            let result = std::panic::catch_unwind(|| fx.command("git", &dir));
            assert!(result.is_err(), "command in {dir:?} was allowed");
        }
        for dir in [fx.canonical(), wt] {
            let top = fx.git(&dir, &["rev-parse", "--show-toplevel"]);
            assert!(Path::new(&top).starts_with(fx.root()), "{dir:?} -> {top}");
        }
        let git_dir = fx.git(&fx.origin(), &["rev-parse", "--absolute-git-dir"]);
        assert!(
            Path::new(&git_dir).starts_with(fx.root()),
            "origin -> {git_dir}"
        );
    }

    /// F1 (r2): git splits `GIT_CEILING_DIRECTORIES` on `:`, so a root
    /// with `:` in it had no ceiling; from `root/worktrees` git walked up
    /// into the enclosing repo. A temp dir or label with `:` is refused and
    /// nothing is created.
    #[test]
    fn refuses_a_root_or_label_with_a_path_list_separator() {
        let outer_dir = TempDir::new("git-enclosing").unwrap();
        let outer = std::fs::canonicalize(outer_dir.path()).unwrap();
        plain_git(&outer, &["init", "-q"]);
        let colon_tmp = outer.join("t:x");
        std::fs::create_dir(&colon_tmp).unwrap();
        let tin = outer.join("tin");
        std::fs::create_dir(&tin).unwrap();
        for (parent, label) in [(&colon_tmp, "colon"), (&tin, "a:b"), (&tin, "a;b")] {
            let result = std::panic::catch_unwind(|| {
                let fx = GitFixture::new_in(parent, label)?;
                let _wt = fx.add_worktree("w", "w", MAIN);
                let container = fx.root().join("worktrees");
                let _ = std::panic::catch_unwind(|| {
                    fx.git(&container, &["config", "escaped.colon", "yes"])
                });
                Ok::<_, io::Error>(())
            });
            let config = std::fs::read_to_string(outer.join(".git/config")).unwrap();
            assert!(
                !config.contains("escaped"),
                "{parent:?} + {label:?}: wrote the enclosing repo's config"
            );
            assert!(
                matches!(result, Ok(Err(_))),
                "fixture in {parent:?} with label {label:?} was created"
            );
            let left: Vec<_> = std::fs::read_dir(parent).unwrap().collect();
            assert!(left.is_empty(), "left behind {left:?}");
        }
    }

    /// F1 (r2): git only runs inside a repo the fixture made (canonical,
    /// origin, a linked worktree, or their subdirectories), so discovery
    /// never starts in a bare container such as `root/worktrees`.
    #[test]
    fn refuses_directories_outside_its_repos() {
        let fx = GitFixture::new("unit").unwrap();
        let wt = fx.add_worktree("w", "w", MAIN);
        std::fs::create_dir(fx.root().join("worktrees/stray")).unwrap();
        std::fs::create_dir(fx.root().join("other")).unwrap();
        std::fs::create_dir(wt.join("sub")).unwrap();
        for dir in ["worktrees", "worktrees/stray", "other"] {
            let dir = fx.root().join(dir);
            let result = std::panic::catch_unwind(|| fx.command("git", &dir));
            assert!(result.is_err(), "command in {dir:?} was allowed");
        }
        for dir in [fx.canonical(), fx.origin(), wt.clone(), wt.join("sub")] {
            let _ = fx.command("git", &dir);
        }
        let result = std::panic::catch_unwind(|| fx.commit(&fx.origin(), "config", "x"));
        assert!(result.is_err(), "commit into the bare origin was allowed");
    }

    /// `commit` must not overwrite git's own files: a linked worktree's
    /// `.git` gitfile could point every later command at another repo.
    #[test]
    fn commit_refuses_a_dot_git_component() {
        let fx = GitFixture::new("unit").unwrap();
        let wt = fx.add_worktree("w", "w", MAIN);
        let gitfile = std::fs::read_to_string(wt.join(".git")).unwrap();
        for (dir, file) in [
            (&wt, ".git"),
            (&wt, "sub/.git"),
            (&fx.canonical(), ".git/config"),
            (&fx.canonical(), ".GIT/config"),
        ] {
            let result = std::panic::catch_unwind(|| fx.commit(dir, file, "gitdir: /x/.git"));
            assert!(result.is_err(), "commit to {file:?} was allowed");
        }
        assert_eq!(std::fs::read_to_string(wt.join(".git")).unwrap(), gitfile);
        assert!(!wt.join("sub").exists());
    }

    /// `commit` must not write through a hard link to a file outside.
    #[cfg(unix)]
    #[test]
    fn commit_refuses_hard_links() {
        let fx = GitFixture::new("unit").unwrap();
        let outside = TempDir::new("git-outside").unwrap();
        let target = outside.path().join("hard.txt");
        std::fs::write(&target, "orig").unwrap();
        std::fs::hard_link(&target, fx.canonical().join("hl")).unwrap();
        let result = std::panic::catch_unwind(|| fx.commit(&fx.canonical(), "hl", "overwritten"));
        assert!(result.is_err(), "commit through a hard link was allowed");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "orig");
    }

    /// F2: a symlink inside the work tree must not let `commit` write
    /// outside it.
    #[cfg(unix)]
    #[test]
    fn commit_refuses_symlinks_in_the_file_path() {
        let fx = GitFixture::new("unit").unwrap();
        let outside = TempDir::new("git-outside").unwrap();
        let canonical = fx.canonical();
        std::os::unix::fs::symlink(outside.path(), canonical.join("ln")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("f.txt"), canonical.join("lf")).unwrap();
        for file in ["ln/escaped.txt", "lf"] {
            let result = std::panic::catch_unwind(|| fx.commit(&canonical, file, "escape"));
            assert!(result.is_err(), "commit to {file:?} was allowed");
        }
        let written: Vec<_> = std::fs::read_dir(outside.path()).unwrap().collect();
        assert!(written.is_empty(), "wrote outside the fixture: {written:?}");
    }

    /// F3: global options that pick another repo or work tree are refused.
    #[test]
    fn git_refuses_global_options() {
        let fx = GitFixture::new("unit").unwrap();
        let outside = TempDir::new("git-outside").unwrap();
        let out = outside.path().to_str().unwrap();
        let git_dir = format!("--git-dir={out}/.git");
        let work_tree = format!("--work-tree={out}");
        let attempts: [&[&str]; 6] = [
            &["-C", out, "init", "-q"],
            &[&git_dir, "init", "-q"],
            &["--work-tree", out, "status"],
            &[&work_tree, "status"],
            &["--namespace=x", "status"],
            &["-c", "core.bare=true", "status"],
        ];
        for args in attempts {
            let result = std::panic::catch_unwind(|| fx.git(&fx.canonical(), args));
            assert!(result.is_err(), "git {args:?} was allowed");
        }
        assert!(
            !outside.path().join(".git").exists(),
            "created a repo outside"
        );
    }
}
