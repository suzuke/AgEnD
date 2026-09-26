//! In-memory scripted source for tests and the demo. It keeps an event log of
//! client protocol v1 events and answers asks with the same rules as the
//! testkit fake daemon (`AskThread::accepts`, an `ask_updated` event per
//! answer), so the screens see exactly what a daemon would send.
//!
//! A [`ScriptHandle`] drives it from outside: emit events, open asks, follow
//! up, take the "daemon" offline and back.
//!
//! Must NOT: be used outside tests, examples and demos.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use agend_core::model::Backend;
use agend_core::pipeline::stage::StageKind;
use agend_core::protocol::ask::{AnswerSource, AskEntry, AskReply, AskThread, ContextRecap};
use agend_core::protocol::client::{
    AttentionRequiredData, DaemonEvent, EventData, InstanceChangedData, TaskChangedData,
};

use super::{AgentInfo, AgentState, Catalog, Source, SourceError, StageInfo, StageState, TaskInfo};

#[derive(Default)]
struct Script {
    catalog: Catalog,
    log: Vec<EventData>,
    asks: BTreeMap<String, AskThread>,
    terminals: BTreeMap<String, String>,
    offline: bool,
    answers: Vec<(String, AskReply)>,
}

fn lock(script: &Mutex<Script>) -> MutexGuard<'_, Script> {
    script.lock().unwrap_or_else(|e| e.into_inner())
}

fn emit(script: &mut Script, event: DaemonEvent) -> u64 {
    let event_id = script.log.len() as u64 + 1;
    script.log.push(EventData { event_id, event });
    event_id
}

pub struct ScriptedSource {
    script: Arc<Mutex<Script>>,
    delivered: usize,
    connected: bool,
}

/// Drives a [`ScriptedSource`] from a test or the demo.
#[derive(Clone)]
pub struct ScriptHandle(Arc<Mutex<Script>>);

impl ScriptedSource {
    pub fn new(catalog: Catalog) -> (ScriptedSource, ScriptHandle) {
        let script = Arc::new(Mutex::new(Script {
            catalog,
            ..Script::default()
        }));
        let source = ScriptedSource {
            script: Arc::clone(&script),
            delivered: 0,
            connected: false,
        };
        (source, ScriptHandle(script))
    }

    /// The demo fleet: [`demo_catalog`], [`demo_script`] and terminal screens.
    pub fn demo() -> (ScriptedSource, ScriptHandle) {
        let (source, handle) = ScriptedSource::new(demo_catalog());
        seed_demo(&handle);
        for (agent, screen) in demo_terminals() {
            handle.set_terminal(agent, &screen);
        }
        (source, handle)
    }

    fn online(&self) -> Result<MutexGuard<'_, Script>, SourceError> {
        let script = lock(&self.script);
        if script.offline || !self.connected {
            return Err(SourceError::Disconnected("connection closed".into()));
        }
        Ok(script)
    }
}

impl Source for ScriptedSource {
    fn connect(&mut self) -> Result<Catalog, SourceError> {
        let script = lock(&self.script);
        if script.offline {
            return Err(SourceError::Disconnected(
                "daemon is not running (connection refused)".into(),
            ));
        }
        let catalog = script.catalog.clone();
        drop(script);
        self.delivered = 0;
        self.connected = true;
        Ok(catalog)
    }

    fn poll(&mut self) -> Result<Vec<EventData>, SourceError> {
        let delivered = self.delivered;
        let result = self.online().map(|script| script.log[delivered..].to_vec());
        let new = result.inspect_err(|_| self.connected = false)?;
        self.delivered += new.len();
        Ok(new)
    }

    fn terminal(&mut self, instance_id: &str) -> Result<String, SourceError> {
        let script = self.online()?;
        Ok(script
            .terminals
            .get(instance_id)
            .cloned()
            .unwrap_or_default())
    }

    fn answer(&mut self, ask_id: &str, reply: AskReply) -> Result<(), SourceError> {
        let mut script = self.online()?;
        let Some(thread) = script.asks.get_mut(ask_id) else {
            return Err(SourceError::Rejected {
                code: "unknown_ask".into(),
                message: format!("no open ask {ask_id}"),
            });
        };
        if !thread.accepts(&reply) {
            return Err(SourceError::Rejected {
                code: "invalid_request".into(),
                message: format!("ask {ask_id} does not accept this reply now"),
            });
        }
        thread.entries.push(AskEntry::Answer {
            from: "operator".into(),
            source: AnswerSource::Tui,
            reply: reply.clone(),
        });
        let thread = thread.clone();
        script.answers.push((ask_id.to_owned(), reply));
        emit(&mut script, DaemonEvent::AskUpdated { data: thread });
        Ok(())
    }
}

impl ScriptHandle {
    pub fn emit(&self, event: DaemonEvent) -> u64 {
        emit(&mut lock(&self.0), event)
    }

