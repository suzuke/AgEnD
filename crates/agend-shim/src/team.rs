//! Recognises the team's repo by destination, not by cwd: a repo whose
//! remote points at the team's remote or at the canonical checkout (a clone),
//! and a push whose destination is either, are the team repo too.
//!
//! Remote URLs are compared after `url.<base>.insteadOf` /
//! `pushInsteadOf` rewriting and normalisation (`git@host:o/r.git`,
//! `ssh://git@host/o/r`, `https://host/o/r/` are the same remote; a local
//! path is compared by its git common dir, found the way git finds it:
//! `/x/origin` and `file:///x/origin` reach `/x/origin.git`).
//!
//! Must NOT: spawn git (callers pass the config they read).

use crate::location;
use std::path::{Path, PathBuf};

/// A normalised remote identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// A local repo, by canonical git common dir.
    Local(PathBuf),
    /// `host/path` of a network URL, lowercased host, no `.git` suffix.
    Remote(String),
}

/// One repo's remote config: URLs by remote name and rewrite rules.
#[derive(Debug, Clone, Default)]
pub struct Remotes {
    /// (remote name, url or pushurl).
    pub urls: Vec<(String, String)>,
    /// (prefix, replacement) from `url.<replacement>.insteadOf|pushInsteadOf`.
    pub rewrites: Vec<(String, String)>,
}

/// Config regex that selects everything `Remotes::from_config` reads.
pub const CONFIG_REGEX: &str = r"^(remote\..*\.(url|pushurl)|url\..*\.(insteadof|pushinsteadof))$";

impl Remotes {
    /// From `git config --get-regexp CONFIG_REGEX` output pairs.
    pub fn from_config(entries: &[(String, String)]) -> Remotes {
        let mut r = Remotes::default();
        for (key, value) in entries {
            let lower = key.to_ascii_lowercase();
            if let Some(rest) = key.strip_prefix("remote.")
                && (lower.ends_with(".url") || lower.ends_with(".pushurl"))
                && let Some((name, _)) = rest.rsplit_once('.')
            {
                r.urls.push((name.to_string(), value.clone()));
            } else if let Some(rest) = key.strip_prefix("url.")
                && let Some((base, _)) = rest.rsplit_once('.')
            {
                r.rewrites.push((value.clone(), base.to_string()));
            }
        }
        r
    }

    /// `url` after the longest matching insteadOf rule.
    pub fn rewrite(&self, url: &str) -> String {
        self.rewrites
            .iter()
            .filter(|(prefix, _)| url.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(prefix, base)| format!("{base}{}", &url[prefix.len()..]))
            .unwrap_or_else(|| url.to_string())
    }

    /// Keys of every remote URL (after rewriting), relative paths resolved
    /// against `base`.
    pub fn keys(&self, base: &Path) -> Vec<Key> {
        self.urls
            .iter()
            .filter_map(|(_, u)| key(&self.rewrite(u), base))
            .collect()
    }

    /// Keys of a push destination: a configured remote's URLs, or the
    /// destination itself as a URL or path.
    pub fn dest_keys(&self, dest: &str, base: &Path) -> Vec<Key> {
        let named: Vec<Key> = self
            .urls
            .iter()
            .filter(|(n, _)| n == dest)
            .filter_map(|(_, u)| key(&self.rewrite(u), base))
            .collect();
        if !named.is_empty() {
            return named;
        }
        key(&self.rewrite(dest), base).into_iter().collect()
    }
}

/// The team: the canonical checkout's common dir and its remotes.
pub fn team_keys(source_repo: &Path, remotes: &Remotes) -> Vec<Key> {
    let mut keys = remotes.keys(source_repo);
    if let Some(k) = local_key(source_repo) {
        keys.push(k);
    }
    keys
}

/// Normalised identity of a URL or path (`None` if it cannot be one).
pub fn key(url: &str, base: &Path) -> Option<Key> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return local_key(&base.join(rest));
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit('@').next().unwrap_or(authority);
        let host = host.split(':').next().unwrap_or(host);
        return Some(remote_key(host, path));
    }
    // git: local if there is no ':' or a '/' comes before the first ':'.
    let colon = url.find(':');
    let slash = url.find('/');
    match (colon, slash) {
        (Some(c), s) if s.is_none_or(|s| c < s) && c > 1 => {
            let host = url[..c].rsplit('@').next().unwrap_or(&url[..c]);
            Some(remote_key(host, &url[c + 1..]))
        }
        _ => local_key(&base.join(url)),
    }
}

fn remote_key(host: &str, path: &str) -> Key {
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    let path = path.strip_prefix("~/").unwrap_or(path);
    Key::Remote(format!("{}/{path}", host.to_ascii_lowercase()))
}

/// A local repo (checkout, worktree, bare or git dir) by common git dir.
/// Resolved the way git's `enter_repo` does for a local remote: it tries
/// `<p>/.git`, `<p>`, `<p>.git/.git`, `<p>.git` and takes the first repo,
/// so `/x/origin` reaches `/x/origin.git`. If none looks like a repo, the
/// path itself (canonical) is the key.
fn local_key(p: &Path) -> Option<Key> {
    // `components()` drops trailing slashes, so `/x/origin/` + `.git` is
    // `/x/origin.git`, not `/x/origin/.git`.
    let plain: PathBuf = p.components().collect();
    let mut dotgit = plain.clone().into_os_string();
    dotgit.push(".git");
    [plain, PathBuf::from(dotgit)]
        .iter()
        .find_map(|c| repo_key(c))
        .or_else(|| std::fs::canonicalize(p).ok().map(Key::Local))
}

/// `Some` when `p` is a checkout (`p/.git`) or a git dir itself.
fn repo_key(p: &Path) -> Option<Key> {
    let p = std::fs::canonicalize(p).ok()?;
    let gitdir =
        location::gitdir_of_checkout(&p).or_else(|| location::is_git_dir(&p).then(|| p.clone()))?;
    Some(Key::Local(location::common_dir(&gitdir)))
}

#[cfg(test)]
mod tests {
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
}
