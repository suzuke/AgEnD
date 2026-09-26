//! Gate 3 walkthrough (`cargo xtask accept shim`): runs the real `agend`
//! binary under the names `git`, `kill` and `pkill` against a temporary repo
//! whose agent worktree has the agend git hooks installed (as the daemon
//! will do), and checks every outcome. Usage: `shim_demo <path to agend
//! binary>`.
//!
//! Set `AGEND_SHIM_DEMO_KEEP=1` to keep the temp dir and get the env lines
//! for trying the shim by hand.
//!
//! Safety: the shim's "real" `kill`/`pkill`/`killall` resolve to fake
//! recorders in `<tmp>/fakebin` (they only log their argv), so no signal is
//! ever delivered by the demo, even if the guard had a bug.

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    match demo::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("shim demo FAILED: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("shim demo: unix only");
}

#[cfg(unix)]
mod demo {
    use agend_core::model::work_branch;
    use agend_shim::binding::{Binding, SNAPSHOT_VERSION, Snapshot, snapshot_path};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Output};

    const INSTANCE: &str = "dev-1";

    struct Env {
        root: PathBuf,
        bin: PathBuf,
        kill_log: PathBuf,
        home: PathBuf,
        repo: PathBuf,
        origin: PathBuf,
        worktree: PathBuf,
        workspace: PathBuf,
        branch: String,
        path: OsString,
    }

    /// Real git, never the shim (the demo's own PATH has no shim dir).
    fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .map_err(|e| format!("cannot run git: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn check(ok: bool, what: &str) -> Result<(), String> {
        if ok {
            println!("   ok: {what}");
            Ok(())
        } else {
            Err(what.to_string())
        }
    }

    impl Env {
        /// Runs `tool args` through the shim, as the agent would, and prints
        /// the command and everything it wrote.
        fn shim(&self, tool: &str, args: &[&str], cwd: &Path) -> Result<Output, String> {
            let shown = cwd.strip_prefix(&self.root).unwrap_or(cwd);
            println!(
                "   $ {tool} {}      (cwd: <tmp>/{})",
                args.join(" "),
                shown.display()
            );
            let out = Command::new(self.bin.join(tool))
                .args(args)
                .current_dir(cwd)
                .env("PATH", &self.path)
                .env("AGEND_HOME", &self.home)
                .env("AGEND_INSTANCE", INSTANCE)
                .env_remove("AGEND_SHIM_BYPASS")
                .output()
                .map_err(|e| format!("cannot run shim {tool}: {e}"))?;
            for stream in [&out.stdout, &out.stderr] {
                for line in String::from_utf8_lossy(stream).lines() {
                    println!("     | {line}");
                }
            }
            println!("     exit {}", out.status.code().unwrap_or(-1));
            Ok(out)
        }

        fn stderr_line(out: &Output, prefix: &str) -> Option<String> {
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .find_map(|l| l.strip_prefix(prefix).map(str::to_string))
        }
    }

    fn setup() -> Result<Env, String> {
        let agend = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .ok_or("usage: shim_demo <path to agend binary>")?;
        let agend = std::fs::canonicalize(&agend)
            .map_err(|e| format!("agend binary {}: {e}", agend.display()))?;
        let root = std::env::temp_dir().join(format!("agend-shim-demo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let root = std::fs::canonicalize(&root).map_err(|e| e.to_string())?;
        let bin = root.join("bin");
        let home = root.join("home");
        let repo = root.join("repo");
        let workspace = home.join("workspace").join(INSTANCE);
        for d in [&bin, &repo, &workspace] {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        for tool in ["git", "kill", "pkill", "killall"] {
            std::os::unix::fs::symlink(&agend, bin.join(tool)).map_err(|e| e.to_string())?;
        }
        // Fake "real" kill tools: the shim finds these, never /bin/kill.
        let fakebin = root.join("fakebin");
        let kill_log = root.join("kill.log");
        std::fs::create_dir_all(&fakebin).map_err(|e| e.to_string())?;
        for tool in ["kill", "pkill", "killall"] {
            let p = fakebin.join(tool);
            std::fs::write(
                &p,
                format!(
                    "#!/bin/sh\nprintf '%s %s\\n' {tool} \"$*\" >> '{}'\n",
                    kill_log.display()
                ),
            )
            .map_err(|e| e.to_string())?;
            std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        let origin = root.join("origin.git");
        git(&root, &["init", "-q", "--bare", "-b", "main", "origin.git"])?;
        git(&repo, &["init", "-q", "-b", "main"])?;
        git(&repo, &["config", "user.name", "Demo"])?;
        git(&repo, &["config", "user.email", "demo@example.com"])?;
        std::fs::write(repo.join("README.md"), "hello\n").map_err(|e| e.to_string())?;
        git(&repo, &["add", "README.md"])?;
        git(&repo, &["commit", "-q", "-m", "init"])?;
        git(&repo, &["branch", "master"])?;
        let origin_s = origin.to_str().ok_or("non-UTF-8 temp dir")?;
        git(&repo, &["remote", "add", "origin", origin_s])?;
        git(&repo, &["push", "-q", "origin", "main", "master"])?;

        // What the daemon does when it assigns task t-1 (gate 10 will own
        // this): create the worktree and write the binding snapshot.
        let branch = work_branch("t-1", "fix");
        let worktree = home.join("worktrees").join("t-1");
        let wt = worktree.to_str().ok_or("non-UTF-8 temp dir")?;
        git(&repo, &["worktree", "add", "-q", "-b", &branch, wt, "main"])?;
        // The agend hooks, in the agent worktree only (the daemon does this
        // when it binds a worktree).
        agend_shim::install_hooks(
            &|| Command::new("git"),
            &agend_shim::hooks_dir(&home),
            &agend,
            &worktree,
        )?;
        let snapshot = Snapshot {
            version: SNAPSHOT_VERSION,
            instance: INSTANCE.into(),
            source_repo: Some(repo.clone()),
            protected_refs: Vec::new(),
            binding: Some(Binding::Work {
                task_id: "t-1".into(),
                branch: branch.clone(),
                worktree: worktree.clone(),
            }),
        };
        let snap_path = snapshot_path(&home, INSTANCE);
        std::fs::create_dir_all(snap_path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(
            &snap_path,
            serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;

        let mut path = OsString::from(&bin);
        path.push(":");
        path.push(&fakebin);
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());
        println!("setup: temp dir {}", root.display());
        println!("   canonical repo   <tmp>/repo (branch main), remote origin = <tmp>/origin.git");
        println!("   bound worktree   <tmp>/home/worktrees/t-1 (branch {branch})");
        println!(
            "   agend git hooks  <tmp>/home/hooks, set in the worktree's config.worktree only"
        );
        println!("   binding snapshot <tmp>/home/bindings/{INSTANCE}.json");
        println!(
            "   shim             <tmp>/bin/{{git,kill,pkill,killall}} -> {}",
            agend.display()
        );
        println!(
            "   fake kill tools  <tmp>/fakebin/{{kill,pkill,killall}} (record argv, send no signal)"
        );
        Ok(Env {
            root,
            bin,
            kill_log,
            home,
            repo,
            origin,
            worktree,
            workspace,
            branch,
            path,
        })
    }

    pub fn run() -> Result<(), String> {
        println!("== shim demo ==");
        let env = setup()?;
        let keep = std::env::var_os("AGEND_SHIM_DEMO_KEEP").is_some_and(|v| v == "1");
        let result = steps(&env);
        if keep {
            print_manual(&env);
        } else {
            let _ = std::fs::remove_dir_all(&env.root);
        }
        result?;
        println!("shim demo: all checks passed");
        Ok(())
    }

    fn steps(env: &Env) -> Result<(), String> {
        route(env)?;
        refuse(env)?;
        snapshot(env)?;
        protected(env)?;
        hooks(env)?;
        kill_guard(env)?;
        audit(env)
    }

    fn route(env: &Env) -> Result<(), String> {
        println!(
            "\n-- route: commit from the agent's workspace (not a repo) and from the canonical checkout"
        );
        let main_before = git(&env.repo, &["rev-parse", "main"])?;
        std::fs::write(env.worktree.join("fix.txt"), "the fix\n").map_err(|e| e.to_string())?;
        env.shim("git", &["add", "fix.txt"], &env.workspace)?;
        let out = env.shim("git", &["commit", "-q", "-m", "fix"], &env.workspace)?;
        check(out.status.success(), "commit ran")?;
        let subject = git(&env.worktree, &["log", "-1", "--format=%s", &env.branch])?;
        check(
            subject == "fix",
            &format!("commit \"fix\" is on {}", env.branch),
        )?;
        std::fs::write(env.worktree.join("more.txt"), "more\n").map_err(|e| e.to_string())?;
        let out = env.shim("git", &["add", "more.txt"], &env.repo)?;
        check(
            String::from_utf8_lossy(&out.stderr).contains("running in your bound worktree"),
            "git run in the canonical checkout is routed into the worktree, with a note",
        )?;
        env.shim("git", &["commit", "-q", "-m", "more"], &env.repo)?;
        check(
            git(&env.repo, &["rev-parse", "main"])? == main_before,
            "canonical main did not move",
        )?;
        check(
            git(&env.repo, &["status", "--porcelain"])?.is_empty(),
            "canonical checkout is clean",
        )
    }

    fn refuse(env: &Env) -> Result<(), String> {
        println!("\n-- refuse: agent tries `git checkout main` and `git worktree add`");
        let out = env.shim("git", &["checkout", "main"], &env.worktree)?;
        check(out.status.code() == Some(1), "exit 1")?;
        let next = Env::stderr_line(&out, "agend-shim: next step: ").unwrap_or_default();
        check(
            next.contains("agend task create"),
            "message names the next step (agend task create)",
        )?;
        check(
            git(&env.worktree, &["branch", "--show-current"])? == env.branch,
            &format!("still on {}", env.branch),
        )?;
        let out = env.shim("git", &["worktree", "add", "../mine"], &env.worktree)?;
        check(out.status.code() == Some(1), "worktree add refused")?;
        check(
            git(&env.repo, &["worktree", "list"])?.lines().count() == 2,
            "no new worktree",
        )
    }

    fn snapshot(env: &Env) -> Result<(), String> {
        println!("\n-- snapshot: `git reset --hard HEAD~1` with uncommitted work, then undo");
        std::fs::write(env.worktree.join("fix.txt"), "the fix, plus unsaved work\n")
            .map_err(|e| e.to_string())?;
        let out = env.shim("git", &["reset", "--hard", "HEAD~1"], &env.worktree)?;
        let id = Env::stderr_line(&out, "agend-shim: snapshot ").unwrap_or_default();
        check(!id.is_empty(), "snapshot id printed before the reset")?;
        check(
            std::fs::read_to_string(env.worktree.join("fix.txt"))
                .map(|s| s == "the fix\n")
                .unwrap_or(true),
            "the reset really discarded the unsaved work",
        )?;
        let undo = Env::stderr_line(&out, "agend-shim: to undo: ").ok_or("no undo line")?;
        println!("   undo command from the message: {undo}");
        for part in undo.split(" && ") {
            let words: Vec<&str> = part.split_whitespace().collect();
            let (tool, args) = words.split_first().ok_or("empty undo command")?;
            let out = env.shim(tool, args, &env.worktree)?;
            check(out.status.success(), &format!("`{part}` ran"))?;
        }
        let restored = std::fs::read_to_string(env.worktree.join("fix.txt")).unwrap_or_default();
        check(
            restored == "the fix, plus unsaved work\n",
            "fix.txt is back, including the unsaved work",
        )?;
        check(
            git(&env.worktree, &["log", "-1", "--format=%s"])? == "more",
            "the reset commits are back",
        )
    }

    fn protected(env: &Env) -> Result<(), String> {
        println!(
            "\n-- protected ref: the agend hooks refuse what git reports, however the command was spelled"
        );
        let main_before = git(&env.repo, &["rev-parse", "main"])?;
        let origin_main = git(&env.origin, &["rev-parse", "main"])?;
        for args in [
            &["update-ref", "refs/heads/main", "HEAD"][..],
            &["push", ".", "HEAD:main"][..],
            &["branch", "-f", "master", "HEAD"][..],
            &["push", "origin", "HEAD:main"][..],
        ] {
            let out = env.shim("git", args, &env.worktree)?;
            check(
                out.status.code() != Some(0)
                    && String::from_utf8_lossy(&out.stderr).contains("it is a protected ref"),
                "refused by an agend hook as a protected ref",
            )?;
        }
        check(
            git(&env.repo, &["rev-parse", "main"])? == main_before,
            "main did not move",
        )?;
        check(
            git(&env.origin, &["rev-parse", "main"])? == origin_main,
            "origin's main did not move",
        )?;
        let out = env.shim(
            "git",
            &["push", "-q", "-u", "origin", &env.branch],
            &env.worktree,
        )?;
        check(out.status.success(), "pushing your own branch runs")?;
        let out = env.shim("git", &["push", "-q"], &env.workspace)?;
        check(
            out.status.success(),
            "plain `git push` runs (git resolves it to your branch)",
        )
    }

    fn hooks(env: &Env) -> Result<(), String> {
        println!(
            "\n-- hooks: only the agent worktree has them; project hooks still run; they cannot be skipped"
        );
        let canonical = Command::new("git")
            .arg("-C")
            .arg(&env.repo)
            .args(["config", "core.hooksPath"])
            .output()
            .map_err(|e| e.to_string())?;
        check(
            canonical.stdout.is_empty(),
            "the canonical checkout has no core.hooksPath",
        )?;
        let log = env.root.join("project-hook.log");
        let hook = env.repo.join(".git").join("hooks").join("pre-commit");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\necho project pre-commit ran >> '{}'\n",
                log.display()
            ),
        )
        .map_err(|e| e.to_string())?;
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        let out = env.shim(
            "git",
            &["commit", "-q", "--allow-empty", "-m", "hooked"],
            &env.worktree,
        )?;
        check(out.status.success(), "the agent's commit ran")?;
        check(
            std::fs::read_to_string(&log).unwrap_or_default() == "project pre-commit ran\n",
            "the project's pre-commit hook ran from the agent worktree",
        )?;
        for args in [
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "update-ref",
                "refs/heads/main",
                "HEAD",
            ][..],
            &["push", "--no-verify", "origin", "HEAD:main"][..],
        ] {
            let out = env.shim("git", args, &env.worktree)?;
            check(
                out.status.code() == Some(1)
                    && String::from_utf8_lossy(&out.stderr).contains("skip the agend"),
                "skipping the hooks is refused",
            )?;
        }
        std::fs::remove_file(&hook).map_err(|e| e.to_string())?;
        let out = git(
            &env.repo,
            &["commit", "-q", "--allow-empty", "-m", "human on main"],
        );
        check(
            out.is_ok(),
            "a human commit on main in the canonical checkout still works",
        )
    }

    fn sleep_bin() -> &'static str {
        if Path::new("/bin/sleep").exists() {
            "/bin/sleep"
        } else {
            "/usr/bin/sleep"
        }
    }

    fn alive(child: &mut Child) -> bool {
        matches!(child.try_wait(), Ok(None))
    }

    fn kill_guard(env: &Env) -> Result<(), String> {
        println!("\n-- kill guard: kill a holder, pkill by pattern");
        // A stand-in holder: `sleep` exec'd under the name `agend`, like the
        // daemon and holders.
        let fake = env.root.join("holder").join("agend");
        std::fs::create_dir_all(fake.parent().unwrap()).map_err(|e| e.to_string())?;
        std::os::unix::fs::symlink(sleep_bin(), &fake).map_err(|e| e.to_string())?;
        let mut holder = Command::new(&fake)
            .arg("300")
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut mine = Command::new(sleep_bin())
            .arg("300")
            .spawn()
            .map_err(|e| e.to_string())?;
        std::thread::sleep(std::time::Duration::from_millis(200));
        let result = (|| {
            let hpid = holder.id().to_string();
            let out = env.shim("kill", &[&hpid], &env.workspace)?;
            check(out.status.code() == Some(1), "kill <holder pid> refused")?;
            let out = env.shim("pkill", &["-f", "sleep 300"], &env.workspace)?;
            check(out.status.code() == Some(1), "pkill refused")?;
            check(alive(&mut holder), "holder still alive")?;
            check(alive(&mut mine), "the agent's own sleep still alive too")?;
            let mpid = mine.id().to_string();
            let out = env.shim("kill", &[&mpid], &env.workspace)?;
            check(out.status.success(), "kill <own pid> is allowed")?;
            let log = std::fs::read_to_string(&env.kill_log).unwrap_or_default();
            check(
                log == format!("kill {mpid}\n"),
                "only that call reached the (fake) real kill",
            )?;
            Ok(())
        })();
        let _ = holder.kill();
        let _ = holder.wait();
        let _ = mine.kill();
        let _ = mine.wait();
        result
    }

    fn audit(env: &Env) -> Result<(), String> {
        println!("\n-- audit: <tmp>/home/audit/shim.jsonl");
        let records = agend_shim::audit::read(&env.home);
        for r in &records {
            println!(
                "   {:<8} {:<6} {:<16} {}",
                r.event,
                r.tool,
                r.code.as_deref().unwrap_or("-"),
                r.argv.join(" ")
            );
        }
        let refusals = records.iter().filter(|r| r.event == "refuse").count();
        let snaps = records.iter().filter(|r| r.event == "snapshot").count();
        check(
            refusals == 10,
            &format!(
                "{refusals} refusals recorded (want 10: 2 by the shim's branch and worktree rules, 4 by the hooks, 2 hook skips, 2 kills)"
            ),
        )?;
        check(snaps >= 2, &format!("{snaps} snapshots recorded"))
    }

    fn print_manual(env: &Env) {
        println!(
            "\nkept {} (AGEND_SHIM_DEMO_KEEP=1). Try the shim by hand:",
            env.root.display()
        );
        println!("   cd {}", env.workspace.display());
        println!(
            "   export PATH={}:$PATH AGEND_HOME={} AGEND_INSTANCE={INSTANCE}",
            env.bin.display(),
            env.home.display()
        );
        println!("   git status");
        println!("remove it afterwards: rm -rf {}", env.root.display());
    }
}
