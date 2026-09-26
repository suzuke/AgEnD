//! Supervisor: keeps the DB's instances running (gate 6 P2, P3, P6). Later
//! gates add detecting stuck agents, usage limits and unknown prompts,
//! reassigning work, and escalating to a human only for exceptions.
//!
//! Gate 6:
//! - Boot: runs [`crate::boot::plan_boot`]'s actions (reconnect, start,
//!   `Shutdown` orphans).
//! - A first start writes `running` to the DB before `Spawn` (P3), so a
//!   daemon that dies in between never leaves an agent the DB does not know.
//! - Death (the agent exits, the holder dies, a start fails): wait 5 s, then
//!   start again with the backend's resume arguments (`--resume <session>`).
//!   Three restarts within 10 minutes and it still dies → `failed`; no
//!   session id to resume → `failed` at once. Never a fresh start after the
//!   first (P6). Restart counts live in memory only.
//!
//! Must NOT: kill or respawn holders on daemon shutdown; start an agent
//! fresh once it has run.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use agend_core::model::Backend;
use agend_core::protocol::holder::ExitedData;
use agend_core::traits::HolderLaunch;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::boot::{BootAction, plan_boot};
use crate::log;
use crate::runtime::{HolderEvent, HolderRuntime, SpawnOutcome, Started, files};
use crate::store::{Instance, InstanceStatus, SqliteStore};

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
    Stop(&'static str),
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
    pub fn new(store: SqliteStore, runtime: HolderRuntime, events: UnboundedSender<Event>) -> Self {
        Self {
            home: runtime.home().to_path_buf(),
            store,
            runtime,
            events,
            watches: BTreeMap::new(),
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
    /// that already ran its agent answers `already_spawned`. The `Spawn`
    /// resumes when the backend can; otherwise (codex, opencode: they never
    /// restart) its holder can only lack an agent if the daemon died between
    /// starting the holder and `Spawn`, when the agent had never run.
    async fn reconnect(&mut self, instance: &Instance, pid: u32) {
        let id = instance.id.clone();
        if instance.status == InstanceStatus::New && !self.mark_running(&id).await {
            return;
        }
        let launch = launch(instance, true)
            .or_else(|_| launch(instance, false))
            .expect("a fresh launch always exists");
        match self.runtime.attach(&launch, pid).await {
            Ok(started) => {
                let note = match started.attached.spawn {
                    Some(SpawnOutcome::Spawned) => "; it had no agent, started one",
                    _ => "",
                };
                log::line(&format!(
                    "{id}: reconnected to holder pid={pid}{note}; screen: {}",
                    last_line(&started.attached.screen)
                ));
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
        if !resume && !self.mark_running(&id).await {
            return;
        }
        let session = session_args(instance, resume)
            .map(|a| a.join(" "))
            .unwrap_or_default();
        let what = match restart {
            Some(n) => format!("restart {n}/{MAX_RESTARTS} {session}"),
            None => format!("start {session}"),
        };
        log::line(&format!("{id}: {}", what.trim_end()));
        match self.runtime.start(&launch).await {
            Ok(Started {
                handle, generation, ..
            }) => {
                log::line(&format!(
                    "{id}: holder pid={} started",
                    handle.process_id.unwrap_or(0)
                ));
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

    /// Writes `running` before the first `Spawn` (P3); false if that failed.
    async fn mark_running(&mut self, id: &str) -> bool {
        match self
            .store
            .set_instance_status(id, InstanceStatus::Running)
            .await
        {
            Ok(()) => true,
            Err(e) => {
                log::line(&format!("{id}: cannot record running: {e}; not started"));
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

    async fn fail(&mut self, id: &str, reason: &str) {
        if let Some(watch) = self.watches.get_mut(id) {
            watch.state = State::Failed;
        }
        if let Err(e) = self
            .store
            .set_instance_status(id, InstanceStatus::Failed)
            .await
        {
            log::line(&format!("{id}: cannot record failed: {e}"));
        }
        log::line(&format!("{id} failed: {reason}"));
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
        if let Err(reason) = session_args(&instance, true) {
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
            Ok(Some(instance)) => self.start(&instance, true, Some(n)).await,
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
