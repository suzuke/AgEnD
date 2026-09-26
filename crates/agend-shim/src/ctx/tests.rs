//! Unit tests for `ctx`.

use super::*;
use agend_testkit::tempdir::TempDir;
use std::os::unix::fs::symlink;

#[test]
fn real_tool_skips_links_to_the_shim() {
    let dir = TempDir::new("ctx").unwrap();
    let shim_dir = dir.path().join("shim");
    let real_dir = dir.path().join("real");
    std::fs::create_dir_all(&shim_dir).unwrap();
    std::fs::create_dir_all(&real_dir).unwrap();
    let me = dir.path().join("agend");
    std::fs::write(&me, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&me, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    symlink(&me, shim_dir.join("git")).unwrap();
    std::fs::hard_link(&me, real_dir.join("pkill")).unwrap();
    let real_git = real_dir.join("git");
    std::fs::write(&real_git, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(
        &real_git,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let ctx = Ctx {
        path: std::env::join_paths([&shim_dir, &real_dir]).unwrap(),
        self_exe: Some(me),
        ..Ctx::default()
    };
    assert_eq!(ctx.find_real("git"), Some(real_git));
    assert_eq!(ctx.find_real("pkill"), None, "hard link to the shim");
    assert_eq!(ctx.find_real("nope"), None);
}
