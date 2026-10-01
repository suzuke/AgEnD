//! Cold detached checks worktrees and fail-closed write sandboxes (P6).
use crate::git::Git;
use crate::runner::{ProcessRunner, quote};
use agend_core::traits::{CommandOutput, Runner};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use agend_core::pipeline::ports::ExecutionError as CheckError;

pub fn tool(home: &Path) -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("AGEND_SANDBOX_TOOL") {
        return Some(p.into());
    }
    #[cfg(target_os = "macos")]
    {
        let _ = home;
        Some("/usr/bin/sandbox-exec".into())
    }
    #[cfg(target_os = "linux")]
    {
        let vars = std::env::vars().collect();
        crate::runtime::env::launch_path(home, &vars)
            .split(':')
            .map(|p| Path::new(p).join("bwrap"))
            .find(|p| p.is_file())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = home;
        None
    }
}

fn profile_path(p: &Path) -> String {
    p.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Probe the platform tool without writing files or opening the database.
pub async fn readiness(home: &Path) -> Result<(), String> {
    let tool = tool(home).ok_or("checks sandbox tool not found")?;
    #[cfg(target_os = "macos")]
    let args = {
        let profile = if cfg!(debug_assertions)
            && std::env::var_os("AGEND_SANDBOX_BAD_PROFILE").is_some()
        {
            "(invalid profile)"
        } else {
            "(version 1) (allow default) (deny file-write*) (deny network-outbound (remote unix-socket))"
        };
        format!("-p {} /bin/sh -c true", quote(profile))
    };
    #[cfg(target_os = "linux")]
    let args =
        "--ro-bind / / --dev /dev --proc /proc --unshare-pid --die-with-parent -- /bin/sh -c true";
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Err("checks sandbox unsupported on this platform".into());
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let runner = ProcessRunner {
            env: std::collections::BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        };
        let out = runner
            .run(
                &format!("exec {} {args}", quote(&tool.to_string_lossy())),
                "/",
                5000,
            )
            .await
            .map_err(|e| e.to_string())?;
        if out.exit_code == Some(0) {
            Ok(())
        } else {
            Err(format!(
                "sandbox probe failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))
        }
    }
}

pub fn sandbox_command(
    home: &Path,
    worktree: &Path,
    tmp: &Path,
    command: &str,
) -> Result<String, String> {
    let tool = tool(home).ok_or("sandbox tool not found")?;
    if !tool.is_file() {
        return Err(format!("sandbox tool {} not found", tool.display()));
    }
    #[cfg(target_os = "macos")]
    {
        let profile = format!(
            "(version 1) (allow default) (deny file-write*) (allow file-write* (subpath \"{}\") (subpath \"{}\") (literal \"/dev/null\") (literal \"/dev/tty\")) (deny file-write* (literal \"{}/.git\")) (deny network-outbound (remote unix-socket)) (allow network-outbound (remote unix-socket (path-literal \"/private/var/run/mDNSResponder\")))",
            profile_path(worktree),
            profile_path(tmp),
            profile_path(worktree)
        );
        #[cfg(debug_assertions)]
        let profile = if std::env::var_os("AGEND_SANDBOX_BAD_PROFILE").is_some() {
            "(invalid profile)".into()
        } else {
            profile
        };
        Ok(format!(
            "exec {} -p {} /bin/sh -c {}",
            quote(&tool.to_string_lossy()),
            quote(&profile),
            quote(command)
        ))
    }
    #[cfg(target_os = "linux")]
    {
        let _ = profile_path(worktree);
        let dotgit = std::fs::read_to_string(worktree.join(".git")).map_err(|e| e.to_string())?;
        let gitdir = PathBuf::from(
            dotgit
                .trim()
                .strip_prefix("gitdir: ")
                .ok_or("invalid linked worktree git file")?,
        );
        let common = gitdir
            .parent()
            .and_then(Path::parent)
            .ok_or("invalid worktree metadata directory")?;
        let canonical_repo = common.parent().ok_or("invalid canonical repository")?;
        // Mount the whole canonical repository read-only before writable roots.
        // Binding only .git after hiding /tmp creates a writable parent skeleton.
        let mut cmd = format!(
            "exec {} --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp --ro-bind {} {} --bind {} {} --ro-bind {} {} --bind {} {}",
            quote(&tool.to_string_lossy()),
            quote(&canonical_repo.to_string_lossy()),
            quote(&canonical_repo.to_string_lossy()),
            quote(&worktree.to_string_lossy()),
            quote(&worktree.to_string_lossy()),
            quote(&worktree.join(".git").to_string_lossy()),
            quote(&worktree.join(".git").to_string_lossy()),
            quote(&tmp.to_string_lossy()),
            quote(&tmp.to_string_lossy())
        );
        let uid = unsafe { libc::getuid() };
        for dir in [PathBuf::from(format!("/run/user/{uid}")), home.join("run")] {
            if dir.is_dir() {
                cmd.push_str(&format!(
                    " --tmpfs {} --remount-ro {}",
                    quote(&dir.to_string_lossy()),
                    quote(&dir.to_string_lossy())
                ));
            }
        }
        cmd.push_str(&format!(
            " --remount-ro /tmp --unshare-pid --die-with-parent -- /bin/sh -c {}",
            quote(command)
        ));
        Ok(cmd)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (worktree, tmp, command);
        Err("checks sandbox is unsupported on this platform".into())
    }
}

fn environment(home: &Path, tmp: &Path) -> BTreeMap<String, String> {
    let vars: BTreeMap<String, String> = std::env::vars().collect();
    let mut env: BTreeMap<String, String> = crate::runtime::env::PASS_THROUGH
        .iter()
        .filter_map(|k| vars.get(*k).map(|v| (k.to_string(), v.clone())))
        .collect();
    env.insert("PATH".into(), crate::runtime::env::launch_path(home, &vars));
    for name in ["TMPDIR", "CARGO_HOME", "CARGO_TARGET_DIR", "XDG_CACHE_HOME"] {
        let path = tmp.join(name.to_lowercase());
        let _ = std::fs::create_dir_all(&path);
        env.insert(name.into(), path.display().to_string());
    }
    // The marker belongs directly to tmp, not the language-specific subdirectories.
    env.insert("TMPDIR".into(), tmp.display().to_string());
    env
}

async fn probe(home: &Path, worktree: &Path, tmp: &Path) -> Result<(), String> {
    let command = sandbox_command(home, worktree, tmp, "true")?;
    let output = ProcessRunner {
        env: environment(home, tmp),
    }
    .run(&command, &worktree.to_string_lossy(), 5000)
    .await
    .map_err(|e| e.to_string())?;
    if output.exit_code == Some(0) {
        Ok(())
    } else {
        Err(format!(
            "sandbox probe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

pub async fn run(
    git: &Git,
    home: &Path,
    repo: &Path,
    ticket: &str,
    head: &str,
    command: &str,
    timeout_ms: u64,
) -> Result<CommandOutput, CheckError> {
    let run = format!(
        "{}-{}",
        ticket.replace('/', "-"),
        crate::store::instances::new_session_id().map_err(|e| CheckError::Failed(e.to_string()))?
    );
    let root = home.join("checks");
    std::fs::create_dir_all(&root).map_err(|e| CheckError::Failed(e.to_string()))?;
    let root = root
        .canonicalize()
        .map_err(|e| CheckError::Failed(e.to_string()))?;
    let wt = root.join(&run);
    let tmp = root.join(format!("{run}.tmp"));
    std::fs::create_dir_all(&tmp).map_err(|e| CheckError::Failed(e.to_string()))?;
    if let Err(e) = git
        .run(
            repo,
            &["worktree", "add", "--detach", &wt.to_string_lossy(), head],
        )
        .await
    {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(CheckError::Failed(e));
    }
    let repo = repo
        .canonicalize()
        .map_err(|e| CheckError::Failed(e.to_string()))?;
    let result = async {
        probe(home, &wt, &tmp).await.map_err(CheckError::Sandbox)?;
        let marked = format!(
            "printf started > \"$TMPDIR/.agend-sandbox-started\" && exec /bin/sh -c {}",
            quote(command)
        );
        let sandbox = sandbox_command(home, &wt, &tmp, &marked).map_err(CheckError::Sandbox)?;
        let mut output = ProcessRunner {
            env: environment(home, &tmp),
        }
        .run(&sandbox, &wt.to_string_lossy(), timeout_ms)
        .await
        .map_err(|e| CheckError::Failed(e.to_string()))?;
        if !std::fs::symlink_metadata(tmp.join(".agend-sandbox-started"))
            .is_ok_and(|m| m.file_type().is_file())
        {
            probe(home, &wt, &tmp).await.map_err(CheckError::Sandbox)?;
            output.exit_code = Some(1);
            output
                .stderr
                .extend_from_slice(b"\nsandbox marker was removed or replaced\n");
        }
        let log = home
            .join("logs/checks")
            .join(ticket.split('/').next().unwrap_or("unknown"));
        std::fs::create_dir_all(&log).map_err(|e| CheckError::Failed(e.to_string()))?;
        let mut bytes = output.stdout.clone();
        bytes.extend_from_slice(&output.stderr);
        if bytes.len() > 10 * 1024 * 1024 {
            bytes.truncate(10 * 1024 * 1024 - 40);
            bytes.extend_from_slice(b"\n[output truncated at 10 MiB]\n");
        }
        std::fs::write(
            log.join(format!(
                "{}.log",
                ticket.split('/').skip(1).collect::<Vec<_>>().join("-")
            )),
            bytes,
        )
        .map_err(|e| CheckError::Failed(e.to_string()))?;
        Ok(output)
    }
    .await;
    let cleanup = git
        .run(
            &repo,
            &["worktree", "remove", "--force", &wt.to_string_lossy()],
        )
        .await;
    let _ = std::fs::remove_dir_all(&tmp);
    if let Err(e) = cleanup {
        crate::log::line(&format!("checks cleanup: {e}"));
    }
    result
}
