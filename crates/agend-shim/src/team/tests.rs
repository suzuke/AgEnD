//! Unit tests for `team`.

use super::*;

#[test]
fn network_urls_normalise_to_host_and_path() {
    let base = Path::new("/");
    let want = Some(Key::Remote("github.com/suzuke/agend".into()));
    for u in [
        "git@github.com:suzuke/agend.git",
        "ssh://git@github.com/suzuke/agend",
        "https://github.com/suzuke/agend/",
        "https://user@GitHub.com:443/suzuke/agend.git",
        "github.com:suzuke/agend",
    ] {
        assert_eq!(key(u, base), want, "{u}");
    }
    assert_ne!(key("https://github.com/other/agend", base), want);
}

#[test]
fn insteadof_rewrites_before_comparing() {
    let r = Remotes::from_config(&[
        ("remote.origin.url".into(), "gh:suzuke/agend".into()),
        ("url.https://github.com/.insteadof".into(), "gh:".into()),
    ]);
    assert_eq!(
        r.rewrite("gh:suzuke/agend"),
        "https://github.com/suzuke/agend"
    );
    assert_eq!(
        r.keys(Path::new("/")),
        vec![Key::Remote("github.com/suzuke/agend".into())]
    );
    assert_eq!(
        r.dest_keys("origin", Path::new("/")),
        r.dest_keys("gh:suzuke/agend", Path::new("/"))
    );
}

/// Round 2: git resolves a local remote without its `.git` suffix.
#[test]
fn local_paths_resolve_like_git_enter_repo() {
    let tmp = agend_testkit::tempdir::TempDir::new("team-key").unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let bare = root.join("origin.git");
    std::fs::create_dir_all(bare.join("objects")).unwrap();
    std::fs::write(bare.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let want = Some(Key::Local(bare.clone()));
    let r = root.display();
    for u in [
        format!("{r}/origin.git"),
        format!("{r}/origin"),
        format!("{r}/origin/"),
        format!("{r}/./origin"),
        format!("file://{r}/origin"),
        format!("file://{r}/origin.git/"),
        format!("file://localhost{r}/origin.git"),
        format!("file://LOCALHOST{r}/origin"),
        format!("file://127.0.0.1{r}/origin"),
        "origin".to_string(),
        "../x/../origin".to_string(),
    ] {
        let base = if u.starts_with("..") {
            root.join("x")
        } else {
            root.clone()
        };
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(key(&u, &base), want, "{u}");
    }
    // A real repo at the suffix-less path wins, as in git.
    let plain = root.join("origin");
    std::fs::create_dir_all(plain.join("objects")).unwrap();
    std::fs::write(plain.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    assert_eq!(key(&format!("{r}/origin"), &root), Some(Key::Local(plain)));
}
