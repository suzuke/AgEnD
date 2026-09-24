//! The six basic stage kinds and their parameters (plan §4.5.0, D19, D20).
//!
//! | kind | parameters |
//! |---|---|
//! | `work` | role, instructions, output (branch or result) |
//! | `command` | command, timeout; runs in a temporary detached worktree at the head |
//! | `approval` | approver (human or role), count, whether bound to head |
//! | `submit` | forge |
//! | `merge` | gate: checks passed and approved head == current head |
//! | `fanout` | child source (from work output or listed), join: all / first / pick |
//!
//! Every stage also takes `timeout` plus a timeout action (notify, reassign,
//! cancel).
//!
//! Must NOT: know how a stage is executed.

/// Basic stage kind. The string form is the `kind` value in workflow TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    Work,
    Command,
    Approval,
    Submit,
    Merge,
    Fanout,
}

impl StageKind {
    pub const ALL: [StageKind; 6] = [
        StageKind::Work,
        StageKind::Command,
        StageKind::Approval,
        StageKind::Submit,
        StageKind::Merge,
        StageKind::Fanout,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            StageKind::Work => "work",
            StageKind::Command => "command",
            StageKind::Approval => "approval",
            StageKind::Submit => "submit",
            StageKind::Merge => "merge",
            StageKind::Fanout => "fanout",
        }
    }

    /// Parses a workflow `kind`. Unknown kinds are rejected at save time (D19).
    pub fn parse(s: &str) -> Option<StageKind> {
        StageKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// How a `fanout` stage joins its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanoutJoin {
    /// Done when every child is done.
    All,
    /// Done when the first child is done.
    First,
    /// A following `approval` picks the winner; the other children are cancelled.
    Pick,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_kinds_round_trip() {
        for k in StageKind::ALL {
            assert_eq!(StageKind::parse(k.as_str()), Some(k));
        }
    }

    #[test]
    fn retired_checks_kind_is_not_a_stage() {
        // D19/D20: checks are expressed as a `command` stage, not a kind of their own.
        assert_eq!(StageKind::parse("checks"), None);
    }
}
