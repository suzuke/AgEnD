//! The data-source seam. Screens read a [`Fleet`], which is built only from
//! what a [`Source`] delivers, and act only through `Source`; they never know
//! which source they have. Events, asks and replies are client protocol
//! types (`agend_core::protocol`).
//!
//! A source hands over a [`Snapshot`] when it connects: the [`Catalog`]
//! (teams, tasks, agents; a TUI-local shape the screens were built on) and
//! the needs-you list at that moment. [`client::ClientSource`] fills it from
//! the daemon's fleet view and keeps it current from `instance_changed` /
//! `task_changed` (gate 11 B P1, P3: one source of truth, no replay);
//! [`scripted::ScriptedSource`] hands over a fixed demo catalog. The one
//! derived value is an agent's "needs you", which follows the current
//! needs-you list ([`Fleet::agent_state`], T18).
//!
//! Must NOT: talk to a socket (only `agend-client` does, through
//! [`client::ClientSource`]), or invent state the source did not report.

pub mod client;
pub mod scripted;

use std::collections::BTreeMap;

use agend_core::model::Backend;
use agend_core::pipeline::stage::StageKind;
use agend_core::policy::attention::{AttentionItem, order};
use agend_core::protocol::ask::{AskEntry, AskReply};
use agend_core::protocol::client::{
    AttentionAction, AttentionRequiredData, DaemonEvent, EventData,
};

/// Agent state as the source reports it; [`Fleet::agent_state`] adjusts
/// "needs you" to the current needs-you list. `Starting` and `Failed` come
/// from the daemon (gate 11 B P3); `NeedsYou` only from the scripted
/// catalog (the protocol derives it from the needs-you list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Starting,
    Working,
    Idle,
    NeedsYou,
    Stuck,
    Failed,
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
    /// `None` from the daemon until gate 10 adds stage kinds (gap G5).
    pub kind: Option<StageKind>,
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
    /// The daemon's status (`open`, `running`, `blocked`, `done`,
    /// `superseded`); what tells a task without stages done or not.
    pub status: String,
}

impl TaskInfo {
    /// Index of the first stage that is not done; `None` when all are done
    /// or the task has no stages.
    pub fn current_stage(&self) -> Option<usize> {
        self.stages.iter().position(|s| s.state != StageState::Done)
    }

    pub fn is_done(&self) -> bool {
        if self.stages.is_empty() {
            matches!(self.status.as_str(), "done" | "superseded")
        } else {
            self.current_stage().is_none()
        }
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

/// Teams, tasks and agents as the screens show them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub teams: Vec<String>,
    pub tasks: Vec<TaskInfo>,
    pub agents: Vec<AgentInfo>,
    /// Task id → what happens to that task if its needs-you item is left
    /// alone (DEMO-01 §4B), for items without the protocol's `if_ignored`
    /// (the scripted demo's fixed text).
    pub if_ignored: BTreeMap<String, String>,
}

/// What a source hands over when it connects.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub catalog: Catalog,
    /// The needs-you list now (the fleet view's; empty for a source that
    /// replays its events instead).
    pub attention: Vec<AttentionRequiredData>,
    /// Events carry the structured instance and task (client protocol 1.1),
    /// so the catalog follows them: an `instance_changed` without one is a
    /// removed instance.
    pub follows_events: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// The daemon cannot be reached; the TUI shows the disconnected state
    /// and keeps trying to connect.
    Disconnected(String),
    /// The daemon speaks an incompatible protocol version: shown like a
    /// disconnect, but not retried automatically (gate 11 B P7).
    Version(String),
    /// The daemon answered with an error frame; the connection stays up.
    Rejected { code: String, message: String },
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Disconnected(reason) => write!(f, "disconnected: {reason}"),
            SourceError::Version(message) => f.write_str(message),
            SourceError::Rejected { code, message } => write!(f, "{code}: {message}"),
        }
    }
}

