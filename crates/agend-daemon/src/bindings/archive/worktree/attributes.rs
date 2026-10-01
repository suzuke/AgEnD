//! Patch-only archival cannot prove raw-byte recovery through content conversions.
use super::super::new_file;
use crate::git::Git;
use std::fs::File;
use std::path::Path;

pub(super) async fn verify(
    git: &Git,
    wt: &Path,
    tracked: &Path,
    untracked: &Path,
    staging: &Path,
) -> Result<(), String> {
    let autocrlf = git
        .output(wt, &["config", "--get", "core.autocrlf"])
        .await?;
    if autocrlf.timed_out || !matches!(autocrlf.exit_code, Some(0 | 1)) {
        return Err("cannot inspect WIP line ending configuration; original WIP retained".into());
    }
    if autocrlf.exit_code == Some(0) {
        let value = String::from_utf8_lossy(&autocrlf.stdout);
        if !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "false" | "no" | "off" | "0"
        ) {
            return Err(
                "WIP content conversion prevents raw-byte archive; original WIP retained".into(),
            );
        }
    }
    let paths = staging.join("attribute-paths");
    let mut names = new_file(&paths)?;
    for path in [tracked, untracked] {
        std::io::copy(
            &mut File::open(path).map_err(|e| e.to_string())?,
            &mut names,
        )
        .map_err(|e| e.to_string())?;
    }
    let out = git
        .output_from_file(
            wt,
            &[
                "check-attr",
                "-z",
                "filter",
                "working-tree-encoding",
                "ident",
                "text",
                "eol",
                "crlf",
                "--stdin",
            ],
            &paths,
        )
        .await?;
    if out.timed_out
        || out.exit_code != Some(0)
        || out.stdout.len() > crate::runner::OUTPUT_LIMIT
        || !out.stdout.is_empty() && out.stdout.last() != Some(&0)
    {
        return Err("cannot inspect WIP content attributes; original WIP retained".into());
    }
    if out.stdout.is_empty() {
        return Ok(());
    }
    let mut fields = out.stdout.split(|b| *b == 0).collect::<Vec<_>>();
    fields.pop();
    if fields.len() % 3 != 0 {
        return Err("invalid WIP content attributes; original WIP retained".into());
    }
    for entry in fields.chunks_exact(3) {
        if entry[2] != b"unspecified" && entry[2] != b"unset" {
            return Err(
                "WIP content conversion prevents raw-byte archive; original WIP retained".into(),
            );
        }
    }
    Ok(())
}
