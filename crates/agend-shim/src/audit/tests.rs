//! Unit tests for `audit`.

use super::*;
use agend_testkit::tempdir::TempDir;

#[test]
fn records_append_and_read_back() {
    let dir = TempDir::new("audit").unwrap();
    let rec = Record {
        ts: 1,
        instance: Some("dev-1".into()),
        tool: "git".into(),
        event: "refuse".into(),
        code: Some("branch_switch".into()),
        argv: vec!["checkout".into(), "main".into()],
        cwd: "/w".into(),
        detail: None,
    };
    append(Some(dir.path()), &rec);
    append(Some(dir.path()), &rec);
    assert_eq!(read(dir.path()), vec![rec.clone(), rec]);
}

#[test]
fn unwritable_home_is_ignored() {
    let rec = Record {
        ts: 1,
        instance: None,
        tool: "git".into(),
        event: "bypass".into(),
        code: None,
        argv: vec![],
        cwd: "/".into(),
        detail: None,
    };
    append(Some(Path::new("/dev/null/nope")), &rec);
    append(None, &rec);
}