/// What an open terminal delivers ([`Source::poll_terminal`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// A new screen (after a subscription or a refresh).
    Screen(String),
    /// The agent printed something: the screen is out of date (gate 11 B
    /// P5: bytes are only a signal).
    Output,
    /// An error on the terminal connection (`no_terminal`, `forbidden`,
    /// `not_supported`, …).
    Error { code: String, message: String },
    /// The terminal connection ended (its events connection may be fine).
    Closed(String),
}

/// Where the screens' data comes from.
pub trait Source {
    /// (Re)connects and hands over the catalog and needs-you list now;
    /// `poll` then gives only what happens after it.
    fn connect(&mut self) -> Result<Snapshot, SourceError>;
    /// Events that arrived since the last call; never blocks.
    fn poll(&mut self) -> Result<Vec<EventData>, SourceError>;
    /// The operator's reply to a needs-you ask (D35).
    fn answer(&mut self, ask_id: &str, reply: AskReply) -> Result<(), SourceError>;
    /// The operator acts on a needs-you item that is not an ask; the item
    /// leaves when `attention_resolved` arrives, not before (gate 11 B P4).
    fn resolve(&mut self, attention_id: &str, action: AttentionAction) -> Result<(), SourceError>;
    /// Opens `instance_id`'s terminal (replacing an open one) and waits for
    /// its first screen (empty: nothing to show).
    fn open_terminal(&mut self, instance_id: &str) -> Result<String, SourceError>;
    /// Asks for the open terminal's screen again (it arrives through
    /// `poll_terminal`); reconnects the terminal when its connection ended.
    fn refresh_terminal(&mut self) -> Result<(), SourceError>;
    /// What the open terminal delivered since the last call; never blocks.
    fn poll_terminal(&mut self) -> Vec<TerminalEvent>;
    /// Operator input for the open terminal; refusals arrive as
    /// [`TerminalEvent::Error`].
    fn terminal_input(&mut self, bytes: &[u8]) -> Result<(), SourceError>;
    /// Closes the terminal; nothing of it is left running.
    fn close_terminal(&mut self);
}

/// What the operator can pick in an expanded needs-you item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// An option of an ask (`answer_ask`).
    Option(String),
    /// An action of an item that is not an ask (`resolve_attention`).
    Action(AttentionAction),
}

/// One `attention_required` event and the latest state of its ask thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attention {
    pub event_id: u64,
    pub data: AttentionRequiredData,
}

