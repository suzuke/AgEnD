//! Recognises the team's repo by destination, not by cwd: a repo whose
//! remote points at the team's remote or at the canonical checkout (a clone),
//! and a push whose destination is either, are the team repo too. These
//! repos have no agend hooks, so the shim guards them (T5).
//!
//! Remote URLs are compared after `url.<base>.insteadOf` /
//! `pushInsteadOf` rewriting and normalisation (`git@host:o/r.git`,
//! `ssh://git@host/o/r`, `https://host/o/r/` are the same remote; a local
//! path is compared by its git common dir, found the way git's `enter_repo`
//! finds a local remote: `/x/origin` and `file:///x/origin` reach
//! `/x/origin.git`; git ignores the host of a `file://` URL).
//!
//! Must NOT: spawn git (callers pass the config they read).

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
            // git drops the host: `file://localhost/x` and `file://h/x` are `/x`.
            let path = rest.find('/').map_or(rest, |i| &rest[i..]);
            return local_key(&base.join(path));
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

/// `Some` when `p` is a checkout (`p/.git`, a directory or a `gitdir:`
/// file) or a git dir itself (a `HEAD` file plus objects of its own or a
/// `commondir` pointing at them); keyed by the common dir.
fn repo_key(p: &Path) -> Option<Key> {
    let p = std::fs::canonicalize(p).ok()?;
    let is_git_dir = |d: &Path| {
        d.join("HEAD").is_file() && (d.join("objects").is_dir() || d.join("commondir").is_file())
    };
    let dot_git = p.join(".git");
    let from_dot_git = if dot_git.is_dir() {
        Some(dot_git)
    } else {
        std::fs::read_to_string(&dot_git).ok().and_then(|text| {
            let target = text.trim().strip_prefix("gitdir:")?.trim().to_string();
            Some(p.join(target))
        })
    };
    let gitdir = from_dot_git.or_else(|| is_git_dir(&p).then(|| p.clone()))?;
    let gitdir = std::fs::canonicalize(gitdir).ok()?;
    let common = std::fs::read_to_string(gitdir.join("commondir"))
        .ok()
        .and_then(|c| std::fs::canonicalize(gitdir.join(c.trim())).ok())
        .unwrap_or(gitdir);
    Some(Key::Local(common))
}

#[cfg(test)]
mod tests;
