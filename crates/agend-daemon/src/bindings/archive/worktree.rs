//! Preserve index and working-tree deltas separately; never discard unresolved index data.
use super::{append, new_file};
mod attributes;
mod paths;
use crate::git::Git;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

fn copy_section(path: &Path, mut destination: &File, marker: &[u8]) -> Result<(), String> {
    let mut source = File::open(path).map_err(|e| e.to_string())?;
    if source.metadata().map_err(|e| e.to_string())?.len() == 0 {
        return Ok(());
    }
    destination.write_all(marker).map_err(|e| e.to_string())?;
    std::io::copy(&mut source, &mut destination).map_err(|e| e.to_string())?;
    Ok(())
}
pub(super) async fn append_wip(
    git: &Git,
    wt: &Path,
    patch: &File,
    staging: &Path,
) -> Result<(), String> {
    paths::verify_metadata(wt)?;
    let conflict_path = staging.join("unmerged");
    let conflicts = new_file(&conflict_path)?;
    append(
        git,
        wt,
        &["ls-files", "--unmerged", "-z"],
        &conflicts,
        false,
    )
    .await?;
    if conflicts.metadata().map_err(|e| e.to_string())?.len() != 0 {
        return Err("unresolved index entries; original WIP retained".into());
    }
    // Rebuild from immutable index entries, without cached stat data or flags.
    // A byte-copy can give the index a fresh mtime and disable Git's racy-file
    // protection, hiding same-size edits. Never modify the original index.
    let entries_path = staging.join("index-entries");
    let entries = new_file(&entries_path)?;
    append(git, wt, &["ls-files", "--stage", "-z"], &entries, false).await?;
    paths::verify_index(&entries_path)?;
    let shadow = staging.join("inspection.index");
    let mut inspection = git.clone();
    inspection.runner.env.insert(
        "GIT_INDEX_FILE".into(),
        shadow.to_string_lossy().into_owned(),
    );
    inspection
        .runner
        .env
        .insert("GIT_CONFIG_COUNT".into(), "2".into());
    inspection
        .runner
        .env
        .insert("GIT_CONFIG_KEY_0".into(), "core.sparseCheckout".into());
    inspection
        .runner
        .env
        .insert("GIT_CONFIG_VALUE_0".into(), "false".into());
    inspection
        .runner
        .env
        .insert("GIT_CONFIG_KEY_1".into(), "core.ignoreStat".into());
    inspection
        .runner
        .env
        .insert("GIT_CONFIG_VALUE_1".into(), "false".into());
    let out = inspection
        .output_from_file(wt, &["update-index", "-z", "--index-info"], &entries_path)
        .await?;
    if out.timed_out || out.exit_code != Some(0) {
        return Err(format!(
            "cannot reconstruct WIP index: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let names_path = staging.join("tracked");
    let names = new_file(&names_path)?;
    append(&inspection, wt, &["ls-files", "-z"], &names, false).await?;
    let git = &inspection;
    let list_path = staging.join("untracked");
    let list = new_file(&list_path)?;
    append(git, wt, &["ls-files", "--others", "-z"], &list, false).await?;
    paths::verify_worktree(wt, &names_path, &list_path)?;
    attributes::verify(git, wt, &names_path, &list_path, staging).await?;
    let index_path = staging.join("index.patch");
    let index = new_file(&index_path)?;
    append(
        git,
        wt,
        &[
            "diff",
            "--binary",
            "--no-color",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--no-relative",
            "--no-ext-diff",
            "--no-textconv",
            "--cached",
            "HEAD",
        ],
        &index,
        false,
    )
    .await?;
    let work_path = staging.join("worktree.patch");
    let work = new_file(&work_path)?;
    append(
        git,
        wt,
        &[
            "diff",
            "--binary",
            "--no-color",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--no-relative",
            "--no-ext-diff",
            "--no-textconv",
        ],
        &work,
        false,
    )
    .await?;
    let names = BufReader::new(File::open(list_path).map_err(|e| e.to_string())?).split(0);
    for name in names {
        let name = name.map_err(|e| e.to_string())?;
        if name.is_empty() {
            continue;
        }
        let name = std::str::from_utf8(&name).map_err(|e| e.to_string())?;
        append(
            git,
            wt,
            &[
                "diff",
                "--no-index",
                "--binary",
                "--no-color",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--no-relative",
                "--no-ext-diff",
                "--no-textconv",
                "--",
                "/dev/null",
                name,
            ],
            &work,
            true,
        )
        .await?;
    }
    copy_section(&index_path, patch, b"\n# agend-wip: index\n")?;
    copy_section(&work_path, patch, b"\n# agend-wip: worktree\n")
}
