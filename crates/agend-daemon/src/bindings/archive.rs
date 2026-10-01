//! Stream WIP artifacts without the diagnostic output cap; publish before removal.
use crate::git::Git;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())
}
async fn append(
    git: &Git,
    repo: &Path,
    args: &[&str],
    file: &File,
    no_index: bool,
) -> Result<(), String> {
    let out = git
        .output_to_file(repo, args, file.try_clone().map_err(|e| e.to_string())?)
        .await?;
    let accepted = out.exit_code == Some(0) || no_index && out.exit_code == Some(1);
    if out.timed_out || !accepted {
        return Err(format!(
            "cannot archive WIP: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

pub async fn archive(
    git: &Git,
    home: &Path,
    repo: &Path,
    wt: &Path,
    task: &str,
    branch: Option<&str>,
    merged: bool,
) -> Result<Option<String>, String> {
    let dir = home.join("archive");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let temporary = dir.join(format!(
        ".{}",
        crate::store::instances::new_session_id().map_err(|e| e.to_string())?
    ));
    std::fs::create_dir(&temporary).map_err(|e| e.to_string())?;
    let staging = Staging(temporary);
    let patch_path = staging.0.join("wip.patch");
    let patch = new_file(&patch_path)?;
    if !merged && let Some(branch) = branch {
        // A retry after successful ref deletion may have neither branch nor worktree.
        // Other inspection failures must not discard potentially unmerged commits.
        let found = git
            .output(
                repo,
                &[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{branch}"),
                ],
            )
            .await?;
        let inspected = found.exit_code == Some(0) || found.exit_code == Some(1) && !wt.exists();
        if found.timed_out || !inspected {
            return Err("cannot inspect branch while archiving WIP".into());
        }
        if found.exit_code == Some(0) {
            append(
                git,
                repo,
                &[
                    "format-patch",
                    "--stdout",
                    "--binary",
                    "--full-index",
                    &format!("main..{branch}"),
                ],
                &patch,
                false,
            )
            .await?;
        }
    }
    if wt.exists() {
        append(git, wt, &["diff", "--binary", "HEAD"], &patch, false).await?;
        let list_path = staging.0.join("untracked");
        let list = new_file(&list_path)?;
        append(
            git,
            wt,
            &["ls-files", "--others", "--exclude-standard", "-z"],
            &list,
            false,
        )
        .await?;
        // Names also bypass the output cap; reopen at offset zero and read one at a time.
        let names = BufReader::new(File::open(&list_path).map_err(|e| e.to_string())?).split(0);
        for name in names {
            let name = name.map_err(|e| e.to_string())?;
            if name.is_empty() {
                continue;
            }
            let name = std::str::from_utf8(&name).map_err(|e| e.to_string())?;
            append(
                git,
                wt,
                &["diff", "--no-index", "--binary", "--", "/dev/null", name],
                &patch,
                true,
            )
            .await?;
        }
    }
    if patch.metadata().map_err(|e| e.to_string())?.len() == 0 {
        return Ok(None);
    }
    patch.sync_all().map_err(|e| e.to_string())?;
    // Publish a complete, synced inode atomically, without replacing an earlier archive.
    let mut stamp = crate::log::now_unix_ms();
    let path = loop {
        let path = dir.join(format!("{task}-{stamp}.patch"));
        match std::fs::hard_link(&patch_path, &path) {
            Ok(()) => break path,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => stamp += 1,
            Err(e) => return Err(e.to_string()),
        }
    };
    File::open(&dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}
