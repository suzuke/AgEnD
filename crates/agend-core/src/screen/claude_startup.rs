//! Complete recorded 2.1.284 frames, with only the Ready suggestion variable.
//! This only identifies startup screens; hooks and runtime decide busy/idle.
use crate::protocol::holder::ControlKey;
/// Evidence label only; recognition still requires the complete recorded frame.
pub const RECORDED_VERSION: &str = "2.1.284";
const ROWS: u16 = 24;

pub fn recorded_dimensions() -> alloc::vec::Vec<(u16, u16)> {
    let mut dimensions = alloc::vec::Vec::new();
    for rule in RULES {
        let size = (rule.columns, ROWS);
        if !dimensions.contains(&size) {
            dimensions.push(size);
        }
    }
    dimensions
}

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

/// Unknown dimensions/content stay manual. Only the text inside one complete
/// single-row Ready suggestion may vary; its position and the rest must match.
pub fn classify(screen: &str, workspace: &str, columns: u16, rows: u16) -> Option<Prompt> {
    if rows != ROWS || !workspace.starts_with('/') || workspace.chars().any(char::is_control) {
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
        let matches = if rule.prompt == Prompt::Ready {
            ready_matches(screen, &expected, columns)
        } else {
            screen.split_whitespace().eq(expected.split_whitespace())
        };
        (own_path && matches).then_some(rule.prompt)
    })
}

fn ready_matches(screen: &str, expected: &str, columns: u16) -> bool {
    let Some((before, after, row)) = ready_parts(screen, columns) else {
        return false;
    };
    let Some((expected_before, expected_after, expected_row)) = ready_parts(expected, columns)
    else {
        return false;
    };
    row == expected_row
        && before
            .split_whitespace()
            .eq(expected_before.split_whitespace())
        && after
            .split_whitespace()
            .eq(expected_after.split_whitespace())
}

// Keep the variable field inside one bounded, complete input-placeholder row.
// No menu classifier or startup key authorization uses this normalization.
fn ready_parts(screen: &str, columns: u16) -> Option<(&str, &str, usize)> {
    if screen.match_indices("Try \"").count() != 1 {
        return None;
    }
    let mut offset = 0;
    for (row, part) in screen.split_inclusive('\n').enumerate() {
        let line = part.strip_suffix('\n').unwrap_or(part);
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('❯') {
            if !rest.starts_with([' ', '\u{a0}']) {
                return None;
            }
            let rest = rest.trim_start_matches([' ', '\u{a0}']);
            let hint = rest.strip_prefix("Try \"")?.strip_suffix('"')?;
            if line.chars().any(char::is_control)
                || trimmed.chars().count() > usize::from(columns)
                || hint.trim().is_empty()
                || hint.contains('"')
                || hint.contains(['\u{2028}', '\u{2029}'])
            {
                return None;
            }
            return Some((&screen[..offset], &screen[offset + part.len()..], row));
        }
        offset += part.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, string::String, vec, vec::Vec};

    #[test]
    fn ready_suggestion_alone_varies_at_both_widths_without_authorizing_a_key() {
        for rule in RULES.iter().filter(|r| r.prompt == Prompt::Ready) {
            assert!(
                rule.frame
                    .contains(&format!("Claude Code v{RECORDED_VERSION}"))
            );
            let screen = rule.frame.replace(PATH, "/private/work space");
            let suggestion = screen
                .lines()
                .find(|l| l.contains("Try \""))
                .unwrap()
                .trim();
            for content in ["fix typecheck errors", "fix lint errors", "新的建議 é", "a"] {
                let changed = screen.replace(suggestion, &format!("❯\u{a0}Try \"{content}\""));
                let prompt = classify(&changed, "/private/work space", rule.columns, 24);
                assert_eq!(prompt, Some(Prompt::Ready));
                assert_eq!(prompt.unwrap().key(), None);
            }
        }
        for screen in [
            include_str!(
                "../../tests/fixtures/screens/claude-2.1.284-main-100x24-v5-typecheck.txt"
            ),
            include_str!("../../tests/fixtures/screens/claude-2.1.284-main-100x24-v5-lint.txt"),
        ] {
            assert_eq!(
                classify(&screen.replace(PATH, "/workspace"), "/workspace", 100, 24),
                Some(Prompt::Ready)
            );
        }
    }

    #[test]
    fn variable_ready_rejects_malformed_rows_and_every_other_content_change() {
        for rule in RULES.iter().filter(|r| r.prompt == Prompt::Ready) {
            let screen = rule.frame.replace(PATH, "/workspace");
            let suggestion = screen
                .lines()
                .find(|l| l.contains("Try \""))
                .unwrap()
                .trim();
            let mut changes: Vec<String> = vec![
                screen.replace("2.1.284", "2.1.999"),
                screen.replace("/workspace", "/foreign"),
                screen.replace("server:agend", "server:foreign"),
                screen.replace("Claude Code", "Unknown Code"),
                screen.replace("bypass permissions", "confirm permissions"),
                screen.clone() + "\n❯ Extra menu",
            ];
            for malformed in [
                "❯ Try \"\"",
                "❯ Try \"   \"",
                "❯ Try \"unfinished",
                "❯ Try \"a\" extra",
                "❯ Try \"a\"b\"",
                "❯ Try \"a\nb\"",
                "❯ Try \"a\rb\"",
                "❯ Try \"a\tb\"",
                "❯ Try \"a\u{1b}b\"",
                "❯ Try \"a\u{2028}b\"",
                "❯ Try \"a\u{2029}b\"",
                "❯Try \"a\"",
                "❯\tTry \"a\"",
                "Try \"a\"",
                "❯ Try \"a\"\n❯ Try \"b\"",
            ] {
                changes.push(screen.replace(suggestion, malformed));
            }
            changes.push(screen.replace(
                suggestion,
                &format!("❯ Try \"{}\"", "a".repeat(usize::from(rule.columns))),
            ));
            changes.push(screen.replace(suggestion, &format!("\n{suggestion}")));
            for changed in changes {
                assert_eq!(
                    classify(&changed, "/workspace", rule.columns, 24),
                    None,
                    "{changed}"
                );
            }
            assert_eq!(classify(&screen, "/workspace", rule.columns, 23), None);
            assert_eq!(classify(&screen, "/workspace", 120, 24), None);
            assert_eq!(classify(suggestion, "/workspace", rule.columns, 24), None);
        }
    }
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
