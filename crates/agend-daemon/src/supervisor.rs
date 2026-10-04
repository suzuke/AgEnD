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
//! Gate 7 (codex):
//! - A codex agent is the `sh` wrapper (`driver::codex::launch`); after its
//!   holder is up the codex driver connects (ready, thread, `$GO`); a
//!   failure there, or an app-server gone for 20 s, is a death like any
//!   other. codex now resumes its thread, so a `failed` codex instance has
//!   `retry` unless migration 0004 marked it `legacy_no_thread`.
//! - The sweep (P2): every time a codex holder is found dead (before the
//!   restart-or-`failed` decision), before a new codex holder starts, and at
//!   boot for a `failed` one whose holder is gone, while `agent_pid` is set:
//!   SIGKILL to the old agent's group when it is still this instance's
//!   (`driver::codex::sweep`), then `agent_pid` is cleared.
//!
//! Gate 12A extends these sweep points to Claude: require a gone holder
//! and exact session argv or this daemon's channel executable/instance pair.
//! A live holder or failed holder inspection never authorizes a signal.
//!
//! Gate 9 (P6, P7):
//! - `instance_add` ([`Event::Add`]): checks the name, the backend, that no
//!   row and no running holder has the name; writes the row (`new`, claude
//!   gets a session id), publishes the starting view, answers, then starts it like at boot.
//! - `instance_remove` ([`Event::Remove`]): stops watching, closes the
//!   link, `Shutdown` to the holder (at most 5 s; an unreachable holder is
//!   left to the next boot's orphan sweep), removes the row and takes it out
//!   of the fleet view. The workspace stays.
//! - [`Event::Exec`] (a restart whose preflight passed): [`Supervisor::run`]
//!   returns [`Stopped::Exec`]; the daemon stops as on SIGINT, then `exec`s.
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
use agend_core::protocol::client::{InstanceAddedData, error_code};
use agend_core::protocol::holder::ExitedData;
use agend_core::traits::HolderLaunch;
use std::path::Path;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use crate::boot::{BootAction, plan_boot};
use crate::driver::codex::sweep::{self, Markers};
use crate::driver::codex::{CodexDriver, CodexEvent, launch as codex_launch};
use crate::fleet::{Fleet, failed_attention_id};
use crate::log;
use crate::runtime::{HolderEvent, HolderRuntime, SpawnOutcome, Started, files};
use crate::store::instances::{new_session_id, validate_id};
use crate::store::{Instance, InstanceStatus, SqliteStore, task_row};

/// How long `instance_remove` waits for the holder to go (gate 9 P6).
pub const REMOVE_WITHIN: Duration = Duration::from_secs(5);

