//! Patch-only archival cannot preserve a nested repository or special filesystem object.
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub(super) fn verify_index(entries: &Path) -> Result<(), String> {
    for entry in BufReader::new(File::open(entries).map_err(|e| e.to_string())?).split(0) {
        if entry.map_err(|e| e.to_string())?.starts_with(b"160000 ") {
            return Err(
                "nested repository index entry cannot be archived; original WIP retained".into(),
            );
        }
    }
    Ok(())
}

pub(super) fn verify_worktree(wt: &Path, tracked: &Path, untracked: &Path) -> Result<(), String> {
    for (list, can_be_deleted) in [(tracked, true), (untracked, false)] {
        for name in BufReader::new(File::open(list).map_err(|e| e.to_string())?).split(0) {
            let name = name.map_err(|e| e.to_string())?;
            if name.is_empty() {
                continue;
            }
            let name = std::str::from_utf8(&name).map_err(|e| e.to_string())?;
            let kind = match std::fs::symlink_metadata(wt.join(name)) {
                Ok(metadata) => metadata.file_type(),
                Err(error) if can_be_deleted && error.kind() == std::io::ErrorKind::NotFound => {
                    continue;
                }
                Err(error) => {
                    return Err(format!(
                        "cannot inspect WIP path; original WIP retained: {error}"
                    ));
                }
            };
            if !kind.is_file() && !kind.is_symlink() {
                return Err("nested repository or special WIP path cannot be archived; original WIP retained".into());
            }
        }
    }
    Ok(())
}

// Git intentionally hides .git paths, even when partial metadata is no longer a repository.
pub(super) fn verify_metadata(wt: &Path) -> Result<(), String> {
    let mut directories = vec![wt.to_path_buf()];
    while let Some(dir) = directories.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if entry
                .file_name()
                .as_encoded_bytes()
                .eq_ignore_ascii_case(b".git")
            {
                if dir == wt && kind.is_file() {
                    continue;
                }
                return Err("nested Git metadata cannot be archived; original WIP retained".into());
            }
            // Do not follow symlinks: their target bytes are archived separately.
            if kind.is_dir() {
                directories.push(entry.path());
            }
        }
    }
    Ok(())
}
