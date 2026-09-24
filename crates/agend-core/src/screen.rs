//! Classify known hard gates in a holder's rendered screen. Rules are data and
//! must be backed by versioned prompt evidence; this crate never sends keys.
//!
//! Must NOT: decide busy/idle or perform screen I/O.

use crate::model::Backend;
use crate::protocol::holder::ControlKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardGateKind {
    UsageLimit,
    PermissionPrompt,
    RateLimit,
    AuthenticationError,
    ContextFull,
    StartupMenu,
    UpdateMenu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRule {
    pub backend: Backend,
    pub kind: HardGateKind,
    pub pattern: &'static str,
    /// Only the holder's deliberately small control-key vocabulary is allowed.
    pub suggested_key: Option<ControlKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenMatch {
    pub kind: HardGateKind,
    pub pattern: &'static str,
    pub suggested_key: Option<ControlKey>,
}

/// Rules whose exact prompt fragments were observed in the backend spikes.
/// Other hard-gate patterns are supplied with their fixtures as each backend
/// driver is implemented; unverified text is never guessed here.
pub const SCREEN_RULES: &[ScreenRule] = &[
    ScreenRule {
        backend: Backend::Codex,
        kind: HardGateKind::StartupMenu,
        pattern: "Trust this folder? Codex can read, edit, and run files here",
        suggested_key: None,
    },
    ScreenRule {
        backend: Backend::Claude,
        kind: HardGateKind::StartupMenu,
        pattern: "Quick safety check: Is this a project you created or one you trust?",
        suggested_key: None,
    },
    ScreenRule {
        backend: Backend::Claude,
        kind: HardGateKind::StartupMenu,
        pattern: "Use this and all future MCP servers in this project",
        suggested_key: None,
    },
];

/// Match only rules for the current backend. ASCII case is ignored because
/// ANSI rendering and theme changes can alter capitalization in a capture.
pub fn classify(backend: Backend, screen: &str, rules: &[ScreenRule]) -> Option<ScreenMatch> {
    rules
        .iter()
        .find(|rule| {
            rule.backend == backend && contains_ascii_case_insensitive(screen, rule.pattern)
        })
        .map(|rule| ScreenMatch {
            kind: rule.kind,
            pattern: rule.pattern,
            suggested_key: rule.suggested_key,
        })
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    !needle.is_empty()
        && haystack.windows(needle.len()).any(|window| {
            window
                .iter()
                .zip(needle)
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_folder_trust_fixture_is_a_hard_gate() {
        let screen = include_str!("../tests/fixtures/screens/codex-folder-access.txt");
        let found = classify(Backend::Codex, screen, SCREEN_RULES).unwrap();
        assert_eq!(found.kind, HardGateKind::StartupMenu);
        assert_eq!(found.suggested_key, None);
    }

    #[test]
    fn claude_trust_fixture_is_not_mistaken_for_codex() {
        let screen = include_str!("../tests/fixtures/screens/claude-workspace-trust.txt");
        assert_eq!(
            classify(Backend::Claude, screen, SCREEN_RULES)
                .unwrap()
                .kind,
            HardGateKind::StartupMenu
        );
        assert_eq!(classify(Backend::Codex, screen, SCREEN_RULES), None);
    }

    #[test]
    fn claude_mcp_trust_fixture_is_a_hard_gate() {
        let screen = include_str!("../tests/fixtures/screens/claude-mcp-trust.txt");
        assert_eq!(
            classify(Backend::Claude, screen, SCREEN_RULES)
                .unwrap()
                .kind,
            HardGateKind::StartupMenu
        );
    }

    #[test]
    fn unrelated_screen_is_not_classified() {
        assert_eq!(
            classify(Backend::Claude, "Ready for input", SCREEN_RULES),
            None
        );
    }

    #[test]
    fn classifier_supports_all_hard_gate_kinds_from_rule_data() {
        let rule = ScreenRule {
            backend: Backend::Opencode,
            kind: HardGateKind::UsageLimit,
            pattern: "captured limit marker",
            suggested_key: None,
        };
        assert_eq!(
            classify(Backend::Opencode, "CAPTURED LIMIT MARKER", &[rule])
                .unwrap()
                .kind,
            HardGateKind::UsageLimit
        );
    }
}
