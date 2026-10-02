//! The real [`Source`]: the daemon through `agend-client` (gate 11 B P1).
//! There is no socket code here; `agend-client` owns the connection.
//!
//! Three connections, each with its own role:
//! - events: `get_fleet`, then `subscribe_events` after its
//!   `as_of_event_id`; a thread blocks in `next_event` and hands events to
//!   the main thread over a channel (`poll` never blocks);
//! - requests: `answer_ask` and `resolve_attention`, sent and waited for on
//!   the main thread (at most 10 s; P7 accepts that freeze);
//! - terminal: opened with the terminal view and closed with it. A thread
//!   blocks in `next_terminal`; the main thread writes `subscribe_terminal`
//!   and `terminal_input` through the connection's write-only
//!   `agend_client::Sender`. Every error line on it belongs to the terminal
//!   (no other request waits there), so `terminal_input`'s id-less errors
//!   never become another request's reply.
//!
//! Closing a connection is `Sender::close` (shutdown both ways: the blocked
//! thread reads the end at once) and joining its thread, so nothing is left
//! behind. The fleet view becomes a [`Catalog`] here (P3); screens never see
//! `FleetView`. What the protocol does not carry yet stays empty (gap G5:
//! stage kinds and agents, repos).
//!
//! Must NOT: build an async runtime (D11), reuse an event cursor after a
//! reconnect (gate 8 P4), or drop a needs-you item before the daemon's
//! `attention_resolved`.

mod full_terminal;
use full_terminal::FullConnection;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agend_client::{Client, ClientError, Sender, TerminalUpdate};
use agend_core::model::Backend;
use agend_core::protocol::ask::{AnswerSource, AskReply};
use agend_core::protocol::client::{
    AgentState as WireState, AttentionAction, EventData, FleetView, InstanceView, TaskView,
};

use super::{
    AgentInfo, AgentState, Catalog, FullTerminalEvent, Snapshot, Source, SourceError, StageInfo,
    StageState, TaskInfo, TerminalEvent,
};

/// Longest wait for a terminal's first screen (as for any reply, gate 8 P7).
const FIRST_SCREEN_WITHIN: Duration = Duration::from_secs(10);

/// A connection read by its own thread.
struct Reader<T> {
    sender: Sender,
    items: Receiver<T>,
    thread: Option<JoinHandle<()>>,
}

impl<T> Reader<T> {
    /// Shuts the connection down (waking the thread) and waits for it.
    fn close(mut self) {
        self.sender.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct ClientSource {
    socket: PathBuf,
    caller: Option<String>,
    requests: Option<Client>,
    events: Option<Reader<Result<EventData, ClientError>>>,
    terminal: Option<Reader<TerminalEvent>>,
    full_terminal: Option<FullConnection>,
    /// The instance of the open terminal view (kept while its connection
    /// is being reconnected).
    terminal_of: Option<String>,
    threads: Arc<AtomicUsize>,
}

impl ClientSource {
    /// A source for the daemon at `socket`; `caller` is `AGEND_INSTANCE`
    /// inside an agent (the daemon then refuses operator actions, D17).
    pub fn new(socket: &Path, caller: Option<String>) -> ClientSource {
        ClientSource {
            socket: socket.to_path_buf(),
            caller,
            requests: None,
            events: None,
            terminal: None,
            full_terminal: None,
            terminal_of: None,
            threads: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Reader threads running now (tests check that none is left behind).
    pub fn threads(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.threads)
    }

    fn connect_once(&self) -> Result<Client, SourceError> {
        Client::connect_once(&self.socket, self.caller.clone()).map_err(connect_error)
    }

    /// Starts a thread that feeds `next(client)` into a channel until
    /// `last` says the connection is done.
    fn read<T: Send + 'static>(
        &self,
        name: &str,
        mut client: Client,
        mut next: impl FnMut(&mut Client) -> T + Send + 'static,
        last: fn(&T) -> bool,
    ) -> Result<Reader<T>, SourceError> {
        let sender = client.sender().map_err(connect_error)?;
        let (tx, items) = channel();
        let threads = Arc::clone(&self.threads);
        threads.fetch_add(1, Ordering::SeqCst);
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                loop {
                    let item = next(&mut client);
                    let done = last(&item);
                    if tx.send(item).is_err() || done {
                        break;
                    }
                }
                threads.fetch_sub(1, Ordering::SeqCst);
            });
        match spawned {
            Ok(thread) => Ok(Reader {
                sender,
                items,
                thread: Some(thread),
            }),
            Err(e) => {
                self.threads.fetch_sub(1, Ordering::SeqCst);
                Err(SourceError::Disconnected(format!(
                    "cannot start a reader thread: {e}"
                )))
            }
        }
    }

