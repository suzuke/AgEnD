//! Detect overlapping file sets reported by in-flight work (plan §1, §4.5).
//! The daemon obtains paths from agents; core compares the supplied data.
//!
//! Must NOT: run git or normalize filesystem paths.

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskFileSet {
    pub task_id: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileConflict {
    pub left_task_id: String,
    pub right_task_id: String,
    pub overlapping_files: Vec<String>,
}

/// Return sorted, unique paths that occur in both sets.
pub fn overlap(left: &[String], right: &[String]) -> Vec<String> {
    let left: BTreeSet<&str> = left.iter().map(String::as_str).collect();
    let right: BTreeSet<&str> = right.iter().map(String::as_str).collect();
    left.intersection(&right)
        .map(|path| (*path).to_string())
        .collect()
}

/// Compare each pair once. Input task order is preserved in the output.
pub fn conflicts(tasks: &[TaskFileSet]) -> Vec<FileConflict> {
    let mut found = Vec::new();
    for left_index in 0..tasks.len() {
        for right_index in (left_index + 1)..tasks.len() {
            let files = overlap(&tasks[left_index].files, &tasks[right_index].files);
            if !files.is_empty() {
                found.push(FileConflict {
                    left_task_id: tasks[left_index].task_id.clone(),
                    right_task_id: tasks[right_index].task_id.clone(),
                    overlapping_files: files,
                });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn overlap_is_sorted_and_deduplicated() {
        let left = vec!["src/a.rs".into(), "src/b.rs".into(), "src/a.rs".into()];
        let right = vec!["src/b.rs".into(), "src/a.rs".into()];
        assert_eq!(overlap(&left, &right), ["src/a.rs", "src/b.rs"]);
    }

    #[test]
    fn conflict_report_names_each_overlapping_task_pair() {
        let tasks = [
            TaskFileSet {
                task_id: "T-1".into(),
                files: vec!["src/a.rs".into()],
            },
            TaskFileSet {
                task_id: "T-2".into(),
                files: vec!["src/a.rs".into(), "src/b.rs".into()],
            },
            TaskFileSet {
                task_id: "T-3".into(),
                files: vec!["docs/readme.md".into()],
            },
        ];
        let conflicts = conflicts(&tasks);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].left_task_id, "T-1");
        assert_eq!(conflicts[0].right_task_id, "T-2");
        assert_eq!(conflicts[0].overlapping_files, ["src/a.rs"]);
    }
}
