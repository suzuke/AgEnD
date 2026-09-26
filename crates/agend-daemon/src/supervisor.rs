//! Supervisor: keeps the DB's instances running (gate 6 P2, P3, P6). Later
//! gates add detecting stuck agents, usage limits and unknown prompts,
//! reassigning work, and escalating to a human only for exceptions.
//!
//! Gate 6:
//! - Boot: runs [`crate::boot::plan_boot`]'s actions (reconnect, start,
//!   `Shutdown` orphans).
//! - The instance row is in the DB before anything starts. It stays `new`
//!   until the first `Spawn` is acknowledged (`Spawned` or
//!   `already_spawned`): only then does the backend session exist, so only
//!   `running` instances resume. A daemon that dies before that point starts
//!   the session again with `--session-id` (verifier r1 F1).
//! - Death (the agent exits, the holder dies, a start fails): wait 5 s, then
//!   start again with the backend's resume arguments (`--resume <session>`).
//!   Three restarts within 10 minutes and it still dies → `failed`; no
//!   session id to resume → `failed` at once. Never a fresh start after the
//!   first (P6). Restart counts live in memory only.
//!
//! Gate 8:
//! - Every state change is shown in the fleet view ([`Fleet`]): `starting`
//!   (starting, or waiting to restart), `unknown` (running; working or idle
//!   needs a driver), `failed`.
//! - A `failed` instance is a needs-you item `instance-failed:<id>` (P5),
//!   waiting since this daemon first saw it failed. Its action is `retry`,
//!   except a codex/opencode instance whose session was started: it has no
//!   session id to resume and is never started fresh again (P6), so it has
//!   no action.
//! - `retry` ([`Event::Retry`]; `handlers` checked the caller, took the item
//!   off the list and answered the operator already):
//!   `Shutdown` of the holder kept for its last screen (H9), then status
//!   `running` if the session was started (claude resumes it) or `new`
//!   (a fresh start), a new restart budget, and a start.
//!
//! Must NOT: kill or respawn holders on daemon shutdown; start an agent
//! fresh once it has run.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agend_core::model::{Backend, DEFAULT_TEAM};
use agend_core::protocol::client::{
    AgentState, AttentionAction, AttentionRequiredData, InstanceView, TaskView,
};
use agend_core::protocol::holder::ExitedData;
use agend_core::traits::HolderLaunch;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::boot::{BootAction, plan_boot};
use crate::fleet::{Fleet, failed_attention_id};
use crate::log;
use crate::runtime::{HolderEvent, HolderRuntime, SpawnOutcome, Started, files};
use crate::store::{Instance, InstanceStatus, SqliteStore, task_row};

/// Wait between a death and the restart.
pub const RESTART_DELAY: Duration = Duration::from_secs(5);
/// Restarts allowed within [`RESTART_WINDOW_MS`] before giving up.
pub const MAX_RESTARTS: usize = 3;
pub const RESTART_WINDOW_MS: u64 = 10 * 60 * 1000;

/// Restarts of one instance in the last 10 minutes (in memory, P6).
#[derive(Debug, Default, Clone)]
pub struct RestartBudget {
    restarts: Vec<u64>,
}

impl RestartBudget {
    /// After a death at `now_unix_ms`: `Some(n)` for the n-th restart in
    /// the window, `None` when [`MAX_RESTARTS`] were used: give up.
    pub fn next(&mut self, now_unix_ms: u64) -> Option<usize> {
        self.restarts
            .retain(|&at| now_unix_ms.saturating_sub(at) < RESTART_WINDOW_MS);
        if self.restarts.len() >= MAX_RESTARTS {
            return None;
        }
        self.restarts.push(now_unix_ms);
        Some(self.restarts.len())
    }
}