    fn disconnect(&mut self) {
        self.requests = None;
        if let Some(events) = self.events.take() {
            events.close();
        }
    }

    /// The request connection's answer to `f`.
    fn request(
        &mut self,
        f: impl FnOnce(&mut Client) -> Result<(), ClientError>,
    ) -> Result<(), SourceError> {
        let client = self
            .requests
            .as_mut()
            .ok_or_else(|| SourceError::Disconnected("not connected".into()))?;
        match f(client) {
            Ok(()) => Ok(()),
            Err(ClientError::Daemon { code, message }) => {
                Err(SourceError::Rejected { code, message })
            }
            Err(e) => {
                self.disconnect();
                Err(SourceError::Disconnected(e.to_string()))
            }
        }
    }

    fn drop_terminal_connection(&mut self) {
        self.full_terminal = None;
        if let Some(terminal) = self.terminal.take() {
            terminal.close();
        }
    }

    /// A new terminal connection subscribed to the open terminal's instance.
    fn start_terminal(&mut self) -> Result<(), SourceError> {
        self.drop_terminal_connection();
        let instance = self
            .terminal_of
            .clone()
            .ok_or_else(|| SourceError::Disconnected("no terminal is open".into()))?;
        let client = self.connect_once()?;
        let mut reader = self.read("tui-terminal", client, next_terminal, |event| {
            matches!(event, TerminalEvent::Closed(_))
        })?;
        if let Err(e) = reader.sender.subscribe_terminal(&instance) {
            reader.close();
            return Err(SourceError::Disconnected(e.to_string()));
        }
        self.terminal = Some(reader);
        Ok(())
    }
}

impl Drop for ClientSource {
    fn drop(&mut self) {
        self.close_terminal();
        self.disconnect();
    }
}

fn connect_error(error: ClientError) -> SourceError {
    match error {
        ClientError::Version(message) => SourceError::Version(message),
        other => SourceError::Disconnected(other.to_string()),
    }
}

fn next_event(client: &mut Client) -> Result<EventData, ClientError> {
    client.next_event()
}

fn next_terminal(client: &mut Client) -> TerminalEvent {
    match client.next_terminal() {
        Ok(TerminalUpdate::Screen { screen, .. }) => TerminalEvent::Screen(screen),
        Ok(TerminalUpdate::Bytes { .. }) => TerminalEvent::Output,
        Err(ClientError::Daemon { code, message }) => TerminalEvent::Error { code, message },
        Err(e) => TerminalEvent::Closed(e.to_string()),
    }
}

impl Source for ClientSource {
    fn poll_full_terminal(&mut self) -> Vec<FullTerminalEvent> {
        if let Some(full) = &self.full_terminal {
            let events = full.poll();
            if events
                .iter()
                .any(|event| matches!(event, FullTerminalEvent::Closed(_)))
            {
                self.full_terminal = None;
            }
            return events;
        }

        Vec::new()
    }

    fn open_full_terminal(
        &mut self,
        instance: &str,
        request_id: String,
        viewport: agend_core::protocol::terminal::TerminalViewport,
    ) -> Result<bool, SourceError> {
        self.close_terminal();
        let client = self.connect_once()?;
        let selected = client.selected();
        if selected.major != 1 || selected.minor < 4 {
            return Ok(false);
        }
        let connection = FullConnection::start(client, self.threads.clone())?;
        connection.send(
            agend_core::protocol::client::ClientRequest::SubscribeTerminalFrames {
                data: agend_core::protocol::client::TerminalSubscribeData {
                    instance_id: instance.into(),
                    request_id,
                    viewport,
                },
            },
        )?;
        self.terminal_of = Some(instance.into());
        self.full_terminal = Some(connection);
        Ok(true)
    }
    fn terminal_control(
        &mut self,
        data: agend_core::protocol::client::ClientTerminalControlData,
    ) -> Result<(), SourceError> {
        self.full_terminal
            .as_ref()
            .ok_or_else(|| SourceError::Disconnected("no full terminal connection".into()))?
            .send(agend_core::protocol::client::ClientRequest::TerminalControl { data })
    }
    fn terminal_viewport(
        &mut self,
        data: agend_core::protocol::client::TerminalViewportData,
    ) -> Result<(), SourceError> {
        self.full_terminal
            .as_ref()
            .ok_or_else(|| SourceError::Disconnected("no full terminal connection".into()))?
            .send(agend_core::protocol::client::ClientRequest::SetTerminalViewport { data })
    }

    fn connect(&mut self) -> Result<Snapshot, SourceError> {
        self.disconnect();
        let requests = self.connect_once()?;
        let mut events = self.connect_once()?;
        let fleet = events.get_fleet().map_err(connect_error)?;
        events
            .subscribe_events(Some(fleet.as_of_event_id))
            .map_err(connect_error)?;
        self.events = Some(self.read("tui-events", events, next_event, Result::is_err)?);
        self.requests = Some(requests);
        Ok(snapshot(fleet))
    }