/// A refused operator request: error code and message.
pub type Refusal = (&'static str, String);

/// `instance_add` as the handler passed it on.
#[derive(Debug)]
pub struct AddRequest {
    pub id: String,
    pub backend: String,
    pub working_directory: Option<String>,
    pub program: Option<String>,
    pub args: Vec<String>,
}

/// Why [`Supervisor::run`] returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stopped {
    Signal(&'static str),
    /// `agend daemon restart`: stop, then `exec` this binary.
    Exec(PathBuf),
}

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
/// after that; codex gets none (its TUI always resumes the thread the
/// daemon created, through `$GO`, gate 7 P3); opencode has no session id
/// before gate 12, so it can start fresh once and never resume.
pub fn session_args(instance: &Instance, resume: bool) -> Result<Vec<String>, String> {
    match (instance.backend, &instance.session_id, resume) {
        (Backend::Claude, Some(id), false) => Ok(vec!["--session-id".into(), id.clone()]),
        (Backend::Claude, Some(id), true) => Ok(vec!["--resume".into(), id.clone()]),
        (Backend::Codex, _, _) | (_, _, false) => Ok(Vec::new()),
        (backend, _, true) => Err(format!(
            "no session id to resume ({} cannot resume yet)",
            backend.as_str()
        )),
    }
}

/// What a start does about the session, for the log.
fn describe_session(instance: &Instance, resume: bool) -> String {
    match (instance.backend, &instance.session_id) {
        (Backend::Codex, Some(thread)) => format!("(codex thread {thread})"),
        (Backend::Codex, None) => "(codex: new thread)".into(),
        _ => session_args(instance, resume)
            .map(|a| a.join(" "))
            .unwrap_or_default(),
    }
}

/// The launch of `instance` under `home`, fresh or resuming. codex runs the
/// gate 7 wrapper (`/bin/sh`).
pub fn launch(home: &Path, instance: &Instance, resume: bool) -> Result<HolderLaunch, String> {
    let (executable, args) = if instance.backend == Backend::Codex {
        (
            codex_launch::SHELL.to_owned(),
            codex_launch::wrapper_args(home, instance)?,
        )
    } else {
        let mut args = if crate::driver::claude::launch::applies(instance) {
            crate::driver::claude::launch::args(home, instance)?
        } else {
            instance.args.clone()
        };
        args.extend(session_args(instance, resume)?);
        (instance.program.clone(), args)
    };
    Ok(HolderLaunch {
        instance_id: instance.id.clone(),
        backend: instance.backend,
        executable,
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
    /// An instance's codex app-server is gone (gate 7).
    Codex(CodexEvent),
    Housekeeping,
    /// The operator chose `retry` for this needs-you item of a `failed`
    /// instance; it is already off the list. When the retry cannot be done
    /// the item is listed again.
    Retry {
        item: Box<AttentionRequiredData>,
    },
    /// The operator adds an instance (gate 9).
    Add {
        request: AddRequest,
        reply: oneshot::Sender<Result<InstanceAddedData, Refusal>>,
    },
    /// The operator removes an instance (gate 9).
    Remove {
        id: String,
        reply: oneshot::Sender<Result<(), Refusal>>,
    },
    /// The restart preflight passed: stop and `exec` this binary.
    Exec(PathBuf),
    Stop(&'static str),
}

/// Why a codex row from before gate 7 is `failed` (gate 7 P3).
pub const LEGACY_NO_THREAD: &str =
    "codex instance from before gate 7 has no thread id; a human decides";

/// The needs-you item of a `failed` instance (P5).
pub fn failed_item(instance: &Instance, reason: &str, since_unix_ms: u64) -> AttentionRequiredData {
    let id = &instance.id;
    // opencode has no session id to resume (gate 12): once its session
    // started it is never started fresh again (P6). codex resumes its
    // thread since gate 7, except a row from before it (P3).
    let retry = match instance.backend {
        Backend::Claude => true,
        Backend::Codex => !instance.legacy_no_thread,
        Backend::Opencode => !instance.session_started,
    };
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
    store: Arc<SqliteStore>,
    runtime: HolderRuntime,
    codex: CodexDriver,
    pipeline: Option<crate::pipeline::Handle>,
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
        store: Arc<SqliteStore>,
        runtime: HolderRuntime,
        codex: CodexDriver,
        events: UnboundedSender<Event>,
        fleet: Arc<Fleet>,
    ) -> Self {
        Self {
            home: runtime.home().to_path_buf(),
            store,
            runtime,
            codex,
            pipeline: None,
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
                working_directory: Some(instance.working_directory.clone()),
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

    pub fn set_pipeline(&mut self, pipeline: crate::pipeline::Handle) {
        self.pipeline = Some(pipeline);
    }

    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// The sweep (gate 7 P2 / D40 P9) of an instance whose holder is gone, or
    /// before its new holder starts: only while `agent_pid` is set; clears
    /// it afterwards.
    async fn sweep(&self, instance: &Instance, why: &str) {
        let Some(pgid) = instance.agent_pid else {
            return;
        };
        let id = &instance.id;
        if instance.backend == Backend::Claude {
            match files::running(self.runtime.home(), id) {
                Ok(None) => {}
                Ok(Some(pid)) => {
                    log::line(&format!(
                        "{id}: sweep refused; holder pid={pid} is still running"
                    ));
                    return;
                }
                Err(error) => {
                    log::line(&format!(
                        "{id}: sweep refused; cannot establish holder death: {error}"
                    ));
                    return;
                }
            }
        }
        let swept = match instance.backend {
            Backend::Codex => sweep::sweep(
                pgid,
                &Markers {
                    socket: codex_launch::socket_path(&self.home, id)
                        .display()
                        .to_string(),
                    thread: instance.session_id.clone(),
                },
            ),
            Backend::Claude => crate::driver::claude::sweep::sweep(
                pgid,
                &crate::driver::claude::sweep::Markers {
                    session: instance.session_id.as_deref(),
                    instance: id,
                    agend: self.runtime.executable(),
                },
            ),
            _ => return,
        };
        log::line(&format!(
            "{id}: sweep of agent group {pgid} ({why}): {swept}"
        ));
        if let Err(e) = self.store.set_agent_pid(id, None).await {
            log::line(&format!("{id}: cannot clear agent_pid: {e}"));
        }
    }

    async fn record_agent_pid(&self, id: &str, spawn: Option<SpawnOutcome>) {
        if let Some(SpawnOutcome::Spawned {
            agent_pid: Some(pid),
        }) = spawn
            && let Err(e) = self.store.set_agent_pid(id, Some(pid)).await
        {
            log::line(&format!("{id}: cannot record agent_pid: {e}"));
        }
    }

    /// For a codex instance whose holder just started or was reconnected:
    /// the driver's readiness, thread and `$GO` (gate 7 P2, P3), in the
    /// background (up to 20 s + 30 s; the event loop and Ctrl-C do not
    /// wait). A failure is a death of this generation.
    fn connect_codex(&self, instance: &Instance, generation: u64) {
        if instance.backend != Backend::Codex || instance.delivery == "inbox" {
            return;
        }
        let (codex, events, id) = (self.codex.clone(), self.events.clone(), instance.id.clone());
        tokio::spawn(async move {
            match codex.connect(&id, generation).await {
                Ok(Some(lines)) => {
                    for line in lines {
                        log::line(&format!("{id}: {line}"));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = events.send(Event::StartFailed {
                        id,
                        generation,
                        error: format!("codex: {e}"),
                    });
                }
            }
        });
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
                    pipeline: None,
                })
                .collect(),
        );
        let now = log::now_unix_ms();
        for instance in &instances {
            if instance.status == InstanceStatus::Failed {
                self.show(instance, AgentState::Failed, "failed".into());
                let reason = if instance.legacy_no_thread {
                    log::line(&format!("{} failed: {LEGACY_NO_THREAD}", instance.id));
                    LEGACY_NO_THREAD
                } else {
                    "it failed before this daemon started"
                };
                self.fleet.raise(failed_item(instance, reason, now));
                // Its holder is left alone while it runs (gate 6 H8).
                if !running_ids.contains(&instance.id) {
                    self.sweep(instance, "failed, holder gone").await;
                }
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
        if !self.prepare_claude(instance).await {
            return;
        }
        let resume = instance.status == InstanceStatus::Running;
        let launch = match launch(&self.home, instance, resume)
            .or_else(|_| launch(&self.home, instance, false))
        {
            Ok(launch) => launch,
            Err(e) => return self.fail(&id, &e).await,
        };
        match self.runtime.attach(&launch, pid).await {
            Ok(started) => {
                if !resume {
                    self.mark_running(&id).await;
                }
                self.record_agent_pid(&id, started.attached.spawn).await;
                let note = match started.attached.spawn {
                    Some(SpawnOutcome::Spawned { .. }) => "; it had no agent, started one",
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
                self.connect_codex(instance, started.generation);
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
        if !self.prepare_claude(instance).await {
            return;
        }
        let launch = match launch(&self.home, instance, resume) {
            Ok(launch) => launch,
            Err(e) => return self.fail(&id, &e).await,
        };
        self.sweep(instance, "before a new holder").await;
        if instance.backend == Backend::Codex {
            match codex_launch::prepare(&self.home, &id) {
                Ok(removed) if !removed.is_empty() => {
                    log::line(&format!("{id}: removed {}", removed.join(", ")))
                }
                Ok(_) => {}
                Err(e) => log::line(&format!("{id}: cannot remove the old codex files: {e}")),
            }
        }
        let session = describe_session(instance, resume);
        let what = match restart {
            Some(n) => format!("restart {n}/{MAX_RESTARTS} {session}"),
            None => format!("start {session}"),
        };
        log::line(&format!("{id}: {}", what.trim_end()));
        self.show(instance, AgentState::Starting, what.trim_end().to_owned());
        match self.runtime.start(&launch).await {
            Ok(Started {
                handle,
                generation,
                attached,
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
                self.record_agent_pid(&id, attached.spawn).await;
                self.watch(&id, generation);
                self.connect_codex(instance, generation);
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

    /// Configuration conflicts follow the same restart budget as Spawn errors.
    async fn prepare_claude(&mut self, instance: &Instance) -> bool {
        match crate::driver::claude::launch::prepare(
            &self.store,
            &self.home,
            self.runtime.executable(),
            instance,
        )
        .await
        {
            Ok(()) => true,
            Err(error) => {
                let id = instance.id.clone();
                let generation = self.watches.get(&id).map_or(0, |w| w.generation);
                self.show(
                    instance,
                    AgentState::Starting,
                    format!("Claude configuration: {error}"),
                );
                self.watch(&id, generation);
                let _ = self.events.send(Event::StartFailed {
                    id,
                    generation,
                    error,
                });
                false
            }
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
        self.codex.disconnect(id);
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
    /// `holder_gone`: the holder itself died (the codex sweep runs first).
    async fn died(&mut self, id: &str, generation: u64, what: String, holder_gone: bool) {
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
        // The old link would otherwise reconnect to the next app-server on
        // the same socket path while it retries.
        self.codex.disconnect(id);
        if holder_gone {
            self.sweep(&instance, "holder died").await;
        }
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

    /// `instance_add` (gate 9 P6): the row, then the start. Answers before
    /// the start, which is seen in the fleet view like any other.
    async fn add(
        &mut self,
        request: AddRequest,
        reply: oneshot::Sender<Result<InstanceAddedData, Refusal>>,
    ) {
        let instance = match self.new_instance(request).await {
            Ok(instance) => instance,
            Err(refusal) => {
                let _ = reply.send(Err(refusal));
                return;
            }
        };
        let id = instance.id.clone();
        log::line(&format!(
            "{id}: added ({}, {}, {})",
            instance.backend.as_str(),
            instance.program,
            instance.working_directory
        ));
        // The accepted response guarantees the immediately following fleet read sees the row.
        self.show(&instance, AgentState::Starting, "added; starting".into());
        let _ = reply.send(Ok(InstanceAddedData {
            instance_id: id,
            session_id: instance.session_id.clone(),
            working_directory: instance.working_directory.clone(),
        }));
        self.start(&instance, false, None).await;
    }

    async fn new_instance(&self, request: AddRequest) -> Result<Instance, Refusal> {
        let invalid = |message: String| (error_code::INVALID_REQUEST, message);
        let id = request.id;
        validate_id(&id).map_err(|_| {
            invalid(format!(
                "invalid name {id:?}: use 1-24 characters from a-z 0-9 - (e.g. dev-1)"
            ))
        })?;
        let backend = Backend::parse(&request.backend).ok_or_else(|| {
            invalid(format!(
                "unknown backend {:?}: use claude, codex or opencode",
                request.backend
            ))
        })?;
        let exists = |message: String| (error_code::INSTANCE_EXISTS, message);
        let read = |e| invalid(format!("cannot read instances: {e}"));
        if self.store.instance(&id).await.map_err(read)?.is_some() {
            return Err(exists(format!("name {id} is already used")));
        }
        if let Ok(Some(pid)) = files::running(&self.home, &id) {
            return Err(exists(format!(
                "name {id} is already used: its holder is still running (run/holders/{id}.lock, pid {pid}); wait a few seconds or restart the daemon to sweep it"
            )));
        }
        let working_directory = match request.working_directory {
            Some(dir) if Path::new(&dir).is_absolute() && Path::new(&dir).is_dir() => dir,
            Some(dir) => return Err(invalid(format!("no directory {dir} (it must exist)"))),
            None => {
                let dir = self.home.join("workspace").join(&id);
                std::os::unix::fs::DirBuilderExt::mode(
                    std::fs::DirBuilder::new().recursive(true),
                    0o700,
                )
                .create(&dir)
                .map_err(|e| invalid(format!("cannot create {}: {e}", dir.display())))?;
                dir.display().to_string()
            }
        };
        let session_id = match backend {
            Backend::Claude => Some(
                new_session_id().map_err(|e| invalid(format!("cannot make a session id: {e}")))?,
            ),
            Backend::Codex | Backend::Opencode => None,
        };
        let instance = Instance {
            id: id.clone(),
            backend,
            program: request
                .program
                .unwrap_or_else(|| backend.as_str().to_owned()),
            args: request.args,
            working_directory,
            session_id,
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        };
        self.store
            .add_instance(&instance)
            .await
            .map_err(|e| match e {
                crate::store::StoreError::Exists(_) => exists(format!("name {id} is already used")),
                e => invalid(format!("cannot add {id}: {e}")),
            })?;
        Ok(instance)
    }

    /// `instance_remove` (gate 9 P6): stop it, remove the row, keep the
    /// workspace.
    async fn remove(&mut self, id: &str) -> Result<(), Refusal> {
        let read = |e| {
            (
                error_code::INVALID_REQUEST,
                format!("cannot read {id}: {e}"),
            )
        };
        let Some(instance) = self.store.instance(id).await.map_err(read)? else {
            return Err((
                error_code::UNKNOWN_INSTANCE,
                format!("no instance {id}; see agend instance list"),
            ));
        };
        if let Some(task) = self.store.held_task(id).await.map_err(read)? {
            return Err((
                error_code::INVALID_REQUEST,
                format!("{id} holds {task}; run agend task cancel {task} first"),
            ));
        }
        self.watches.remove(id);
        self.codex.disconnect(id);
        self.runtime.detach(id);
        let (home, holder) = (self.home.clone(), id.to_owned());
        let stopped = tokio::task::spawn_blocking(move || {
            crate::runtime::shutdown_holder_within(&home, &holder, REMOVE_WITHIN)
        })
        .await
        .unwrap_or_else(|e| Err(format!("panicked: {e}")));
        if let Err(e) = stopped {
            log::line(&format!(
                "{id}: its holder did not stop ({e}); the next boot sweeps it"
            ));
        }
        self.sweep(&instance, "removed").await;
        self.store.remove_instance(id).await.map_err(|e| {
            (
                error_code::INVALID_REQUEST,
                format!("cannot remove {id}: {e}"),
            )
        })?;
        self.fleet.remove_instance(id);
        log::line(&format!(
            "{id}: removed (workspace kept at {})",
            instance.working_directory
        ));
        Ok(())
    }

    /// Handles events until a stop signal or a restart.
    pub async fn run(&mut self, events: &mut UnboundedReceiver<Event>) -> Stopped {
        while let Some(event) = events.recv().await {
            match event {
                Event::Holder(HolderEvent::AgentExited {
                    id,
                    generation,
                    exited,
                }) => {
                    let what = format!("agent {id} exited ({})", describe_exit(&exited));
                    self.died(&id, generation, what, false).await;
                }
                Event::Holder(HolderEvent::HolderGone { id, generation }) => {
                    let what = format!("holder {id} died");
                    self.died(&id, generation, what, true).await;
                }
                Event::StartFailed {
                    id,
                    generation,
                    error,
                } => {
                    let what = format!("{id}: start failed: {error}");
                    self.died(&id, generation, what, false).await;
                }
                Event::Codex(CodexEvent::Gone { id, generation }) => {
                    let what = format!("{id}: its app-server is gone");
                    self.died(&id, generation, what, false).await;
                }
                Event::Restart { id, generation } => self.restart(&id, generation).await,
                Event::Housekeeping => {
                    crate::housekeeping::run(&self.store, &self.home, log::now_unix_ms()).await;
                    if let Some(p) = &self.pipeline {
                        p.daily();
                    }
                }
                Event::Retry { item } => self.retry(*item).await,
                Event::Add { request, reply } => self.add(request, reply).await,
                Event::Remove { id, reply } => {
                    let removed = self.remove(&id).await;
                    let _ = reply.send(removed);
                }
                Event::Exec(binary) => return Stopped::Exec(binary),
                Event::Stop(signal) => return Stopped::Signal(signal),
            }
            if let Some(p) = &self.pipeline {
                p.wake();
            }
        }
        Stopped::Signal("channel closed")
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
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
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
        let mut claude = instance(Backend::Claude, Some("s-abc"));
        claude.delivery = "inbox".into();
        let fresh = launch(Path::new("/h"), &claude, false).unwrap();
        assert_eq!(fresh.args[3..], ["--session-id", "s-abc"]);
        let resumed = launch(Path::new("/h"), &claude, true).unwrap();
        assert_eq!(resumed.args[3..], ["--resume", "s-abc"]);
        assert_eq!(resumed.args[..3], claude.args[..]);
        assert_eq!(resumed.executable, "/bin/bash");
    }

    /// P5: `retry` for claude always; for opencode only while its session
    /// never started (no session id to resume, never fresh again); for
    /// codex (gate 7) unless migration 0004 marked it `legacy_no_thread`.
    #[test]
    fn a_failed_instance_is_a_needs_you_item_with_retry_unless_it_cannot_resume() {
        let table = [
            (Backend::Claude, true, false, true),
            (Backend::Claude, false, false, true),
            (Backend::Codex, false, false, true),
            (Backend::Opencode, false, false, true),
            (Backend::Codex, true, false, true),
            (Backend::Codex, true, true, false),
            (Backend::Opencode, true, false, false),
        ];
        for (backend, started, legacy, retry) in table {
            let mut inst = instance(backend, None);
            inst.id = "g8-2".into();
            inst.session_started = started;
            inst.legacy_no_thread = legacy;
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
        let h = Path::new("/h");
        let inst = instance(Backend::Opencode, None);
        assert_eq!(launch(h, &inst, false).unwrap().args, inst.args);
        let error = launch(h, &inst, true).unwrap_err();
        assert!(error.starts_with("no session id to resume"), "{error}");
        let error = launch(h, &instance(Backend::Claude, None), true).unwrap_err();
        assert!(error.starts_with("no session id to resume"), "{error}");
    }

    /// Gate 7 P2, P3: codex always runs the wrapper, with or without a
    /// thread and whether it resumes or not (the thread goes through $GO).
    #[test]
    fn codex_always_launches_the_wrapper() {
        let h = Path::new("/h");
        for session in [None, Some("thread-1")] {
            let mut inst = instance(Backend::Codex, session);
            inst.working_directory = "/".into();
            for resume in [false, true] {
                let l = launch(h, &inst, resume).unwrap();
                assert_eq!(l.executable, "/bin/sh");
                assert_eq!(l.args, codex_launch::wrapper_args(h, &inst).unwrap());
            }
        }
    }
}
