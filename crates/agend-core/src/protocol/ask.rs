//! Needs-you asks (D35) and the context recap shown with them (D37).
//!
//! An ask is a conversation, not only a multiple-choice prompt: an agent asks
//! (optionally offering options), the operator answers by choosing an option
//! or in free text from the TUI or Telegram, the agent may follow up, and the
//! thread ends with a resolution. All of it travels in client protocol v1 as
//! additive fields and variants.
//!
//! The recap tells the operator where they are when switching to an item:
//! the feature goal, the decisions so far, what is being asked and what
//! happens next. Core only defines the type; the daemon fills it in
//! (gate 11).
//!
//! Must NOT: generate recap text or decide who answers.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

/// One needs-you conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskThread {
    pub ask_id: String,
    pub task_id: Option<String>,
    pub entries: Vec<AskEntry>,
}

impl AskThread {
    pub fn is_resolved(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry, AskEntry::Resolution { .. }))
    }

    /// Whether `reply` answers the thread now: it is open, its last entry is
    /// a question or follow-up waiting for an answer, and a choice names one
    /// of the options offered there. Free text is always an answer.
    pub fn accepts(&self, reply: &AskReply) -> bool {
        if self.is_resolved() {
            return false;
        }
        let options = match self.entries.last() {
            Some(AskEntry::Question { options, .. } | AskEntry::FollowUp { options, .. }) => {
                options
            }
            _ => return false,
        };
        match reply {
            AskReply::Choice { option } => options.contains(option),
            AskReply::Text { text } => !text.trim().is_empty(),
            AskReply::Unknown => false,
        }
    }
}

/// One turn of an ask thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub enum AskEntry {
    Question {
        from: String,
        text: String,
        #[serde(default)]
        options: Vec<String>,
    },
    Answer {
        from: String,
        source: AnswerSource,
        reply: AskReply,
    },
    FollowUp {
        from: String,
        text: String,
        #[serde(default)]
        options: Vec<String>,
    },
    Resolution {
        from: String,
        summary: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum AskReply {
    Choice {
        option: String,
    },
    Text {
        text: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerSource {
    Tui,
    Telegram,
    Cli,
    #[serde(other)]
    Unknown,
}

/// Context shown with a needs-you item (D37).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRecap {
    pub goal: String,
    #[serde(default)]
    pub decisions: Vec<String>,
    pub asking: String,
    pub next: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn thread(entries: Vec<AskEntry>) -> AskThread {
        AskThread {
            ask_id: "A-1".into(),
            task_id: Some("T-1".into()),
            entries,
        }
    }

    fn question(options: &[&str]) -> AskEntry {
        AskEntry::Question {
            from: "dev-1".into(),
            text: "Which storage?".into(),
            options: options.iter().map(|option| (*option).into()).collect(),
        }
    }

    fn choice(option: &str) -> AskReply {
        AskReply::Choice {
            option: option.into(),
        }
    }

    fn text(text: &str) -> AskReply {
        AskReply::Text { text: text.into() }
    }

    #[test]
    fn open_question_accepts_an_offered_choice_or_free_text() {
        let ask = thread(vec![question(&["sqlite", "files"])]);
        assert!(ask.accepts(&choice("sqlite")));
        assert!(!ask.accepts(&choice("postgres")));
        assert!(ask.accepts(&text("sqlite, but keep a files export")));
        assert!(!ask.accepts(&text("  ")));
        assert!(thread(vec![question(&[])]).accepts(&text("either")));
    }

    #[test]
    fn multi_turn_thread_waits_for_a_follow_up_and_ends_with_a_resolution() {
        let mut ask = thread(vec![
            question(&["sqlite", "files"]),
            AskEntry::Answer {
                from: "operator".into(),
                source: AnswerSource::Telegram,
                reply: text("depends on size"),
            },
        ]);
        assert!(!ask.accepts(&text("again")), "no question is waiting");
        ask.entries.push(AskEntry::FollowUp {
            from: "dev-1".into(),
            text: "About 2 GB. Still sqlite?".into(),
            options: vec!["yes".into(), "no".into()],
        });
        assert!(ask.accepts(&choice("yes")));
        ask.entries.push(AskEntry::Answer {
            from: "operator".into(),
            source: AnswerSource::Tui,
            reply: choice("yes"),
        });
        ask.entries.push(AskEntry::Resolution {
            from: "dev-1".into(),
            summary: "Use sqlite.".into(),
        });
        assert!(ask.is_resolved());
        ask.entries.push(question(&["a"]));
        assert!(
            !ask.accepts(&choice("a")),
            "a resolved thread takes no answer"
        );
    }
}
