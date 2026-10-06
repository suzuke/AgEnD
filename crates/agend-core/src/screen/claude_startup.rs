//! Complete recorded 2.1.284 frames, not fragment-based key authorization.
//! This only identifies startup screens; hooks and runtime decide busy/idle.
use crate::protocol::holder::ControlKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    TrustNo,
    TrustYes,
    Development,
    Ready,
}
impl Prompt {
    pub fn key(self) -> Option<ControlKey> {
        match self {
            Self::TrustNo => Some(ControlKey::Down),
            Self::TrustYes | Self::Development => Some(ControlKey::Enter),
            Self::Ready => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::TrustNo => "trust_no",
            Self::TrustYes => "trust_yes",
            Self::Development => "development",
            Self::Ready => "ready",
        }
    }
}
const PATH: &str = "<rec>/h1/workspace/g12-startup-capture";
struct Rule {
    columns: u16,
    prompt: Prompt,
    frame: &'static str,
}
macro_rules! rule {
    ($columns:literal, $prompt:ident, $file:literal) => {
        Rule {
            columns: $columns,
            prompt: Prompt::$prompt,
            frame: include_str!(concat!("../../tests/fixtures/screens/", $file)),
        }
    };
}
const RULES: &[Rule] = &[
    rule!(100, TrustNo, "claude-2.1.284-workspace-trust-100x24.txt"),
    rule!(140, TrustNo, "claude-2.1.284-workspace-trust-140x24.txt"),
    rule!(
        100,
        TrustYes,
        "claude-2.1.284-workspace-trust-selected-yes-100x24.txt"
    ),
    rule!(
        140,
        TrustYes,
        "claude-2.1.284-workspace-trust-selected-yes-140x24.txt"
    ),
    rule!(
        100,
        Development,
        "claude-2.1.284-development-channels-100x24.txt"
    ),
    rule!(
        140,
        Development,
        "claude-2.1.284-development-channels-140x24.txt"
    ),
    rule!(100, Ready, "claude-2.1.284-main-100x24-0.txt"),
    rule!(100, Ready, "claude-2.1.284-main-100x24-1.txt"),
    rule!(100, Ready, "claude-2.1.284-main-100x24-2.txt"),
    rule!(100, Ready, "claude-2.1.284-main-100x24-3.txt"),
    rule!(100, Ready, "claude-2.1.284-main-100x24-4.txt"),
    rule!(140, Ready, "claude-2.1.284-main-140x24-0.txt"),
    rule!(140, Ready, "claude-2.1.284-main-140x24-1.txt"),
];

/// Unknown dimensions/content stay manual. Whitespace normalization permits
/// PTY wrapping but never weakens the separately checked canonical path.
pub fn classify(screen: &str, workspace: &str, columns: u16, rows: u16) -> Option<Prompt> {
    if rows != 24 || !workspace.starts_with('/') || workspace.chars().any(char::is_control) {
        return None;
    }
    RULES.iter().find_map(|rule| {
        if rule.columns != columns {
            return None;
        }
        let own_path = match rule.prompt {
            Prompt::TrustNo | Prompt::TrustYes => {
                let mut lines = screen.lines().map(str::trim).filter(|l| !l.is_empty());
                let mut found = None;
                while let Some(line) = lines.next() {
                    if line == "Accessing workspace:" {
                        if found.is_some() {
                            return None;
                        }
                        found = lines.next();
                    }
                }
                found == Some(workspace)
            }
            Prompt::Ready => screen
                .lines()
                .filter(|l| !l.trim().is_empty())
                .nth(2)
                .and_then(|l| l.trim().strip_prefix("▝▝   ▝▝"))
                .is_some_and(|l| l.trim() == workspace),
            Prompt::Development => true,
        };
        let expected = rule.frame.replace(PATH, workspace);
        (own_path && screen.split_whitespace().eq(expected.split_whitespace()))
            .then_some(rule.prompt)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_recorded_frames_only_authorize_their_own_startup_action() {
        for rule in RULES {
            let screen = rule.frame.replace(PATH, "/private/work space");
            assert_eq!(
                classify(&screen, "/private/work space", rule.columns, 24),
                Some(rule.prompt)
            );
            assert_eq!(
                classify(&screen, "/private/workspace", rule.columns, 24),
                if rule.prompt == Prompt::Development {
                    Some(rule.prompt)
                } else {
                    None
                }
            );
            assert_eq!(
                classify(
                    &(screen.clone() + "\n❯ 2. Exit"),
                    "/private/work space",
                    rule.columns,
                    24
                ),
                None
            );
            assert_eq!(
                classify(&screen, "/private/work space", rule.columns, 23),
                None
            );
            assert_eq!(
                classify(
                    &screen.replace("2.1.284", "2.1.999"),
                    "/private/work space",
                    rule.columns,
                    24
                ),
                if rule.prompt == Prompt::Ready {
                    None
                } else {
                    Some(rule.prompt)
                }
            );
        }
        assert_eq!(classify("Ready for input", "/workspace", 100, 24), None);
        let development = RULES
            .iter()
            .find(|r| r.prompt == Prompt::Development)
            .unwrap();
        assert_eq!(
            classify(
                &development.frame.replace("server:agend", "server:ag end"),
                "/workspace",
                100,
                24
            ),
            None
        );
    }
}