    /// Opens an answerable ask and emits its `attention_required` event.
    pub fn open_ask(&self, thread: AskThread, recap: Option<ContextRecap>) -> u64 {
        let mut script = lock(&self.0);
        script.asks.insert(thread.ask_id.clone(), thread.clone());
        emit(
            &mut script,
            DaemonEvent::AttentionRequired {
                data: AttentionRequiredData {
                    reason: "ask".into(),
                    task_id: thread.task_id.clone(),
                    ask: Some(thread),
                    recap,
                },
            },
        )
    }

    /// The agent continues an ask after an answer (D35).
    pub fn follow_up(&self, ask_id: &str, from: &str, text: &str, options: &[&str]) {
        let mut script = lock(&self.0);
        let Some(thread) = script.asks.get_mut(ask_id) else {
            return;
        };
        thread.entries.push(AskEntry::FollowUp {
            from: from.into(),
            text: text.into(),
            options: options.iter().map(|o| (*o).into()).collect(),
        });
        let thread = thread.clone();
        emit(&mut script, DaemonEvent::AskUpdated { data: thread });
    }

    /// Offline: every call fails as if the daemon stopped; online again:
    /// the next `connect` succeeds.
    pub fn set_online(&self, online: bool) {
        lock(&self.0).offline = !online;
    }

    pub fn set_terminal(&self, instance_id: &str, screen: &str) {
        lock(&self.0)
            .terminals
            .insert(instance_id.to_owned(), screen.to_owned());
    }

    /// Every accepted answer, in order.
    pub fn answers(&self) -> Vec<(String, AskReply)> {
        lock(&self.0).answers.clone()
    }
}

fn stage(name: &str, kind: StageKind, state: StageState, agent: Option<&str>) -> StageInfo {
    StageInfo {
        name: name.into(),
        kind,
        state,
        agent: agent.map(Into::into),
    }
}

/// The `code` workflow's stages with the first `done` done and the next one
/// running (by `agent` for work and approval stages).
fn code_stages(done: usize, holder: &str, reviewer: &str) -> Vec<StageInfo> {
    let plan = [
        ("implement", StageKind::Work, Some(holder)),
        ("submit", StageKind::Submit, None),
        ("checks", StageKind::Command, None),
        ("review", StageKind::Approval, Some(reviewer)),
        ("merge", StageKind::Merge, None),
    ];
    plan.iter()
        .enumerate()
        .map(|(i, (name, kind, agent))| {
            let state = match i.cmp(&done) {
                std::cmp::Ordering::Less => StageState::Done,
                std::cmp::Ordering::Equal => StageState::Running,
                std::cmp::Ordering::Greater => StageState::NotStarted,
            };
            stage(name, *kind, state, *agent)
        })
        .collect()
}

fn task(id: &str, team: &str, title: &str, repo: Option<&str>, holder: &str) -> TaskInfo {
    TaskInfo {
        id: id.into(),
        team_id: team.into(),
        title: title.into(),
        repo: repo.map(Into::into),
        holder: Some(holder.into()),
        stages: Vec::new(),
    }
}

fn agent(
    id: &str,
    team: &str,
    backend: Backend,
    state: AgentState,
    task: Option<&str>,
) -> AgentInfo {
    AgentInfo {
        id: id.into(),
        team_id: team.into(),
        backend,
        state,
        task_id: task.map(Into::into),
    }
}

/// Three teams (one of them the built-in `general`), five tasks, six agents.
/// One title is CJK so wide-character layout is exercised.
pub fn demo_catalog() -> Catalog {
    let repo = Some("agend-terminal");
    let sims = Some("nulls-sim");
    Catalog {
        teams: vec!["archfix".into(), "research".into(), "general".into()],
        tasks: vec![
            TaskInfo {
                stages: code_stages(0, "dev-2", "qa-1"),
                ..task(
                    "T-45",
                    "archfix",
                    "Restructure state boundary",
                    repo,
                    "dev-2",
                )
            },
            TaskInfo {
                stages: code_stages(2, "dev-1", "qa-1"),
                ..task("T-52", "archfix", "Fix lock-order inversion", repo, "dev-1")
            },
            TaskInfo {
                stages: code_stages(5, "dev-1", "qa-1"),
                ..task("T-37", "archfix", "Tidy config loader", repo, "dev-1")
            },
            TaskInfo {
                stages: code_stages(3, "dev-3", "reviewer-1"),
                ..task(
                    "T-88",
                    "research",
                    "Finish simulator review · 完成模擬器審查報告",
                    sims,
                    "dev-3",
                )
            },
            TaskInfo {
                stages: code_stages(0, "dev-3", "reviewer-1"),
                ..task("T-90", "research", "Survey fuzzers", sims, "dev-3")
            },
        ],
        agents: vec![
            agent(
                "dev-1",
                "archfix",
                Backend::Claude,
                AgentState::Working,
                Some("T-52"),
            ),
            agent(
                "dev-2",
                "archfix",
                Backend::Codex,
                AgentState::NeedsYou,
                Some("T-45"),
            ),
            agent("qa-1", "archfix", Backend::Opencode, AgentState::Idle, None),
            agent(
                "dev-3",
                "research",
                Backend::Codex,
                AgentState::NeedsYou,
                Some("T-90"),
            ),
            agent(
                "reviewer-1",
                "research",
                Backend::Claude,
                AgentState::Stuck,
                Some("T-88"),
            ),
            agent(
                "writer-1",
                "general",
                Backend::Codex,
                AgentState::Idle,
                None,
            ),
        ],
        // Fixed text: protocol v1 has no "if ignored" field (gap G1).
        if_ignored: [
            (
                "T-45",
                "T-45 stays blocked at implement; dev-2 cannot rerun the suite.",
            ),
            (
                "T-88",
                "T-88 review stays stopped until reviewer-1's usage limit resets.",
            ),
            (
                "T-90",
                "dev-3 keeps waiting; the T-90 survey does not start.",
            ),
        ]
        .into_iter()
        .map(|(task, text)| (task.to_owned(), text.to_owned()))
        .collect(),
    }
}