impl Attention {
    /// Stable id: the protocol's `attention_id`, else the ask id, else the
    /// event id.
    pub fn key(&self) -> String {
        if let Some(id) = &self.data.attention_id {
            return id.clone();
        }
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
    /// "needs you" until the agent follows up. An item that is not an ask
    /// stays until its `attention_resolved` removes it from the fleet.
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

    /// What the operator can pick: an ask's options, or the actions of an
    /// item that is not an ask (only the ones this TUI knows).
    pub fn choices(&self) -> Vec<Choice> {
        if self.data.ask.is_some() {
            return self
                .question()
                .1
                .iter()
                .cloned()
                .map(Choice::Option)
                .collect();
        }
        self.data
            .actions
            .iter()
            .copied()
            .filter(|a| *a != AttentionAction::Unknown)
            .map(Choice::Action)
            .collect()
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

/// Everything the screens show: a snapshot plus the events after it.
#[derive(Debug, Clone, Default)]
pub struct Fleet {
    pub catalog: Catalog,
    attention: Vec<Attention>,
    events: Vec<EventData>,
    follows_events: bool,
}

impl Fleet {
    pub fn new(catalog: Catalog) -> Fleet {
        Fleet {
            catalog,
            ..Fleet::default()
        }
    }

    pub fn from_snapshot(snapshot: Snapshot) -> Fleet {
        Fleet {
            catalog: snapshot.catalog,
            attention: snapshot
                .attention
                .into_iter()
                .map(|data| Attention { event_id: 0, data })
                .collect(),
            events: Vec::new(),
            follows_events: snapshot.follows_events,
        }
    }

    pub fn apply(&mut self, event: EventData) {
        match &event.event {
            DaemonEvent::AttentionRequired { data } => {
                let item = Attention {
                    event_id: event.event_id,
                    data: data.clone(),
                };
                let key = item.key();
                match self.attention.iter_mut().find(|a| a.key() == key) {
                    Some(existing) => *existing = item,
                    None => self.attention.push(item),
                }
            }
            DaemonEvent::AskUpdated { data } => {
                for item in &mut self.attention {
                    if let Some(ask) = &mut item.data.ask
                        && ask.ask_id == data.ask_id
                    {
                        *ask = data.clone();
                    }
                }
            }
            DaemonEvent::AttentionResolved { data } => {
                self.attention.retain(|a| a.key() != data.attention_id);
            }
            DaemonEvent::InstanceChanged { data } if self.follows_events => {
                let agents = &mut self.catalog.agents;
                agents.retain(|a| a.id != data.instance_id);
                if let Some(view) = &data.instance {
                    agents.push(client::agent_info(view, &self.catalog.tasks));
                }
            }
            DaemonEvent::TaskChanged { data } if self.follows_events => {
                if let Some(view) = &data.task {
                    let task = client::task_info(view);
                    match self.catalog.tasks.iter_mut().find(|t| t.id == task.id) {
                        Some(existing) => *existing = task,
                        None => self.catalog.tasks.push(task),
                    }
                    client::link_tasks(&mut self.catalog);
                }
            }
            _ => {}
        }
        self.events.push(event);
    }

    /// Items that need the operator, in the D36 order: the protocol's
    /// `unblocks` and `waiting_since_unix_ms`; a source without them (the
    /// scripted demo) counts 0 and the event id instead (oldest first).
    pub fn needs_you(&self) -> Vec<&Attention> {
        let mut items: Vec<AttentionItem> = self
            .attention
            .iter()
            .filter(|a| a.waiting())
            .map(|a| AttentionItem {
                id: a.key(),
                unblocks: a.data.unblocks.unwrap_or(0),
                waiting_since_unix_ms: a.data.waiting_since_unix_ms.unwrap_or(a.event_id),
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

    /// The agent a needs-you item waits on (and `t` opens): who asked, else
    /// the item's instance, else the task holder (gate 11 B P3, P5).
    pub fn item_agent(&self, item: &Attention) -> Option<String> {
        item.asker()
            .map(str::to_owned)
            .or_else(|| item.data.instance_id.clone())
            .or_else(|| self.task(item.task_id()?)?.holder.clone())
    }

    /// The team an item belongs to: its task's, else its agent's.
    pub fn item_team(&self, item: &Attention) -> Option<String> {
        if let Some(task) = item.task_id().and_then(|id| self.task(id)) {
            return Some(task.team_id.clone());
        }
        let agent = self.item_agent(item)?;
        Some(self.agent(&agent)?.team_id.clone())
    }

    /// What happens if the item is left alone: the protocol's `if_ignored`,
    /// else the catalog's text for its task.
    pub fn if_ignored(&self, item: &Attention) -> Option<String> {
        item.data
            .if_ignored
            .clone()
            .or_else(|| self.catalog.if_ignored.get(item.task_id()?).cloned())
    }

    /// An agent's state as shown. "Needs you" follows the current needs-you
    /// list: an agent some waiting item points at needs you. Otherwise the
    /// source's state stands, except a scripted "needs you" with nothing
    /// waiting any more, which becomes working while the agent holds an
    /// unfinished task and idle otherwise.
    pub fn agent_state(&self, agent: &AgentInfo) -> AgentState {
        let waiting = self
            .needs_you()
            .iter()
            .any(|item| self.item_agent(item).as_deref() == Some(agent.id.as_str()));
        match agent.state {
            _ if waiting => AgentState::NeedsYou,
            AgentState::NeedsYou => {
                let busy = agent
                    .task_id
                    .as_deref()
                    .and_then(|id| self.task(id))
                    .is_some_and(|task| !task.is_done());
                if busy {
                    AgentState::Working
                } else {
                    AgentState::Idle
                }
            }
            state => state,
        }
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
                    attention_id: None,
                    unblocks: None,
                    waiting_since_unix_ms: None,
                    if_ignored: None,
                    actions: Vec::new(),
                    instance_id: None,
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

    fn failed(id: &str, since: u64) -> EventData {
        EventData {
            event_id: 100 - since,
            event: DaemonEvent::AttentionRequired {
                data: AttentionRequiredData {
                    reason: format!("{id} failed"),
                    task_id: None,
                    ask: None,
                    recap: None,
                    attention_id: Some(format!("instance-failed:{id}")),
                    unblocks: Some(0),
                    waiting_since_unix_ms: Some(since),
                    if_ignored: Some(format!("{id} stays stopped")),
                    actions: vec![AttentionAction::Retry],
                    instance_id: Some(id.into()),
                },
            },
        }
    }

    fn view(id: &str, state: agend_core::protocol::client::AgentState) -> EventData {
        EventData {
            event_id: 50,
            event: DaemonEvent::InstanceChanged {
                data: agend_core::protocol::client::InstanceChangedData {
                    instance_id: id.into(),
                    summary: state.as_str().into(),
                    instance: Some(agend_core::protocol::client::InstanceView {
                        instance_id: id.into(),
                        team_id: "general".into(),
                        backend: "claude".into(),
                        state,
                        working_directory: None,
                    }),
                },
            },
        }
    }

    #[test]
    fn items_leave_on_attention_resolved_and_order_by_waiting_time() {
        let mut fleet = Fleet::from_snapshot(Snapshot {
            follows_events: true,
            ..Snapshot::default()
        });
        fleet.apply(failed("g-late", 20));
        fleet.apply(failed("g-early", 10));
        fleet.apply(failed("g-early", 10)); // raised again: still one item
        let keys: Vec<String> = fleet.needs_you().iter().map(|a| a.key()).collect();
        assert_eq!(keys, ["instance-failed:g-early", "instance-failed:g-late"]);
        let item = fleet.attention("instance-failed:g-early").unwrap();
        assert_eq!(fleet.item_agent(item).as_deref(), Some("g-early"));
        assert_eq!(
            fleet.if_ignored(item).as_deref(),
            Some("g-early stays stopped")
        );
        assert_eq!(item.choices(), [Choice::Action(AttentionAction::Retry)]);
        fleet.apply(EventData {
            event_id: 200,
            event: DaemonEvent::AttentionResolved {
                data: agend_core::protocol::client::AttentionResolvedData {
                    attention_id: "instance-failed:g-early".into(),
                    action: AttentionAction::Retry,
                },
            },
        });
        assert_eq!(fleet.needs_you().len(), 1);
    }

    #[test]
    fn a_following_catalog_takes_instance_changes_and_removals() {
        use agend_core::protocol::client::AgentState as Wire;
        let mut fleet = Fleet::from_snapshot(Snapshot {
            follows_events: true,
            ..Snapshot::default()
        });
        fleet.apply(view("g-1", Wire::Starting));
        fleet.apply(view("g-1", Wire::Failed));
        fleet.apply(failed("g-1", 1));
        let agent = fleet.agent("g-1").unwrap().clone();
        assert_eq!(fleet.catalog.agents.len(), 1);
        assert_eq!(fleet.agent_state(&agent), AgentState::NeedsYou);
        fleet.apply(EventData {
            event_id: 60,
            event: DaemonEvent::InstanceChanged {
                data: agend_core::protocol::client::InstanceChangedData {
                    instance_id: "g-1".into(),
                    summary: "removed".into(),
                    instance: None,
                },
            },
        });
        assert!(fleet.agent("g-1").is_none());
        // A replaying source's events carry no structure: nothing changes.
        let mut scripted = Fleet::default();
        scripted.apply(view("g-1", Wire::Failed));
        assert!(scripted.agent("g-1").is_none());
    }
}
