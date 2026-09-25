//! Protected refs: `main` and `master`, plus the refs the binding snapshot
//! adds. The `reference-transaction` and `pre-push` hooks (`hook`) refuse
//! every agent write to them, whatever the command; there is no exception
//! for bound agents (v1 let a bound agent's `update-ref` through).
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
}

/// `main` → `refs/heads/main`; `refs/...` and globs over `refs/` unchanged.
fn full_pattern(p: &str) -> String {
    if p.starts_with("refs/") {
        p.to_string()
    } else {
        format!("refs/heads/{p}")
    }
}

#[cfg(test)]
mod tests;
