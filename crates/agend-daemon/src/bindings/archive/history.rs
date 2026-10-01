//! A first-parent patch series includes each merge's complete resolved tree delta.
use super::{append, new_file};
use crate::git::Git;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub(super) async fn append_branch(
    git: &Git,
    repo: &Path,
    branch: &str,
    patch: &File,
    staging: &Path,
) -> Result<(), String> {
    let path = staging.join("commits");
    let revisions = new_file(&path)?;
    append(
        git,
        repo,
        &[
            "rev-list",
            "--reverse",
            "--first-parent",
            &format!("main..{branch}"),
        ],
        &revisions,
        false,
    )
    .await?;
    let commits = BufReader::new(File::open(path).map_err(|e| e.to_string())?).lines();
    for commit in commits {
        let commit = commit.map_err(|e| e.to_string())?;
        let parents = git
            .run(repo, &["show", "--no-patch", "--format=%P", &commit])
            .await?;
        if parents.split_whitespace().count() > 1 {
            // format-patch omits merges. Preserve metadata, then the exact delta
            // against the preceding first-parent tree, including conflict resolutions.
            append(
                git,
                repo,
                &["show", "--no-patch", "--format=email", &commit],
                patch,
                false,
            )
            .await?;
            append(
                git,
                repo,
                &[
                    "diff",
                    "--binary",
                    "--full-index",
                    &format!("{commit}^1"),
                    &commit,
                ],
                patch,
                false,
            )
            .await?;
        } else {
            append(
                git,
                repo,
                &[
                    "format-patch",
                    "-1",
                    "--stdout",
                    "--binary",
                    "--full-index",
                    &commit,
                ],
                patch,
                false,
            )
            .await?;
        }
    }
    Ok(())
}
