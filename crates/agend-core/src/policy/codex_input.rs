//! Codex input remains denied until U17 and its tested version are approved.
//! A diagnostic daemon may explicitly scope input to one verification instance.
use alloc::string::String;
#[derive(Clone, Debug, Default)]
pub struct CodexInputPolicy {
    verification_instance: Option<String>,
}
impl CodexInputPolicy {
    /// Diagnostic-only policy. The production daemon always uses Default.
    /// This does not register any approved CLI version or persist permission.
    pub fn for_u17_verification(instance: String) -> Self {
        Self {
            verification_instance: Some(instance),
        }
    }
    pub fn allows_instance(&self, instance: &str) -> bool {
        !instance.is_empty() && self.verification_instance.as_deref() == Some(instance)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_denies_every_unverified_instance() {
        let policy = CodexInputPolicy::default();
        for id in ["codex", "fake-codex", "other", ""] {
            assert!(!policy.allows_instance(id));
        }
    }
    #[test]
    fn verification_is_scoped_to_one_nonempty_instance() {
        let policy = CodexInputPolicy::for_u17_verification("codex-probe".into());
        assert!(policy.allows_instance("codex-probe"));
        assert!(!policy.allows_instance("codex-probe-2"));
        assert!(!policy.allows_instance("other"));
        assert!(!policy.allows_instance(""));
        assert!(!CodexInputPolicy::for_u17_verification(String::new()).allows_instance(""));
    }
}