    fn poll(&mut self) -> Result<Vec<EventData>, SourceError> {
        let events = self
            .events
            .as_ref()
            .ok_or_else(|| SourceError::Disconnected("not connected".into()))?;
        let mut out = Vec::new();
        let lost = loop {
            match events.items.try_recv() {
                Ok(Ok(event)) => out.push(event),
                Ok(Err(e)) => break e.to_string(),
                Err(TryRecvError::Empty) => return Ok(out),
                Err(TryRecvError::Disconnected) => break "the event reader ended".into(),
            }
        };
        self.disconnect();
        Err(SourceError::Disconnected(lost))
    }

    fn answer(&mut self, ask_id: &str, reply: AskReply) -> Result<(), SourceError> {
        self.request(|c| c.answer_ask(ask_id, AnswerSource::Tui, reply))
    }

    fn resolve(&mut self, attention_id: &str, action: AttentionAction) -> Result<(), SourceError> {
        self.request(|c| c.resolve_attention(attention_id, action))
    }
    fn request_changes(&mut self, attention_id: &str, note: String) -> Result<(), SourceError> {
        self.request(|c| {
            c.resolve_attention_with_note(attention_id, AttentionAction::RequestChanges, Some(note))
        })
    }

    fn open_terminal(&mut self, instance_id: &str) -> Result<String, SourceError> {
        self.terminal_of = Some(instance_id.to_owned());
        self.start_terminal()?;
        let reader = self.terminal.as_ref().expect("started");
        let deadline = Instant::now() + FIRST_SCREEN_WITHIN;
        let failed = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match reader.items.recv_timeout(left) {
                Ok(TerminalEvent::Screen(screen)) => return Ok(screen),
                Ok(TerminalEvent::Output) => {}
                Ok(TerminalEvent::Error { code, message }) => {
                    break SourceError::Rejected { code, message };
                }
                Ok(TerminalEvent::Closed(reason)) => break SourceError::Disconnected(reason),
                Err(RecvTimeoutError::Timeout) => {
                    break SourceError::Disconnected(format!(
                        "no screen from the daemon within {} s",
                        FIRST_SCREEN_WITHIN.as_secs()
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    break SourceError::Disconnected("the terminal reader ended".into());
                }
            }
        };
        self.drop_terminal_connection();
        Err(failed)
    }

    fn refresh_terminal(&mut self) -> Result<(), SourceError> {
        let Some(instance) = self.terminal_of.clone() else {
            return Err(SourceError::Disconnected("no terminal is open".into()));
        };
        match self.terminal.as_mut() {
            Some(reader) => match reader.sender.subscribe_terminal(&instance) {
                Ok(()) => Ok(()),
                Err(e) => {
                    self.drop_terminal_connection();
                    Err(SourceError::Disconnected(e.to_string()))
                }
            },
            None => self.start_terminal(),
        }
    }

    fn poll_terminal(&mut self) -> Vec<TerminalEvent> {
        let Some(reader) = &self.terminal else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut ended = false;
        loop {
            match reader.items.try_recv() {
                Ok(event) => {
                    ended |= matches!(event, TerminalEvent::Closed(_));
                    out.push(event);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !ended {
                        out.push(TerminalEvent::Closed("the terminal reader ended".into()));
                    }
                    ended = true;
                    break;
                }
            }
        }
        if ended {
            // Only this connection: the caller reconnects it (P5).
            self.drop_terminal_connection();
        }
        out
    }

    fn terminal_input(&mut self, bytes: &[u8]) -> Result<(), SourceError> {
        let instance = self.terminal_of.clone().unwrap_or_default();
        let Some(reader) = self.terminal.as_mut() else {
            return Err(SourceError::Disconnected(
                "the terminal connection is closed".into(),
            ));
        };
        if let Err(e) = reader.sender.terminal_input(&instance, bytes) {
            self.drop_terminal_connection();
            return Err(SourceError::Disconnected(e.to_string()));
        }
        Ok(())
    }

    fn close_terminal(&mut self) {
        self.terminal_of = None;
        self.drop_terminal_connection();
    }
}

/// The fleet view as the screens' catalog and needs-you list (P3).
pub fn snapshot(fleet: FleetView) -> Snapshot {
    let mut catalog = Catalog {
        teams: fleet.teams.into_iter().map(|t| t.team_id).collect(),
        tasks: fleet.tasks.iter().map(task_info).collect(),
        agents: Vec::new(),
        if_ignored: Default::default(),
    };
    catalog.agents = fleet
        .instances
        .iter()
        .map(|view| agent_info(view, &catalog.tasks))
        .collect();
    Snapshot {
        catalog,
        attention: fleet.attention,
        follows_events: true,
    }
}

/// A task of the fleet view. Its stages are ids only (kinds, per-stage
/// states and agents are gap G5): before `current_stage` done, it running,
/// after it not started. No repo (G5).
pub fn task_info(view: &TaskView) -> TaskInfo {
    let current = view
        .current_stage
        .as_ref()
        .and_then(|c| view.stages.iter().position(|s| s == c));
    let done = matches!(view.status.as_str(), "done" | "superseded");
    let stages = view
        .stages
        .iter()
        .enumerate()
        .map(|(i, name)| StageInfo {
            name: name.clone(),
            kind: view
                .pipeline
                .as_ref()
                .and_then(|p| p.stage_kinds.get(i))
                .and_then(|kind| agend_core::pipeline::stage::StageKind::parse(kind)),
            state: match current {
                _ if done => StageState::Done,
                Some(c) if i < c => StageState::Done,
                Some(c) if i == c && view.status == "failed" => StageState::Failed,
                Some(c) if i == c && view.status == "cancelled" => StageState::Cancelled,
                Some(c) if i == c => StageState::Running,
                _ => StageState::NotStarted,
            },
            agent: view
                .pipeline
                .as_ref()
                .and_then(|p| p.stage_agents.get(i))
                .cloned()
                .flatten(),
        })
        .collect();
    TaskInfo {
        id: view.task_id.clone(),
        team_id: view.team_id.clone(),
        title: view.title.clone(),
        repo: view.pipeline.as_ref().and_then(|p| p.repo.clone()),
        holder: view.assignee.clone(),
        stages,
        status: view.status.clone(),
        pipeline: view.pipeline.clone(),
    }
}

/// An instance of the fleet view; its task is the first unfinished task it
/// holds (the protocol names the assignee on the task, not the task on the
/// instance).
pub fn agent_info(view: &InstanceView, tasks: &[TaskInfo]) -> AgentInfo {
    AgentInfo {
        id: view.instance_id.clone(),
        team_id: view.team_id.clone(),
        backend: Backend::parse(&view.backend).unwrap_or(Backend::Claude),
        state: match view.state {
            WireState::Starting => AgentState::Starting,
            WireState::Working => AgentState::Working,
            WireState::Idle => AgentState::Idle,
            WireState::Stuck => AgentState::Stuck,
            WireState::Failed => AgentState::Failed,
            WireState::Unknown => AgentState::Unknown,
        },
        task_id: held_task(&view.instance_id, tasks),
    }
}

fn held_task(agent: &str, tasks: &[TaskInfo]) -> Option<String> {
    tasks
        .iter()
        .find(|t| t.holder.as_deref() == Some(agent) && !t.is_done())
        .map(|t| t.id.clone())
}

/// After a task changed: every agent's task again.
pub fn link_tasks(catalog: &mut Catalog) {
    let tasks = catalog.tasks.clone();
    for agent in &mut catalog.agents {
        agent.task_id = held_task(&agent.id, &tasks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::protocol::client::TeamView;

    fn task(id: &str, holder: &str, status: &str, current: Option<&str>) -> TaskView {
        TaskView {
            task_id: id.into(),
            title: format!("title {id}"),
            team_id: "general".into(),
            status: status.into(),
            assignee: Some(holder.into()),
            stages: vec!["work".into(), "review".into(), "merge".into()],
            current_stage: current.map(Into::into),
            pipeline: None,
        }
    }

    #[test]
    fn the_fleet_view_becomes_the_catalog() {
        let view = FleetView {
            as_of_event_id: 7,
            teams: vec![TeamView {
                team_id: "general".into(),
            }],
            tasks: vec![
                task("t-1", "g-1", "done", Some("merge")),
                task("t-2", "g-1", "running", Some("review")),
            ],
            instances: vec![InstanceView {
                instance_id: "g-1".into(),
                team_id: "general".into(),
                backend: "codex".into(),
                state: WireState::Failed,
                working_directory: None,
            }],
            attention: Vec::new(),
        };
        let snapshot = snapshot(view);
        let catalog = &snapshot.catalog;
        assert!(snapshot.follows_events);
        assert_eq!(catalog.teams, ["general"]);
        let states: Vec<StageState> = catalog.tasks[1].stages.iter().map(|s| s.state).collect();
        assert_eq!(
            states,
            [
                StageState::Done,
                StageState::Running,
                StageState::NotStarted
            ]
        );
        assert!(catalog.tasks[0].is_done() && !catalog.tasks[1].is_done());
        assert!(catalog.tasks.iter().all(|t| t.repo.is_none()));
        let agent = &catalog.agents[0];
        assert_eq!(
            (agent.backend, agent.state, agent.task_id.as_deref()),
            (Backend::Codex, AgentState::Failed, Some("t-2"))
        );
    }
}
