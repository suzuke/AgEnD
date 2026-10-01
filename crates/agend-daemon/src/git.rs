//! Git through Runner, with a clean environment and hooks/fsmonitor disabled.
use crate::runner::{ProcessRunner, quote};
use agend_core::traits::{CommandOutput, Runner};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Git<R = ProcessRunner> {
    pub executable: PathBuf,
    pub runner: R,
}

impl Git {
    pub async fn checked(home: &Path) -> Result<Self, String> {
        let git = Self::discover(home)?;
        let version = git.run(home, &["--version"]).await?;
        if !agend_core::setup::parse_git_version(&version)
            .is_some_and(agend_core::setup::git_is_new_enough)
        {
            return Err(format!("git >= 2.38 required, found {version}"));
        }
        Ok(git)
    }
    /// Artifact output bypasses the diagnostic cap and goes directly to an owned file.
    pub async fn output_to_file(
        &self,
        repo: &Path,
        args: &[&str],
        file: std::fs::File,
    ) -> Result<CommandOutput, String> {
        self.runner
            .run_to_file(&self.command(args), &repo.to_string_lossy(), 60_000, file)
            .await
            .map_err(|e| e.to_string())
    }
    /// Feed a complete path list without shell interpolation or argv size limits.
    pub async fn output_from_file(
        &self,
        repo: &Path,
        args: &[&str],
        input: &Path,
    ) -> Result<CommandOutput, String> {
        let command = format!(
            "{} < {}",
            self.command(args),
            quote(&input.to_string_lossy())
        );
        self.runner
            .run(&command, &repo.to_string_lossy(), 60_000)
            .await
            .map_err(|e| e.to_string())
    }
    pub fn discover(home: &Path) -> Result<Self, String> {
        let daemon: BTreeMap<String, String> = std::env::vars().collect();
        let path = crate::runtime::env::launch_path(home, &daemon);
        let shim = home.join("bin");
        let executable = path
            .split(':')
            .filter_map(|p| {
                let p = Path::new(p).join("git");
                let actual = p.canonicalize().ok()?;
                if p.parent().and_then(|p| p.canonicalize().ok()) == shim.canonicalize().ok() {
                    return None;
                }
                if actual.is_file() { Some(actual) } else { None }
            })
            .next()
            .ok_or("real git not found outside AGEND_HOME/bin")?;
        let mut env = BTreeMap::from([
            ("PATH".into(), path),
            ("LANG".into(), "C".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ]);
        if let Some(value) = daemon.get("HOME") {
            env.insert("HOME".into(), value.clone());
        }
        Ok(Self {
            executable,
            runner: ProcessRunner { env },
        })
    }
}

impl<R: Runner> Git<R>
where
    R::Error: std::fmt::Display,
{
    fn command(&self, args: &[&str]) -> String {
        let mut command = format!(
            "{} -c core.hooksPath=/dev/null -c core.fsmonitor=false",
            quote(&self.executable.to_string_lossy())
        );
        for arg in args {
            command.push(' ');
            command.push_str(&quote(arg));
        }
        command
    }

    pub async fn output(&self, repo: &Path, args: &[&str]) -> Result<CommandOutput, String> {
        let command = self.command(args);
        self.runner
            .run(&command, &repo.to_string_lossy(), 60_000)
            .await
            .map_err(|e| e.to_string())
    }
    pub async fn run(&self, repo: &Path, args: &[&str]) -> Result<String, String> {
        let out = self.output(repo, args).await?;
        if out.exit_code != Some(0) || out.timed_out {
            return Err(format!(
                "git {}: {}{}",
                args.first().unwrap_or(&""),
                String::from_utf8_lossy(&out.stderr),
                if out.timed_out { " (timeout)" } else { "" }
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }
    /// A concealed index entry cannot prove a worktree safe to overwrite.
    pub async fn clean_worktree(&self, repo: &Path) -> Result<bool, String> {
        let out = self.output(repo, &["ls-files", "-v", "-z"]).await?;
        if out.timed_out
            || out.exit_code != Some(0)
            || out.stdout.len() > crate::runner::OUTPUT_LIMIT
            || !out.stdout.is_empty() && out.stdout.last() != Some(&0)
        {
            return Err("cannot inspect worktree index flags".into());
        }
        if out.stdout.split(|b| *b == 0).any(|entry| {
            entry
                .first()
                .is_some_and(|tag| *tag == b'S' || tag.is_ascii_lowercase())
        }) {
            return Ok(false);
        }
        Ok(self.run(repo, &["status", "--porcelain"]).await?.is_empty())
    }
    pub async fn ancestor(&self, repo: &Path, a: &str, b: &str) -> Result<bool, String> {
        let out = self
            .output(repo, &["merge-base", "--is-ancestor", a, b])
            .await?;
        match out.exit_code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(String::from_utf8_lossy(&out.stderr).into()),
        }
    }
    pub async fn patch_id(&self, repo: &Path, head: &str) -> Result<String, String> {
        let base = self.run(repo, &["merge-base", "main", head]).await?;
        let git = quote(&self.executable.to_string_lossy());
        let command = format!(
            "{git} -c core.hooksPath=/dev/null -c core.fsmonitor=false diff --binary --no-ext-diff --no-textconv {} {} | {git} -c core.hooksPath=/dev/null -c core.fsmonitor=false patch-id --stable",
            quote(&base),
            quote(head)
        );
        let output = self
            .runner
            .run(
                &format!("/bin/bash -o pipefail -c {}", quote(&command)),
                &repo.to_string_lossy(),
                60_000,
            )
            .await
            .map_err(|e| e.to_string())?;
        if output.exit_code != Some(0) {
            return Err("git patch-id failed".into());
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .unwrap_or("empty")
            .into())
    }
}
