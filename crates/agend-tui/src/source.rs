//! The data-source seam. Screens read a [`Fleet`], which is built only from
//! what a [`Source`] delivers, and act only through `Source`; they never know
//! which source they have. Events, asks and replies are client protocol v1
//! types (`agend_core::protocol`).
//!
//! Protocol v1 has no request that lists teams, tasks or agents, and no
//! structured agent state or task stages, so a source hands over a
//! [`Catalog`] when it connects. The catalog types are TUI-local until gate 8
//! adds them to the protocol (see docs/gates/gate-11-tui.md, "待你追認").
//!
//! Must NOT: talk to a socket (gate 11 proper implements `Source` on
//! `agend-client`), or invent state the source did not report.

pub mod scripted;

use std::collections::BTreeMap;

use agend_core::model::Backend;
use agend_core::pipeline::stage::StageKind;
use agend_core::policy::attention::{AttentionItem, order};
use agend_core::protocol::ask::{AskEntry, AskReply};
use agend_core::protocol::client::{AttentionRequiredData, DaemonEvent, EventData};

/// Agent state exactly as the source reports it (never derived here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Working,
    Idle,
    NeedsYou,
    Stuck,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageState {
    Done,
    Running,
    NotStarted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageInfo {
    pub name: String,
    pub kind: StageKind,
    pub state: StageState,
    /// The agent working on this stage, if any.
    pub agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInfo {
    pub id: String,
    pub team_id: String,
    pub title: String,
    /// Shown only in Task Detail.
    pub repo: Option<String>,
    /// The task holder.
    pub holder: Option<String>,
    pub stages: Vec<StageInfo>,
}

impl TaskInfo {
    /// Index of the first stage that is not done; `None` when all are done.
    pub fn current_stage(&self) -> Option<usize> {
        self.stages.iter().position(|s| s.state != StageState::Done)
    }

    pub fn is_done(&self) -> bool {
        self.current_stage().is_none()
    }

    pub fn stages_done(&self) -> usize {
        self.stages
            .iter()
            .filter(|s| s.state == StageState::Done)
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub id: String,
    pub team_id: String,
    pub backend: Backend,
    pub state: AgentState,
    pub task_id: Option<String>,
}

/// Teams, tasks and agents: what protocol v1 cannot list yet (gap for gate 8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub teams: Vec<String>,
    pub tasks: Vec<TaskInfo>,
    pub agents: Vec<AgentInfo>,
    /// Task id → what happens to that task if its needs-you item is left
    /// alone (DEMO-01 §4B). `attention_required` has no such field, so the
    /// demo catalog carries fixed text (gap G1 for gate 8).
    pub if_ignored: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// The daemon cannot be reached; the TUI shows the disconnected state
    /// and keeps trying to connect.
    Disconnected(String),
    /// The daemon answered with an error frame; the connection stays up.
    Rejected { code: String, message: String },
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Disconnected(reason) => write!(f, "disconnected: {reason}"),
            SourceError::Rejected { code, message } => write!(f, "{code}: {message}"),
        }
    }
}

/// Where the screens' data comes from.
pub trait Source {
    /// (Re)connects. After success, `poll` replays every event from the
    /// beginning, so the caller starts a fresh [`Fleet`] from the catalog.
    fn connect(&mut self) -> Result<Catalog, SourceError>;
    /// Events that arrived since the last call; never blocks for long.
    fn poll(&mut self) -> Result<Vec<EventData>, SourceError>;
    /// A snapshot of one agent's terminal screen (empty: nothing to show).
    fn terminal(&mut self, instance_id: &str) -> Result<String, SourceError>;
    /// The operator's reply to a needs-you ask (D35).
    fn answer(&mut self, ask_id: &str, reply: AskReply) -> Result<(), SourceError>;
}

/// One `attention_required` event and the latest state of its ask thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attention {
    pub event_id: u64,
    pub data: AttentionRequiredData,
}

