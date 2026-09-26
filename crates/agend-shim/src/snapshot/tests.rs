//! Unit tests for `snapshot`.

use super::*;

#[test]
fn report_names_the_undo_command() {
    let saved = Saved {
        id: "1-2".into(),
        reference: format!("{REF_PREFIX}dev-1/1-2"),
        previous_head: Some("abc".into()),
    };
    let lines = saved.report("reset --hard HEAD~1");
    assert!(lines[0].contains("snapshot 1-2 saved before `git reset --hard HEAD~1`"));
    assert_eq!(
        lines[1],
        "agend-shim: to undo: git reset --keep abc && git restore --source=refs/agend/snapshots/dev-1/1-2 -- :/"
    );
}