/// One step of the demo script: an event, or an ask to open (answerable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemoStep {
    Event(DaemonEvent),
    Ask(AskThread, Option<ContextRecap>),
}

/// The demo, in order. Both the scripted source ([`seed_demo`]) and the
/// fake-daemon demo seed from it, so they carry the same data.
pub fn demo_script() -> Vec<DemoStep> {
    let task = |id: &str, summary: &str| {
        DemoStep::Event(DaemonEvent::TaskChanged {
            data: TaskChangedData {
                task_id: id.into(),
                summary: summary.into(),
            },
        })
    };
    let instance = |id: &str, summary: &str| {
        DemoStep::Event(DaemonEvent::InstanceChanged {
            data: InstanceChangedData {
                instance_id: id.into(),
                summary: summary.into(),
            },
        })
    };
    vec![
        task("T-37", "merged"),
        instance("dev-1", "started checks for T-52"),
        DemoStep::Ask(
            AskThread {
                ask_id: "A-1".into(),
                task_id: Some("T-45".into()),
                entries: vec![AskEntry::Question {
                    from: "dev-2".into(),
                    text: "Regression suite fails 3 of 10 runs. Which test strategy?".into(),
                    options: vec!["fixed seed 42".into(), "20-run plan".into()],
                }],
            },
            Some(ContextRecap {
                goal: "Split the state module so the daemon owns all writes.".into(),
                decisions: vec!["Keep the public API unchanged.".into()],
                asking: "How to tell a real regression from flakiness.".into(),
                next: "dev-2 reruns the suite with your strategy, then submits.".into(),
            }),
        ),
        task("T-52", "checks started"),
        DemoStep::Event(DaemonEvent::AttentionRequired {
            data: AttentionRequiredData {
                reason: "reviewer-1 hit its usage limit".into(),
                task_id: Some("T-88".into()),
                ask: None,
                recap: None,
            },
        }),
        instance("reviewer-1", "usage limit reached"),
        DemoStep::Ask(
            AskThread {
                ask_id: "A-2".into(),
                task_id: Some("T-90".into()),
                entries: vec![AskEntry::Question {
                    from: "dev-3".into(),
                    text: "Which fuzzer should the survey cover first?".into(),
                    options: vec!["cargo-fuzz".into(), "AFL++".into()],
                }],
            },
            None,
        ),
    ]
}

pub fn seed_demo(handle: &ScriptHandle) {
    for step in demo_script() {
        match step {
            DemoStep::Event(event) => {
                handle.emit(event);
            }
            DemoStep::Ask(thread, recap) => {
                handle.open_ask(thread, recap);
            }
        }
    }
}

/// Fixed terminal screens for agents that have output.
pub fn demo_terminals() -> Vec<(&'static str, String)> {
    vec![
        (
            "dev-1",
            "$ cargo test --workspace\n   Compiling agend-core\ntest result: ok. 212 passed\n$ agend status\nT-52 · checks running"
                .into(),
        ),
        (
            "dev-2",
            "Running regression suite (10 runs)\nrun 3: FAIL state_boundary::drop_order\nrun 7: FAIL state_boundary::drop_order\n> agend ask \"Which test strategy?\""
                .into(),
        ),
        (
            "dev-3",
            "Reading fuzzing notes\n> agend ask \"Which fuzzer first?\"".into(),
        ),
        (
            "reviewer-1",
            "Reviewing simulator report\nUsage limit reached. Try again later.".into(),
        ),
    ]
}