impl Attention {
    /// Stable id: the ask id, or the event id for items that are not asks.
    pub fn key(&self) -> String {
        match &self.data.ask {
            Some(ask) => ask.ask_id.clone(),
            None => format!("event-{}", self.event_id),
        }
    }

    /// What "read" is recorded against: the key plus how many questions the
    /// thread has, so a follow-up counts as new again.
    pub fn read_key(&self) -> String {
        let questions = self.data.ask.as_ref().map_or(0, |ask| {
            ask.entries
                .iter()
                .filter(|e| matches!(e, AskEntry::Question { .. } | AskEntry::FollowUp { .. }))
                .count()
        });
        format!("{}#{questions}", self.key())
    }

    /// Whether the item still needs the operator. An ask needs you while a
    /// question waits for an answer; once answered (or resolved) it leaves
    /// "needs you" until the agent follows up. Protocol v1 has no event that
    /// clears a non-ask item, so those stay.
    pub fn waiting(&self) -> bool {
        match &self.data.ask {
            None => true,
            Some(ask) => {
                !ask.is_resolved()
                    && matches!(
                        ask.entries.last(),
                        Some(AskEntry::Question { .. } | AskEntry::FollowUp { .. })
                    )
            }
        }
    }

    /// The question waiting now (or the reason for a non-ask item) and its options.
    pub fn question(&self) -> (&str, &[String]) {
        let Some(ask) = &self.data.ask else {
            return (&self.data.reason, &[]);
        };
        let open = ask.entries.iter().rev().find_map(|entry| match entry {
            AskEntry::Question { text, options, .. } | AskEntry::FollowUp { text, options, .. } => {
                Some((text.as_str(), options.as_slice()))
            }
            _ => None,
        });
        open.unwrap_or((&self.data.reason, &[]))
    }

    /// Who asked: the author of the first question.
    pub fn asker(&self) -> Option<&str> {
        self.data
            .ask
            .as_ref()?
            .entries
            .iter()
            .find_map(|e| match e {
                AskEntry::Question { from, .. } => Some(from.as_str()),
                _ => None,
            })
    }

    pub fn task_id(&self) -> Option<&str> {
        self.data
            .task_id
            .as_deref()
            .or_else(|| self.data.ask.as_ref()?.task_id.as_deref())
    }
}

/// Everything the screens show, rebuilt from a catalog plus the event log.
#[derive(Debug, Clone, Default)]
pub struct Fleet {
    pub catalog: Catalog,
    attention: Vec<Attention>,
    events: Vec<EventData>,
}

impl Fleet {
    pub fn new(catalog: Catalog) -> Fleet {
        Fleet {
            catalog,
            ..Fleet::default()
        }
    }

    pub fn apply(&mut self, event: EventData) {
        match &event.event {
            DaemonEvent::AttentionRequired { data } => self.attention.push(Attention {
                event_id: event.event_id,
                data: data.clone(),
            }),
            DaemonEvent::AskUpdated { data } => {
                for item in &mut self.attention {
                    if let Some(ask) = &mut item.data.ask
                        && ask.ask_id == data.ask_id
                    {
                        *ask = data.clone();
                    }
                }
            }
            _ => {}
        }
        self.events.push(event);
    }

    /// Items that need the operator, in the D36 order. Protocol v1 carries
    /// neither how much work an item unblocks nor when it started waiting,
    /// so every item counts as unblocking nothing and the event id stands in
    /// for the waiting time (oldest first); gap for gate 8.
    pub fn needs_you(&self) -> Vec<&Attention> {
        let mut items: Vec<AttentionItem> = self
            .attention
            .iter()
            .filter(|a| a.waiting())
            .map(|a| AttentionItem {
                id: a.key(),
                unblocks: 0,
                waiting_since_unix_ms: a.event_id,
            })
            .collect();
        order(&mut items);
        items
            .iter()
            .filter_map(|item| self.attention(&item.id))
            .collect()
    }