/// The session arguments a start adds to the instance's base arguments:
/// claude gets `--session-id <id>` on its first start and `--resume <id>`
/// after that; codex and opencode have no session id before gates 7 and 12,
/// so they can start fresh once and never resume.
pub fn session_args(instance: &Instance, resume: bool) -> Result<Vec<String>, String> {
    match (instance.backend, &instance.session_id, resume) {
        (Backend::Claude, Some(id), false) => Ok(vec!["--session-id".into(), id.clone()]),
        (Backend::Claude, Some(id), true) => Ok(vec!["--resume".into(), id.clone()]),
        (_, _, false) => Ok(Vec::new()),
        (backend, _, true) => Err(format!(
            "no session id to resume ({} cannot resume yet)",
            backend.as_str()
        )),
    }
}

/// The launch of `instance`, fresh or resuming.
pub fn launch(instance: &Instance, resume: bool) -> Result<HolderLaunch, String> {
    let mut args = instance.args.clone();
    args.extend(session_args(instance, resume)?);
    Ok(HolderLaunch {
        instance_id: instance.id.clone(),
        backend: instance.backend,
        executable: instance.program.clone(),
        args,
        working_directory: instance.working_directory.clone(),
    })
}

#[derive(Debug)]
pub enum Event {
    Holder(HolderEvent),
    StartFailed {
        id: String,
        generation: u64,
        error: String,
    },
    Restart {
        id: String,
        generation: u64,
    },
    Housekeeping,
    /// The operator chose `retry` for this needs-you item of a `failed`
    /// instance; it is already off the list. When the retry cannot be done
    /// the item is listed again.
    Retry {
        item: Box<AttentionRequiredData>,
    },
    Stop(&'static str),
}

/// The needs-you item of a `failed` instance (P5).
pub fn failed_item(instance: &Instance, reason: &str, since_unix_ms: u64) -> AttentionRequiredData {
    let id = &instance.id;
    // codex/opencode have no session id to resume (gates 7, 12): once their
    // session started they are never started fresh again (P6).
    let retry = instance.backend == Backend::Claude || !instance.session_started;
    AttentionRequiredData {
        reason: format!("{id} failed: {reason}"),
        task_id: None,
        ask: None,
        recap: None,
        attention_id: Some(failed_attention_id(id)),
        unblocks: Some(0),
        waiting_since_unix_ms: Some(since_unix_ms),
        if_ignored: Some(if retry {
            format!("{id} stays stopped")
        } else {
            "delete and re-add the instance (gate 9)".into()
        }),
        actions: if retry {
            vec![AttentionAction::Retry]
        } else {
            Vec::new()
        },
        instance_id: Some(id.clone()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Up,
    /// Waiting for the restart (the n-th).
    Restarting(usize),
    Failed,
}

struct Watch {
    generation: u64,
    state: State,
    budget: RestartBudget,
}

/// Counts for the ready line (gate 6 P5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BootReport {
    pub instances: usize,
    pub recovered: usize,
    pub started: usize,
    pub orphans: usize,
}

pub struct Supervisor {
    home: PathBuf,
    store: SqliteStore,
    runtime: HolderRuntime,
    events: UnboundedSender<Event>,
    watches: BTreeMap<String, Watch>,
    fleet: Arc<Fleet>,
}

/// The screen's last non-empty line, for the log.
fn last_line(screen: &str) -> &str {
    screen
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
}

fn describe_exit(exited: &ExitedData) -> String {
    match (&exited.code, &exited.signal) {
        (Some(code), _) => format!("code={code}"),
        (None, Some(signal)) => format!("signal={signal}"),
        (None, None) => "unknown status".into(),
    }
}

impl Supervisor {
    pub fn new(
        store: SqliteStore,
        runtime: HolderRuntime,
        events: UnboundedSender<Event>,
        fleet: Arc<Fleet>,
    ) -> Self {
        Self {
            home: runtime.home().to_path_buf(),
            store,
            runtime,
            events,
            watches: BTreeMap::new(),
            fleet,
        }
    }

    /// Shows `id` in the fleet view with `state` (and `summary` on its
    /// `instance_changed` event).
    fn show(&self, instance: &Instance, state: AgentState, summary: String) {
        self.fleet.set_instance(
            InstanceView {
                instance_id: instance.id.clone(),
                team_id: DEFAULT_TEAM.into(),
                backend: instance.backend.as_str().into(),
                state,
            },
            summary,
        );
    }

    /// Changes the state of an instance already in the fleet view.
    fn set_state(&self, id: &str, state: AgentState, summary: String) {
        if let Some(mut view) = self.fleet.instance(id) {
            view.state = state;
            self.fleet.set_instance(view, summary);
        }
    }

    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// Runs the boot plan.
    pub async fn boot(&mut self) -> Result<BootReport, String> {
        let instances = self
            .store
            .instances()
            .await
            .map_err(|e| format!("read instances: {e}"))?;
        let running = files::running_holders(self.runtime.home())
            .map_err(|e| format!("scan run/holders: {e}"))?;
        let running_ids: Vec<String> = running.iter().map(|(id, _)| id.clone()).collect();
        let tasks = self
            .store
            .tasks()
            .await
            .map_err(|e| format!("read tasks: {e}"))?;
        self.fleet.set_tasks(
            tasks
                .into_iter()
                .map(|t| TaskView {
                    task_id: t.id,
                    title: t.title,
                    team_id: t.team_id,
                    status: task_row::status_text(t.status).into(),
                    assignee: t.assignee,
                    stages: Vec::new(),
                    current_stage: None,
                })
                .collect(),
        );
        let now = log::now_unix_ms();
        for instance in &instances {
            if instance.status == InstanceStatus::Failed {
                self.show(instance, AgentState::Failed, "failed".into());
                let reason = "it failed before this daemon started";
                self.fleet.raise(failed_item(instance, reason, now));
            } else {
                self.show(instance, AgentState::Starting, "starting".into());
            }
        }
        let mut report = BootReport {
            instances: instances.len(),
            ..BootReport::default()
        };
        for action in plan_boot(&instances, &running_ids) {
            match action {
                BootAction::Reconnect { id } => {
                    report.recovered += 1;
                    let pid = running.iter().find(|(i, _)| *i == id).map_or(0, |r| r.1);
                    let instance = instances.iter().find(|i| i.id == id).expect("planned");
                    self.reconnect(instance, pid).await;
                }
                BootAction::Start { id, resume } => {
                    report.started += 1;
                    let instance = instances.iter().find(|i| i.id == id).expect("planned");
                    self.start(instance, resume, None).await;
                }
                BootAction::Orphan { id } => {
                    report.orphans += 1;
                    match self.runtime.stop(&id).await {
                        Ok(()) => log::line(&format!("orphan {id}: Shutdown sent")),
                        Err(e) => log::line(&format!("orphan {id}: Shutdown failed: {e}")),
                    }
                }
            }
        }
        Ok(report)
    }

    /// Reconnects to a running holder and re-sends `Spawn` (P3): a holder
    /// that already ran its agent answers `already_spawned`. A `new`
    /// instance's `Spawn` starts the session (`--session-id`); a `running`
    /// one's resumes when the backend can (codex and opencode never restart,
    /// so their holder can only lack an agent if the agent never ran).
    async fn reconnect(&mut self, instance: &Instance, pid: u32) {
        let id = instance.id.clone();
        let resume = instance.status == InstanceStatus::Running;
        let launch = launch(instance, resume)
            .or_else(|_| launch(instance, false))
            .expect("a fresh launch always exists");
        match self.runtime.attach(&launch, pid).await {
            Ok(started) => {
                if !resume {
                    self.mark_running(&id).await;
                }
                let note = match started.attached.spawn {
                    Some(SpawnOutcome::Spawned) => "; it had no agent, started one",
                    _ => "",
                };
                log::line(&format!(
                    "{id}: reconnected to holder pid={pid}{note}; screen: {}",
                    last_line(&started.attached.screen)
                ));
                self.show(
                    instance,
                    AgentState::Unknown,
                    format!("running (holder pid={pid})"),
                );
                self.watch(&id, started.generation);
            }
            Err(e) => {
                let generation = self.watches.get(&id).map_or(0, |w| w.generation);
                self.watch(&id, generation);
                let _ = self.events.send(Event::StartFailed {
                    id,
                    generation,
                    error: format!("cannot reconnect: {e}"),
                });
            }
        }
    }

    /// Starts `instance` fresh (its first start) or resuming. `restart` is
    /// the n-th restart, for the log.
    async fn start(&mut self, instance: &Instance, resume: bool, restart: Option<usize>) {
        let id = instance.id.clone();
        let launch = match launch(instance, resume) {
            Ok(launch) => launch,
            Err(e) => return self.fail(&id, &e).await,
        };
        let session = session_args(instance, resume)
            .map(|a| a.join(" "))
            .unwrap_or_default();
        let what = match restart {
            Some(n) => format!("restart {n}/{MAX_RESTARTS} {session}"),
            None => format!("start {session}"),
        };
        log::line(&format!("{id}: {}", what.trim_end()));
        self.show(instance, AgentState::Starting, what.trim_end().to_owned());
        match self.runtime.start(&launch).await {
            Ok(Started {
                handle, generation, ..
            }) => {
                let pid = handle.process_id.unwrap_or(0);
                log::line(&format!("{id}: holder pid={pid} started"));
                self.show(
                    instance,
                    AgentState::Unknown,
                    format!("running (holder pid={pid})"),
                );
                if !resume {
                    self.mark_running(&id).await;
                }
                self.watch(&id, generation);
            }
            Err(e) => {
                let generation = self.watches.get(&id).map_or(0, |w| w.generation);
                self.watch(&id, generation);
                let _ = self.events.send(Event::StartFailed {
                    id,
                    generation,
                    error: e.to_string(),
                });
            }
        }
    }

    /// Records `running` once the first `Spawn` was acknowledged: the
    /// session exists and every later start resumes it.
    async fn mark_running(&mut self, id: &str) {
        if let Err(e) = self
            .store
            .set_instance_status(id, InstanceStatus::Running)
            .await
        {
            log::line(&format!("{id}: cannot record running: {e}"));
        }
    }

    fn watch(&mut self, id: &str, generation: u64) {
        let watch = self.watches.entry(id.to_owned()).or_insert(Watch {
            generation,
            state: State::Up,
            budget: RestartBudget::default(),
        });
        watch.generation = generation;
        watch.state = State::Up;
    }

    /// Gives up on `id`. Its holder (if any) keeps the last screen for a
    /// human; the daemon drops its connection, so the holder's idle exit
    /// (gate 4 P2, 24 h without a client) can end it.
    async fn fail(&mut self, id: &str, reason: &str) {
        if let Some(watch) = self.watches.get_mut(id) {
            watch.state = State::Failed;
        }
        self.runtime.detach(id);
        if let Err(e) = self
            .store
            .set_instance_status(id, InstanceStatus::Failed)
            .await
        {
            log::line(&format!("{id}: cannot record failed: {e}"));
        }
        log::line(&format!("{id} failed: {reason}"));
        self.set_state(id, AgentState::Failed, reason.to_owned());
        match self.store.instance(id).await {
            Ok(Some(instance)) => {
                self.fleet
                    .raise(failed_item(&instance, reason, log::now_unix_ms()));
            }
            Ok(None) => {}
            Err(e) => log::line(&format!("{id}: cannot read the instance: {e}")),
        }
    }

    /// Starts a `failed` instance again (P5): stops the holder it left,
    /// then resumes a started session or starts one, with a new budget. If
    /// that cannot begin, the needs-you item is listed again.
    async fn retry(&mut self, item: AttentionRequiredData) {
        let Some(id) = item.instance_id.clone() else {
            return;
        };
        let id = id.as_str();
        log::line(&format!("{id}: retry requested by the operator"));
        if let Err(e) = self.runtime.stop(id).await {
            // `fail` lists a new item with this reason.
            return self
                .fail(id, &format!("cannot stop its old holder: {e}"))
                .await;
        }
        let instance = match self.store.instance(id).await {
            Ok(Some(instance)) => instance,
            Ok(None) => return log::line(&format!("{id}: no longer in the DB; not retried")),
            Err(e) => {
                log::line(&format!("{id}: cannot read the instance: {e}; not retried"));
                return self.fleet.raise(item);
            }
        };
        let status = if instance.session_started {
            InstanceStatus::Running
        } else {
            InstanceStatus::New
        };
        if let Err(e) = self.store.set_instance_status(id, status).await {
            log::line(&format!(
                "{id}: cannot record {}: {e}; not retried",
                status.as_str()
            ));
            return self.fleet.raise(item);
        }
        self.watches.remove(id);
        let instance = Instance { status, ..instance };
        self.start(&instance, instance.session_started, None).await;
    }

    /// A death of the current generation: plan the restart or give up.
    async fn died(&mut self, id: &str, generation: u64, what: String) {
        let Some(watch) = self.watches.get(id) else {
            return;
        };
        if watch.generation != generation || watch.state != State::Up {
            return;
        }
        log::line(&what);
        let instance = match self.store.instance(id).await {
            Ok(Some(instance)) => instance,
            Ok(None) => {
                log::line(&format!("{id}: no longer in the DB; not restarted"));
                self.watches.remove(id);
                return;
            }
            Err(e) => return log::line(&format!("{id}: cannot read the instance: {e}")),
        };
        let resume = instance.status == InstanceStatus::Running;
        if let (true, Err(reason)) = (resume, session_args(&instance, true)) {
            return self.fail(id, &reason).await;
        }
        let now = log::now_unix_ms();
        let watch = self.watches.get_mut(id).expect("checked above");
        match watch.budget.next(now) {
            None => {
                let reason = format!(
                    "restarted {MAX_RESTARTS} times in 10m and it still died; not restarting"
                );
                self.fail(id, &reason).await;
            }
            Some(n) => {
                watch.state = State::Restarting(n);
                let summary = format!("died; restart {n}/{MAX_RESTARTS} in 5 s");
                self.set_state(id, AgentState::Starting, summary);
                let events = self.events.clone();
                let id = id.to_owned();
                tokio::spawn(async move {
                    tokio::time::sleep(RESTART_DELAY).await;
                    let _ = events.send(Event::Restart { id, generation });
                });
            }
        }
    }

    async fn restart(&mut self, id: &str, generation: u64) {
        let Some(watch) = self.watches.get(id) else {
            return;
        };
        let State::Restarting(n) = watch.state else {
            return;
        };
        if watch.generation != generation {
            return;
        }
        // The agent may have ended in a holder that still runs: a holder
        // never starts a second agent, so it is stopped first.
        if let Err(e) = self.runtime.stop(id).await {
            return self
                .fail(id, &format!("cannot stop its old holder: {e}"))
                .await;
        }
        match self.store.instance(id).await {
            Ok(Some(instance)) => {
                let resume = instance.status == InstanceStatus::Running;
                self.start(&instance, resume, Some(n)).await;
            }
            Ok(None) => {
                log::line(&format!("{id}: no longer in the DB; not restarted"));
                self.watches.remove(id);
            }
            Err(e) => log::line(&format!("{id}: cannot read the instance: {e}")),
        }
    }

    /// Handles events until a stop signal. Returns the signal's name.
    pub async fn run(&mut self, events: &mut UnboundedReceiver<Event>) -> &'static str {
        while let Some(event) = events.recv().await {
            match event {
                Event::Holder(HolderEvent::AgentExited {
                    id,
                    generation,
                    exited,
                }) => {
                    let what = format!("agent {id} exited ({})", describe_exit(&exited));
                    self.died(&id, generation, what).await;
                }
                Event::Holder(HolderEvent::HolderGone { id, generation }) => {
                    let what = format!("holder {id} died");
                    self.died(&id, generation, what).await;
                }
                Event::StartFailed {
                    id,
                    generation,
                    error,
                } => {
                    let what = format!("{id}: start failed: {error}");
                    self.died(&id, generation, what).await;
                }
                Event::Restart { id, generation } => self.restart(&id, generation).await,
                Event::Housekeeping => {
                    crate::housekeeping::run(&self.store, &self.home, log::now_unix_ms()).await;
                }
                Event::Retry { item } => self.retry(*item).await,
                Event::Stop(signal) => return signal,
            }
        }
        "channel closed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn instance(backend: Backend, session: Option<&str>) -> Instance {
        Instance {
            id: "g6-1".into(),
            backend,
            program: "/bin/bash".into(),
            args: vec!["-c".into(), "exit 1".into(), "agent".into()],
            working_directory: "/tmp".into(),
            session_id: session.map(str::to_owned),
            status: InstanceStatus::Running,
            session_started: false,
        }
    }

    #[test]
    fn three_restarts_in_ten_minutes_then_give_up() {
        let mut budget = RestartBudget::default();
        let t = 1_000_000;
        assert_eq!(budget.next(t), Some(1));
        assert_eq!(budget.next(t + MIN), Some(2));
        assert_eq!(budget.next(t + 2 * MIN), Some(3));
        assert_eq!(budget.next(t + 3 * MIN), None, "4th death in 10m");
        assert_eq!(
            budget.next(t + 4 * MIN),
            None,
            "still within 10m of all three"
        );
    }

    #[test]
    fn restarts_older_than_ten_minutes_no_longer_count() {
        let mut budget = RestartBudget::default();
        let t = 1_000_000;
        for n in 1..=3 {
            assert_eq!(budget.next(t + n * MIN), Some(n as usize));
        }
        // The first restart (t + 1m) is 10 minutes old at t + 11m.
        assert_eq!(budget.next(t + 11 * MIN), Some(3));
        assert_eq!(budget.next(t + 11 * MIN + 1), None);
    }

    #[test]
    fn claude_starts_fresh_once_then_only_resumes() {
        let claude = instance(Backend::Claude, Some("s-abc"));
        let fresh = launch(&claude, false).unwrap();
        assert_eq!(fresh.args[3..], ["--session-id", "s-abc"]);
        let resumed = launch(&claude, true).unwrap();
        assert_eq!(resumed.args[3..], ["--resume", "s-abc"]);
        assert_eq!(resumed.args[..3], claude.args[..]);
        assert_eq!(resumed.executable, "/bin/bash");
    }

    /// P5: `retry` for claude always; for codex/opencode only while their
    /// session never started (no session id to resume, never fresh again).
    #[test]
    fn a_failed_instance_is_a_needs_you_item_with_retry_unless_it_cannot_resume() {
        let table = [
            (Backend::Claude, true, true),
            (Backend::Claude, false, true),
            (Backend::Codex, false, true),
            (Backend::Opencode, false, true),
            (Backend::Codex, true, false),
            (Backend::Opencode, true, false),
        ];
        for (backend, started, retry) in table {
            let mut inst = instance(backend, None);
            inst.id = "g8-2".into();
            inst.session_started = started;
            let item = failed_item(&inst, "it died", 42);
            assert_eq!(item.attention_id.as_deref(), Some("instance-failed:g8-2"));
            assert_eq!(item.instance_id.as_deref(), Some("g8-2"));
            assert_eq!(
                (item.unblocks, item.waiting_since_unix_ms),
                (Some(0), Some(42))
            );
            assert_eq!(item.reason, "g8-2 failed: it died");
            let (actions, if_ignored) = if retry {
                (vec![AttentionAction::Retry], "g8-2 stays stopped")
            } else {
                (vec![], "delete and re-add the instance (gate 9)")
            };
            assert_eq!(item.actions, actions, "{backend:?} started={started}");
            assert_eq!(item.if_ignored.as_deref(), Some(if_ignored));
        }
    }

    #[test]
    fn without_a_session_id_there_is_no_resume_only_failed() {
        for backend in [Backend::Codex, Backend::Opencode] {
            let inst = instance(backend, None);
            assert_eq!(launch(&inst, false).unwrap().args, inst.args);
            let error = launch(&inst, true).unwrap_err();
            assert!(error.starts_with("no session id to resume"), "{error}");
        }
        let error = launch(&instance(Backend::Claude, None), true).unwrap_err();
        assert!(error.starts_with("no session id to resume"), "{error}");
    }
}
