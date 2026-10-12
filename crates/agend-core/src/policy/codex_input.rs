//! Admission for the one operator-approved Codex CLI version.
//! Thread receipt attribution is a separate, persistent daemon decision.
use alloc::string::String;
pub const APPROVED_CLI_VERSION: &str = "codex-cli 0.159.3";
#[derive(Clone, Debug, Default)]
pub struct CodexInputPolicy {
    verification_instance: Option<String>,
    approved_versions: bool,
}
impl CodexInputPolicy {
    /// Production admission approved after the real U17 run on 0.159.3.
    pub fn approved() -> Self {
        Self {
            approved_versions: true,
            verification_instance: None,
        }
    }
    /// Diagnostic-only admission scoped to one explicit verification instance.
    pub fn for_u17_verification(instance: String) -> Self {
        Self {
            verification_instance: Some(instance),
            approved_versions: false,
        }
    }
    pub fn allows_instance(&self, instance: &str) -> bool {
        !instance.is_empty() && self.verification_instance.as_deref() == Some(instance)
    }
    pub fn allows_version(&self, output: &str) -> bool {
        self.approved_versions && output.trim() == APPROVED_CLI_VERSION
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_requires_the_exact_approved_cli_version() {
        let policy = CodexInputPolicy::approved();
        assert!(policy.allows_version("codex-cli 0.159.3\n"));
        for version in [
            "",
            "0.159.3",
            "codex-cli 0.159.30",
            "codex-cli 0.159.4",
            "codex-cli 0.158.0",
            "codex-cli 0.159.3-dev",
            "codex-cli 0.159.3\nother",
        ] {
            assert!(!policy.allows_version(version), "{version:?}");
        }
        assert!(!policy.allows_instance("codex"));
        assert!(!CodexInputPolicy::default().allows_version("codex-cli 0.159.3"));
    }
    #[test]
    fn verification_is_scoped_to_one_nonempty_instance() {
        let policy = CodexInputPolicy::for_u17_verification("codex-probe".into());
        assert!(policy.allows_instance("codex-probe"));
        for id in ["codex-probe-2", "other", ""] {
            assert!(!policy.allows_instance(id));
        }
        assert!(!policy.allows_version("codex-cli 0.159.3"));
        assert!(!CodexInputPolicy::for_u17_verification(String::new()).allows_instance(""));
    }
}