    pub fn attention(&self, key: &str) -> Option<&Attention> {
        self.attention.iter().find(|a| a.key() == key)
    }

    pub fn events(&self) -> &[EventData] {
        &self.events
    }

    pub fn task(&self, id: &str) -> Option<&TaskInfo> {
        self.catalog.tasks.iter().find(|t| t.id == id)
    }

    pub fn agent(&self, id: &str) -> Option<&AgentInfo> {
        self.catalog.agents.iter().find(|a| a.id == id)
    }

    pub fn tasks_of<'a>(&'a self, team: &'a str) -> impl Iterator<Item = &'a TaskInfo> + 'a {
        self.catalog.tasks.iter().filter(move |t| t.team_id == team)
    }

    pub fn agents_of<'a>(&'a self, team: &'a str) -> impl Iterator<Item = &'a AgentInfo> + 'a {
        self.catalog
            .agents
            .iter()
            .filter(move |a| a.team_id == team)
    }

    /// The waiting needs-you item for a task, if any.
    pub fn needs_you_for_task(&self, task_id: &str) -> Option<&Attention> {
        self.needs_you()
            .into_iter()
            .find(|a| a.task_id() == Some(task_id))
    }
}

/// The task an event is about, if it names one.
pub fn event_task(event: &DaemonEvent) -> Option<&str> {
    match event {
        DaemonEvent::TaskChanged { data } => Some(&data.task_id),
        DaemonEvent::AttentionRequired { data } => data
            .task_id
            .as_deref()
            .or_else(|| data.ask.as_ref()?.task_id.as_deref()),
        DaemonEvent::AskUpdated { data } => data.task_id.as_deref(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::protocol::ask::{AnswerSource, AskThread};

    fn ask_event(id: u64, ask_id: &str) -> EventData {
        EventData {
            event_id: id,
            event: DaemonEvent::AttentionRequired {
                data: AttentionRequiredData {
                    reason: "ask".into(),
                    task_id: None,
                    ask: Some(AskThread {
                        ask_id: ask_id.into(),
                        task_id: Some("T-1".into()),
                        entries: vec![AskEntry::Question {
                            from: "dev-1".into(),
                            text: "Which?".into(),
                            options: vec!["a".into()],
                        }],
                    }),
                    recap: None,
                },
            },
        }
    }

    #[test]
    fn answering_an_ask_removes_it_and_a_follow_up_brings_it_back() {
        let mut fleet = Fleet::default();
        fleet.apply(ask_event(1, "A-1"));
        assert_eq!(fleet.needs_you().len(), 1);
        let mut thread = fleet.attention("A-1").unwrap().data.ask.clone().unwrap();
        thread.entries.push(AskEntry::Answer {
            from: "operator".into(),
            source: AnswerSource::Tui,
            reply: AskReply::Choice { option: "a".into() },
        });
        fleet.apply(EventData {
            event_id: 2,
            event: DaemonEvent::AskUpdated {
                data: thread.clone(),
            },
        });
        assert!(
            fleet.needs_you().is_empty(),
            "answered ask leaves needs-you"
        );
        thread.entries.push(AskEntry::FollowUp {
            from: "dev-1".into(),
            text: "Sure?".into(),
            options: vec![],
        });
        fleet.apply(EventData {
            event_id: 3,
            event: DaemonEvent::AskUpdated { data: thread },
        });
        assert_eq!(fleet.needs_you()[0].question().0, "Sure?");
        assert_eq!(fleet.needs_you()[0].task_id(), Some("T-1"));
    }

    #[test]
    fn needs_you_is_oldest_first_by_event_id() {
        let mut fleet = Fleet::default();
        fleet.apply(ask_event(7, "A-late"));
        fleet.apply(ask_event(3, "A-early"));
        let keys: Vec<String> = fleet.needs_you().iter().map(|a| a.key()).collect();
        assert_eq!(keys, ["A-early", "A-late"]);
    }
}
