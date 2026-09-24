//! Protected-ref check: refuses agent writes to main/master (and the refs the
//! snapshot adds) via `update-ref`, `push`, `push .`, `fetch <src>:<dst>`,
//! `branch -f`, `tag`. v1 let a bound agent's `update-ref` through
//! (`classify.rs:752-759`); here there is no exception for bound agents.
//!
//! Must NOT: have exceptions for bound agents.

/// Branches that are always protected.
pub const BUILTIN: [&str; 2] = ["main", "master"];

/// The protected-ref set for one agent: the built-ins plus the snapshot's
/// `protected_refs`.
#[derive(Debug, Clone)]
pub struct ProtectedRefs {
    patterns: Vec<String>,
}

impl ProtectedRefs {
    pub fn new(extra: &[String]) -> ProtectedRefs {
        let patterns = BUILTIN
            .iter()
            .map(|s| s.to_string())
            .chain(extra.iter().map(|s| s.trim().to_string()))
            .map(|p| full_pattern(&p))
            .collect();
        ProtectedRefs { patterns }
    }

    /// Whether the fully qualified `full_ref` (e.g. `refs/heads/main`) is
    /// protected.
    pub fn is_protected(&self, full_ref: &str) -> bool {
        self.patterns.iter().any(|p| match p.strip_suffix('*') {
            Some(prefix) => full_ref.starts_with(prefix),
            None => full_ref == p,
        })
    }

    /// Whether a ref as typed on a command line may name a protected ref.
    /// Unqualified names are checked as a branch, a tag and the literal name
    /// (so `update-ref main` is caught too).
    pub fn names_protected(&self, name: &str) -> bool {
        candidates(name).iter().any(|r| self.is_protected(r))
    }
}

/// `main` → `refs/heads/main`; `refs/...` and globs over `refs/` unchanged.
fn full_pattern(p: &str) -> String {
    if p.starts_with("refs/") {
        p.to_string()
    } else {
        format!("refs/heads/{p}")
    }
}

/// Fully qualified refs a command-line ref name may resolve to.
pub fn candidates(name: &str) -> Vec<String> {
    let name = name.trim_start_matches('+');
    if name.starts_with("refs/") {
        return vec![name.to_string()];
    }
    vec![
        format!("refs/heads/{name}"),
        format!("refs/tags/{name}"),
        name.to_string(),
    ]
}

/// Destination of one refspec (`src:dst`, `+src:dst`, `src`, `:dst`), or
/// `None` when it names no destination (`src` with an implicit, same-named
/// destination returns `src`).
pub fn refspec_dst(spec: &str) -> Option<&str> {
    let spec = spec.strip_prefix('+').unwrap_or(spec);
    let dst = match spec.split_once(':') {
        Some((_, dst)) => dst,
        None => spec,
    };
    (!dst.is_empty()).then_some(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_and_extras_are_protected() {
        let p = ProtectedRefs::new(&["release/*".into(), "refs/tags/v1".into()]);
        for r in [
            "refs/heads/main",
            "refs/heads/master",
            "refs/heads/release/2026",
            "refs/tags/v1",
        ] {
            assert!(p.is_protected(r), "{r}");
        }
        for r in [
            "refs/heads/agend/t-1/x",
            "refs/heads/mainline",
            "refs/tags/v2",
        ] {
            assert!(!p.is_protected(r), "{r}");
        }
    }

    #[test]
    fn command_line_names_are_checked_in_every_namespace() {
        let p = ProtectedRefs::new(&[]);
        for n in ["main", "+main", "refs/heads/master"] {
            assert!(p.names_protected(n), "{n}");
        }
        assert!(!p.names_protected("agend/t-1/x"));
        assert!(!p.names_protected("refs/remotes/origin/main"));
    }

    #[test]
    fn refspec_destinations() {
        assert_eq!(refspec_dst("HEAD:main"), Some("main"));
        assert_eq!(refspec_dst("+a:refs/heads/b"), Some("refs/heads/b"));
        assert_eq!(refspec_dst("main"), Some("main"));
        assert_eq!(refspec_dst(":main"), Some("main"));
        assert_eq!(refspec_dst("a:"), None);
    }
}
