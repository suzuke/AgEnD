//! [`CodexDriver`]: `agend_core::traits::Driver` for codex (gate 7).
//!
//! - [`CodexDriver::connect`] (the supervisor, after a holder is up): waits
//!   for the app-server (every 100 ms, [`launch::READY_WITHIN`]), creates
//!   the thread when the instance has none (`thread/start`, its id stored
//!   before anything else) or resumes it, writes `$GO` for the wrapper, and
//!   starts the instance's link. A thread that `thread/resume` cannot find
//!   is replaced only when no message was ever sent to it (owner-approved
//!   exception to gate 6 P6: an empty thread has no context to lose).
//! - `deliver`: [`crate::store::messages::claim`] (same id + same content →
//!   the current state, no codex call; same id + other content →
//!   [`DriverError::InvalidRequest`]); then the link sends it. Without a
//!   link (the app-server restarting, a backend without a driver) it stays
//!   `queued`.
//! - `events`: the thread history ([`super::history`]); reading it also
//!   confirms what it finds.
//!
//! Must NOT: send a message itself (only the instance's link does), or
//! start a thread for an instance marked `legacy_no_thread`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agend_core::model::{Backend, DeliveryState};
use agend_core::policy::busy::BusyLevel;
use agend_core::traits::{AgentMessage, DeliveryReceipt, Driver, DriverEvent};

use super::history::{after, expand, match_identified_items, match_items, user_items};
use super::launch;
use super::link::{self, CodexSink, Link, Worker};
use super::rpc::RpcError;
use crate::log;
use crate::store::{Claim, Instance, InstanceStatus, Message, NewMessage, SqliteStore, StoreError};

/// How long `deliver` waits for the link to send.
const SEND_WITHIN: Duration = Duration::from_secs(60);
const EVENTS_WITHIN: Duration = Duration::from_secs(60);
const READY_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub enum DriverError {
    /// No such instance (DRV-2, DRV-8).
    UnknownInstance(String),
    /// The id was used for other content (P5).
    InvalidRequest(String),
    /// No link to this instance's app-server now.
    NotConnected(String),
    Store(StoreError),
    Backend(String),
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownInstance(id) => write!(f, "no instance {id}"),
            Self::InvalidRequest(what) => write!(f, "invalid_request: {what}"),
            Self::NotConnected(id) => write!(f, "{id}: the app-server is not connected"),
            Self::Store(e) => write!(f, "store: {e}"),
            Self::Backend(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for DriverError {}

impl From<StoreError> for DriverError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

#[derive(Clone)]
pub struct CodexDriver {
    inner: Arc<Inner>,
}

struct Inner {
    home: PathBuf,
    store: Arc<SqliteStore>,
    sink: CodexSink,
    input_policy: agend_core::policy::codex_input::CodexInputPolicy,
    links: Mutex<BTreeMap<String, Link>>,
    /// The holder generation each instance's latest `connect` is for; an
    /// older connect still running gives up instead of storing a thread,
    /// writing `$GO` or keeping a link.
    current: Mutex<BTreeMap<String, u64>>,
    activity: Mutex<BTreeMap<String, Vec<std::sync::Weak<()>>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let links = std::mem::take(&mut *self.lock_links());
        for (_, link) in links {
            link.close();
        }
    }
}

/// Runs blocking driver work on tokio's blocking pool, or on the caller's
/// thread outside a tokio runtime (tests poll with testkit's `block_on`).
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, DriverError> + Send + 'static,
) -> Result<T, DriverError> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle
            .spawn_blocking(f)
            .await
            .map_err(|e| DriverError::Backend(format!("driver call panicked: {e}")))?,
        Err(_) => f(),
    }
}

fn receipt(row: &Message) -> DeliveryReceipt {
    DeliveryReceipt {
        backend_message_id: row.turn_id.clone(),
        state: row.state,
    }
}

