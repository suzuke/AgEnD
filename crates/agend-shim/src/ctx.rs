//! Everything the shim reads from its environment, gathered once so the
//! decision code is testable without touching process-global state, plus
//! locating the real binary on PATH (excluding the shim itself).
//!
//! Inputs (injected by the holder when it starts the agent):
//! - `AGEND_HOME`: AgEnD home; binding snapshots and the audit log live here.
//! - `AGEND_INSTANCE`: this agent's instance id.
//! - `AGEND_SHIM_BYPASS=1`: run the real tool unchecked (audited).
//! - `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE`: git's
//!   own retargeting; `GIT_CEILING_DIRECTORIES` (kept when the shim asks
//!   git where a call acts).
//! - `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_PARAMETERS`: config set for one
//!   call (checked only for `core.hooksPath`, like `-c`).
//!
//! Must NOT: read config files, open the DB or contact the daemon.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Env var that makes the shim run the real tool without checks.
pub const BYPASS_ENV: &str = "AGEND_SHIM_BYPASS";
/// Nesting depth of shim → real tool → shim; guards against a PATH loop.
pub const DEPTH_ENV: &str = "AGEND_SHIM_DEPTH";
/// Depth at which the shim refuses instead of recursing further.
pub const MAX_DEPTH: u32 = 8;
/// git's env vars that choose the repo, work tree or index; removed when
/// the shim asks git about a repo it names itself.
pub const RETARGET_ENV: [&str; 4] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
];

#[derive(Debug, Clone, Default)]
pub struct Ctx {
    pub home: Option<PathBuf>,
    pub instance: Option<String>,
    pub bypass: bool,
    pub depth: u32,
    pub cwd: PathBuf,
    /// `PATH` used to find the real tool.
    pub path: OsString,
    /// The running shim binary; candidates resolving to it are skipped.
    pub self_exe: Option<PathBuf>,
    pub git_dir: Option<PathBuf>,
    pub git_work_tree: Option<PathBuf>,
    pub git_common_dir: Option<PathBuf>,
    pub git_index_file: Option<PathBuf>,
    pub git_ceiling_dirs: Option<OsString>,
    /// Values of `GIT_CONFIG_PARAMETERS` and every `GIT_CONFIG_KEY_<n>`.
    pub config_env: Vec<String>,
}

impl Ctx {
    pub fn from_env() -> Ctx {
        let non_empty = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        let config_env = std::env::vars_os()
            .filter(|(k, _)| {
                k == "GIT_CONFIG_PARAMETERS"
                    || k.to_str().is_some_and(|k| k.starts_with("GIT_CONFIG_KEY_"))
            })
            .map(|(_, v)| v.to_string_lossy().into_owned())
            .collect();
        Ctx {
            home: non_empty("AGEND_HOME").map(PathBuf::from),
            instance: non_empty("AGEND_INSTANCE").map(|v| v.to_string_lossy().into_owned()),
            bypass: std::env::var_os(BYPASS_ENV).is_some_and(|v| v == "1"),
            depth: std::env::var(DEPTH_ENV)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            path: std::env::var_os("PATH").unwrap_or_default(),
            self_exe: std::env::current_exe().ok(),
            git_dir: non_empty("GIT_DIR").map(PathBuf::from),
            git_work_tree: non_empty("GIT_WORK_TREE").map(PathBuf::from),
            git_common_dir: non_empty("GIT_COMMON_DIR").map(PathBuf::from),
            git_index_file: non_empty("GIT_INDEX_FILE").map(PathBuf::from),
            git_ceiling_dirs: non_empty("GIT_CEILING_DIRECTORIES"),
            config_env,
        }
    }

    /// Whether the env chooses the git dir, work tree or common dir (git
    /// then does not find the repo from the work tree).
    pub fn git_env_names_repo(&self) -> bool {
        self.git_dir.is_some() || self.git_work_tree.is_some() || self.git_common_dir.is_some()
    }

    /// Whether git's retargeting env vars are set.
    pub fn git_env_retargets(&self) -> bool {
        self.git_env_names_repo() || self.git_index_file.is_some()
    }

    /// The real `name` binary: the first executable `name` on PATH that is
    /// not this shim (compared by resolved path and by file identity, so a
    /// symlink or hard link to the shim is skipped).
    pub fn find_real(&self, name: &str) -> Option<PathBuf> {
        let me = self.self_exe.as_deref().and_then(identity);
        std::env::split_paths(&self.path)
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(|dir| dir.join(name))
            .find(|cand| is_executable(cand) && (me.is_none() || identity(cand) != me))
    }

    /// A command for the real tool, with the nesting depth bumped and the
    /// working directory set to the caller's.
    pub fn real_command(&self, real: &Path, args: &[OsString]) -> std::process::Command {
        let mut cmd = std::process::Command::new(real);
        cmd.args(args)
            .current_dir(&self.cwd)
            .env(DEPTH_ENV, (self.depth + 1).to_string());
        cmd
    }
}

/// Identity of a file: (device, inode) on unix, so symlinks and hard links
/// to the same binary compare equal; the resolved path elsewhere.
#[cfg(unix)]
pub(crate) fn identity(p: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(p).ok()?;
    Some((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
pub(crate) fn identity(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()
}

pub(crate) fn is_executable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// Lossy UTF-8 view of an argv, for classification and messages.
pub fn lossy(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|a| OsStr::to_string_lossy(a).into_owned())
        .collect()
}

#[cfg(all(test, unix))]
mod tests;