impl CodexDriver {
    /// A driver for the instances under `home`; `sink` hears when an
    /// instance's app-server is gone.
    pub fn new(home: &Path, store: Arc<SqliteStore>, sink: CodexSink) -> Self {
        Self::with_input_policy(home, store, sink, Default::default())
    }
    pub fn with_input_policy(
        home: &Path,
        store: Arc<SqliteStore>,
        sink: CodexSink,
        input_policy: agend_core::policy::codex_input::CodexInputPolicy,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                home: home.to_path_buf(),
                store,
                sink,
                input_policy,
                links: Mutex::new(BTreeMap::new()),
                current: Mutex::new(BTreeMap::new()),
                activity: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    /// Readiness, thread and `$GO` for instance `id` whose holder runs
    /// (`generation` tags its [`super::CodexEvent`]s). Returns the lines to
    /// log; `None` when a newer connect (or a disconnect) superseded it.
    pub async fn connect(&self, id: &str, generation: u64) -> Result<Option<Vec<String>>, String> {
        let inner = Arc::clone(&self.inner);
        let id = id.to_owned();
        let activity = Arc::new(());
        {
            let mut active = inner.activity.lock().unwrap_or_else(|p| p.into_inner());
            let generations = active.entry(id.clone()).or_default();
            generations.retain(|token| token.strong_count() != 0);
            generations.push(Arc::downgrade(&activity));
        }
        blocking(move || {
            inner
                .connect(&id, generation, activity)
                .map_err(DriverError::Backend)
        })
        .await
        .map_err(|e| e.to_string())
    }

    /// Closes the link of `id` (nothing is sent to codex); a connect still
    /// running for it gives up.
    pub fn disconnect(&self, id: &str) {
        self.inner.disconnect(id);
    }

    /// Includes pending handshakes and retired link workers whose bounded
    /// disconnect wait expired. Callers must serialize new connects with a
    /// version switch; backend turn completion is a separate check.
    pub fn workers_stopped(&self, id: &str) -> bool {
        let mut active = self
            .inner
            .activity
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Some(generations) = active.get_mut(id) else {
            return true;
        };
        generations.retain(|token| token.strong_count() != 0);
        let stopped = generations.is_empty();
        if stopped {
            active.remove(id);
        }
        stopped
    }

    /// Whether the link of `id` is connected now.
    pub fn is_connected(&self, id: &str) -> bool {
        self.inner
            .lock_links()
            .get(id)
            .is_some_and(|l| l.shared.connected.load(Ordering::SeqCst))
    }

    /// Only a connected, version-approved peer with committed strict attribution.
    pub fn can_input(&self, id: &str) -> bool {
        self.inner.lock_links().get(id).is_some_and(|link| {
            link.shared.connected.load(Ordering::SeqCst)
                && link.shared.input_allowed.load(Ordering::SeqCst)
                && crate::runtime::files::running(&self.inner.home, id)
                    .ok()
                    .flatten()
                    == Some(link.shared.holder_pid.load(Ordering::SeqCst))
        })
    }

    /// A disconnected cached busy flag is not readiness evidence.
    pub fn connected_busy(&self, id: &str) -> Option<bool> {
        let links = self.inner.lock_links();
        let link = links.get(id)?;
        link.shared
            .connected
            .load(Ordering::SeqCst)
            .then(|| link.shared.busy.load(Ordering::SeqCst))
    }

    /// Fresh read-only thread snapshot, never a cached busy flag. The caller
    /// must pause all input, drain writers, and separately verify the managed
    /// holder identity before using this observation to stop a backend.
    pub async fn thread_idle(&self, id: &str) -> Result<bool, DriverError> {
        let inner = Arc::clone(&self.inner);
        let id = id.to_owned();
        blocking(move || {
            let generation = inner.lock_current().get(&id).copied();
            let instance = inner
                .instance(&id)?
                .ok_or_else(|| DriverError::UnknownInstance(id.clone()))?;
            if instance.session_id.is_none()
                || instance.backend != Backend::Codex
                || instance.status != crate::store::InstanceStatus::Running
            {
                return Err(DriverError::NotConnected(id));
            }
            let (shared, wait) = inner
                .lock_links()
                .get(&id)
                .filter(|link| {
                    link.shared.connected.load(Ordering::SeqCst)
                        && instance.session_id.as_deref() == Some(link.session_id.as_str())
                })
                .map(|link| (Arc::clone(&link.shared), link.idle_request()))
                .ok_or_else(|| DriverError::NotConnected(id.clone()))?;
            let idle = wait
                .wait(Duration::from_secs(5))
                .map_err(DriverError::Backend)?;
            let same_link = inner.lock_links().get(&id).is_some_and(|link| {
                Arc::ptr_eq(&shared, &link.shared) && link.shared.connected.load(Ordering::SeqCst)
            });
            if !same_link
                || generation.is_none()
                || inner.lock_current().get(&id).copied() != generation
                || inner.instance(&id)?.as_ref() != Some(&instance)
            {
                return Err(DriverError::NotConnected(id));
            }
            Ok(!shared.busy.load(Ordering::SeqCst) && idle)
        })
        .await
    }

    /// Query the connected thread without sending or confirming any message.
    pub async fn message_outcome(
        &self,
        row: Message,
    ) -> Result<agend_core::protocol::client::MessageOutcomeState, DriverError> {
        let inner = Arc::clone(&self.inner);
        blocking(move || {
            let id = &row.to_instance;
            let generation = inner.lock_current().get(id).copied();
            let instance = inner
                .instance(id)?
                .ok_or_else(|| DriverError::UnknownInstance(id.clone()))?;
            let wait = inner
                .lock_links()
                .get(id)
                .filter(|link| link.shared.connected.load(Ordering::SeqCst))
                .map(Link::turns_request)
                .ok_or_else(|| DriverError::NotConnected(id.clone()))?;
            let turns = wait
                .wait(Duration::from_secs(5))
                .map_err(DriverError::Backend)?;
            if generation.is_none()
                || inner.lock_current().get(id).copied() != generation
                || inner.instance(id)?.and_then(|i| i.session_id) != instance.session_id
            {
                return Err(DriverError::NotConnected(id.clone()));
            }
            Ok(super::history::message_outcome(&turns, &row))
        })
        .await
    }

    /// Busy as the link of `id` sees it (not debounced); `None` without a link.
    pub fn busy(&self, id: &str) -> Option<bool> {
        self.inner
            .lock_links()
            .get(id)
            .map(|l| l.shared.busy.load(Ordering::SeqCst))
    }
}

impl Inner {
    fn lock_links(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Link>> {
        self.links.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_current(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, u64>> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn is_current(&self, id: &str, generation: u64) -> bool {
        self.lock_current().get(id) == Some(&generation)
    }

    fn close_link(&self, id: &str) {
        let link = self.lock_links().remove(id);
        if let Some(link) = link {
            link.close();
        }
    }

    fn disconnect(&self, id: &str) {
        self.lock_current().remove(id);
        self.close_link(id);
    }

    fn instance(&self, id: &str) -> Result<Option<Instance>, StoreError> {
        let id = id.to_owned();
        self.store
            .call_blocking(move |conn| crate::store::instances::get(conn, &id))
    }

    fn connect(
        &self,
        id: &str,
        generation: u64,
        activity: Arc<()>,
    ) -> Result<Option<Vec<String>>, String> {
        {
            let mut current = self.lock_current();
            if current.get(id).is_some_and(|&newer| newer > generation) {
                return Ok(None);
            }
            current.insert(id.to_owned(), generation);
        }
        self.close_link(id);
        let instance = self
            .instance(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no instance {id}"))?;
        if instance.legacy_no_thread {
            return Err("a codex instance from before gate 7; a human decides".into());
        }
        let previous_thread = instance.session_id.clone();
        let identified = self
            .store
            .call_blocking(move |conn| match previous_thread {
                Some(thread) => crate::store::codex_input::requires_identified(conn, &thread),
                None => Ok(false),
            })
            .map_err(|e| format!("cannot read thread receipt attribution: {e}"))?;
        let listen = launch::socket_path(&self.home, id);
        let started = Instant::now();
        let mut worker = loop {
            if !self.is_current(id, generation) {
                return Ok(None);
            }
            match Worker::open(
                id,
                generation,
                listen.clone(),
                Arc::clone(&self.store),
                identified,
            ) {
                Ok(worker) => break worker,
                Err(e) if started.elapsed() >= launch::READY_WITHIN => {
                    return Err(format!(
                        "app-server not ready within {} s: {e}",
                        launch::READY_WITHIN.as_secs()
                    ));
                }
                Err(_) => std::thread::sleep(READY_EVERY),
            }
        };
        let mut lines = vec![format!(
            "app-server ready ({} ms)",
            started.elapsed().as_millis()
        )];
        let cwd = launch::real_workdir(&instance)?;
        if !self.is_current(id, generation) {
            return Ok(None);
        }
        let thread = match &instance.session_id {
            None => {
                let thread = worker.start_thread(&cwd).map_err(|e| e.to_string())?;
                self.store_thread(id, &thread)?;
                lines.push(format!("thread {thread} created"));
                thread
            }
            Some(old) => match worker.resume(old) {
                Ok(()) => {
                    let busy = if worker.is_busy() { "busy" } else { "idle" };
                    lines.push(format!("thread {old} resumed ({busy})"));
                    old.clone()
                }
                // K10/K15: codex 0.158.0 answers a thread that never had a
                // turn with -32600 `no rollout found for thread id <id>`
                // (resume_empty.jsonl); the fake says `not found`.
                Err(e @ RpcError::Rpc { .. })
                    if ["not found", "no rollout found"]
                        .iter()
                        .any(|m| e.to_string().contains(m)) =>
                {
                    let rows = link::messages_to(&self.store, id).map_err(|e| e.to_string())?;
                    let used = rows
                        .iter()
                        .any(|r| matches!(r.state, DeliveryState::Sent | DeliveryState::Confirmed));
                    if used {
                        return Err(format!(
                            "thread {old} not found and messages were sent to it: {e}"
                        ));
                    }
                    let thread = worker.start_thread(&cwd).map_err(|e| e.to_string())?;
                    self.store_thread(id, &thread)?;
                    lines.push(format!(
                        "thread {old} not found and never used; new thread {thread}"
                    ));
                    thread
                }
                Err(e) => return Err(format!("thread/resume {old}: {e}")),
            },
        };
        let real = super::socket_connect_path(&listen).map_err(|e| e.to_string())?;
        if !self.is_current(id, generation) {
            return Ok(None);
        }
        let holder_pid = crate::runtime::files::running(&self.home, id)
            .map_err(|e| format!("cannot read holder identity: {e}"))?
            .unwrap_or(0);
        let admitted = holder_pid != 0
            && (self.input_policy.allows_instance(id)
                || launch::launched_record(&self.home, id).is_some_and(|(pid, version)| {
                    pid == holder_pid && self.input_policy.allows_version(&version)
                }));
        let receipt_thread = thread.clone();
        let identified = self
            .store
            .call_blocking(move |conn| {
                if admitted {
                    crate::store::codex_input::enable_identified(conn, &receipt_thread)?;
                }
                crate::store::codex_input::requires_identified(conn, &receipt_thread)
            })
            .map_err(|e| format!("cannot establish thread receipt attribution: {e}"))?;
        if !self.is_current(id, generation) {
            return Ok(None);
        }
        worker.set_input_policy(identified, admitted, holder_pid);
        lines.push(format!(
            "operator input {}; receipts {}",
            if admitted { "enabled" } else { "denied" },
            if identified {
                "own clientId only"
            } else {
                "legacy"
            }
        ));
        match launch::write_go(&self.home, id, &thread, &real) {
            Ok(true) => lines.push(format!("go (resume {thread})")),
            Ok(false) => {}
            Err(e) => return Err(format!("cannot write the handoff file: {e}")),
        }
        worker
            .catch_up()
            .map_err(|e| format!("catching up with the thread: {e}"))?;
        let link = worker
            .spawn(Arc::clone(&self.sink), activity)
            .map_err(|e| format!("cannot start the link thread: {e}"))?;
        let mut links = self.lock_links();
        if !self.is_current(id, generation) {
            drop(links);
            link.close();
            return Ok(None);
        }
        links.insert(id.to_owned(), link);
        Ok(Some(lines))
    }

    fn store_thread(&self, id: &str, thread: &str) -> Result<(), String> {
        let (id, thread) = (id.to_owned(), thread.to_owned());
        self.store
            .call_blocking(move |conn| crate::store::instances::set_session_id(conn, &id, &thread))
            .map_err(|e| format!("cannot store the thread id: {e}"))
    }

    fn deliver(
        &self,
        id: &str,
        message: &AgentMessage,
        level: BusyLevel,
    ) -> Result<DeliveryReceipt, DriverError> {
        let instance = self
            .instance(id)?
            .ok_or_else(|| DriverError::UnknownInstance(id.to_owned()))?;
        let new = NewMessage {
            id: message.id.clone(),
            from_instance: message.from.clone(),
            to_instance: id.to_owned(),
            task_id: message.task_id.clone(),
            body: message.body.clone(),
            level,
        };
        let now = log::now_unix_ms();
        let claimed = self
            .store
            .call_blocking(move |conn| crate::store::messages::claim(conn, &new, now))?;
        let row = match claimed {
            Claim::Different(_) => {
                return Err(DriverError::InvalidRequest(format!(
                    "message id {} was already used for other content",
                    message.id
                )));
            }
            Claim::Existing(row) => {
                log::line(&format!(
                    "{id}: {} already {}; not sent again",
                    row.id,
                    crate::store::messages::state_text(row.state)
                ));
                return Ok(receipt(&row));
            }
            Claim::Inserted(row) => row,
        };
        if instance.backend != Backend::Codex {
            return Ok(receipt(&row));
        }
        if instance.status == InstanceStatus::Failed {
            let failed = self.store.call_blocking({
                let row = row.id.clone();
                move |conn| {
                    crate::store::messages::advance(conn, &row, DeliveryState::Failed, None, now)
                }
            })?;
            return Ok(receipt(failed.as_ref().unwrap_or(&row)));
        }
        let flushed = {
            let links = self.lock_links();
            match links.get(id) {
                Some(link) if link.shared.connected.load(Ordering::SeqCst) => {
                    // The link thread sends; wait for it without the lock.
                    Some(link.flush_request())
                }
                _ => None,
            }
        };
        if let Some(wait) = flushed {
            wait.wait(SEND_WITHIN);
        }
        let now_row = self
            .store
            .call_blocking({
                let row = row.id.clone();
                move |conn| crate::store::messages::get(conn, &row)
            })?
            .unwrap_or(row);
        Ok(receipt(&now_row))
    }

    fn events(&self, id: &str, cursor: Option<&str>) -> Result<Vec<DriverEvent>, DriverError> {
        let instance = self
            .instance(id)?
            .ok_or_else(|| DriverError::UnknownInstance(id.to_owned()))?;
        let wait = self
            .lock_links()
            .get(id)
            .map(Link::turns_request)
            .ok_or_else(|| DriverError::NotConnected(id.to_owned()))?;
        let turns = wait.wait(EVENTS_WITHIN).map_err(DriverError::Backend)?;
        let rows = link::messages_to(&self.store, id)?;
        let items = user_items(&turns);
        let thread = instance
            .session_id
            .ok_or_else(|| DriverError::Backend("no thread id".into()))?;
        let identified = self.store.call_blocking(move |conn| {
            crate::store::codex_input::requires_identified(conn, &thread)
        })?;
        let match_receipts = if identified {
            match_identified_items
        } else {
            match_items
        };
        let found = match_receipts(
            &items.iter().map(|(_, u)| u.clone()).collect::<Vec<_>>(),
            &rows,
        );
        let mut matched = Vec::new();
        for ((pos, item), row) in items.into_iter().zip(found) {
            let Some(row) = row else { continue };
            if rows.iter().any(|r| {
                r.id == row && matches!(r.state, DeliveryState::Queued | DeliveryState::Sent)
            }) && link::confirm(&self.store, &row, &item.turn_id)?.is_some()
            {
                log::line(&format!(
                    "{id}: {row} confirmed (turn {}) from the thread history",
                    item.turn_id
                ));
            }
            matched.push((pos, row));
        }
        let events = expand(&turns, &matched);
        Ok(after(&turns, events, cursor))
    }
}

impl Driver for CodexDriver {
    type Error = DriverError;

    async fn deliver(
        &self,
        instance_id: &str,
        message: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, DriverError> {
        let inner = Arc::clone(&self.inner);
        let (id, message) = (instance_id.to_owned(), message.clone());
        blocking(move || inner.deliver(&id, &message, mode)).await
    }

    async fn events(
        &self,
        instance_id: &str,
        after_cursor: Option<&str>,
    ) -> Result<Vec<DriverEvent>, DriverError> {
        let inner = Arc::clone(&self.inner);
        let id = instance_id.to_owned();
        let cursor = after_cursor.map(str::to_owned);
        blocking(move || inner.events(&id, cursor.as_deref())).await
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use agend_testkit::{block_on, tempdir::TempDir};
    #[test]
    fn unreadable_attribution_denies_connect_before_resume_or_input() {
        let dir = TempDir::new("codex-input-read-failure").unwrap();
        let store = Arc::new(SqliteStore::open(dir.path(), 0).unwrap());
        let instance = Instance {
            id: "codex".into(),
            backend: Backend::Codex,
            program: "/unused-codex".into(),
            args: vec![],
            working_directory: dir.path().display().to_string(),
            session_id: Some("previously-enabled-thread".into()),
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        };
        block_on(store.add_instance(&instance)).unwrap();
        store
            .call_blocking(|conn| {
                conn.execute_batch("DROP TABLE codex_input_threads")?;
                Ok(())
            })
            .unwrap();
        let driver = CodexDriver::with_input_policy(
            dir.path(),
            store,
            Arc::new(|_| {}),
            agend_core::policy::codex_input::CodexInputPolicy::approved(),
        );
        let error = block_on(driver.connect("codex", 1)).unwrap_err();
        assert!(
            error.contains("cannot read thread receipt attribution"),
            "{error}"
        );
        assert!(!driver.can_input("codex"));
        assert!(!launch::go_path(dir.path(), "codex").exists());
    }
}
