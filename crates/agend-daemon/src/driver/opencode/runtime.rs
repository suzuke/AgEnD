//! Supervisor-owned workers. Cancellation is generation scoped; daemon exit
//! stops workers without stopping holder-owned servers or terminals.
use super::{api::Session, http::Http, launch::Layout, worker::Worker};
use crate::{
    runtime::files,
    store::{SqliteStore, instances},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub enum Notice {
    State(Option<bool>),
    Failed(String),
}
pub type Sink = Arc<dyn Fn(Notice) + Send + Sync>;
pub struct Runtime {
    store: Arc<SqliteStore>,
    workers: Mutex<BTreeMap<String, Vec<RunningWorker>>>,
}
struct RunningWorker {
    cancelled: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}
impl Runtime {
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self {
            store,
            workers: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn disconnect(&self, id: &str) {
        let mut workers = self.workers.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(generations) = workers.get_mut(id) {
            for worker in generations.iter() {
                worker.cancelled.store(true, Ordering::SeqCst);
            }
            generations.retain(|worker| !worker.thread.is_finished());
            if generations.is_empty() {
                workers.remove(id);
            }
        }
    }
    /// Actual thread completion, including older cancelled generations. The
    /// supervisor must serialize a subsequent start with its switch workflow.
    /// This proves local worker cessation, not backend turn completion.
    pub fn workers_stopped(&self, id: &str) -> bool {
        let mut workers = self.workers.lock().unwrap_or_else(|p| p.into_inner());
        let Some(generations) = workers.get_mut(id) else {
            return true;
        };
        generations.retain(|worker| !worker.thread.is_finished());
        let stopped = generations.is_empty();
        if stopped {
            workers.remove(id);
        }
        stopped
    }
    pub fn start(&self, id: &str, sink: Sink) {
        let mut workers = self.workers.lock().unwrap_or_else(|p| p.into_inner());
        let generations = workers.entry(id.into()).or_default();
        for worker in generations.iter() {
            worker.cancelled.store(true, Ordering::SeqCst);
        }
        generations.retain(|worker| !worker.thread.is_finished());
        let flag = Arc::new(AtomicBool::new(false));
        let cancelled = flag.clone();
        let (store, id) = (self.store.clone(), id.to_owned());
        let thread = std::thread::spawn(move || {
            if let Err(error) = run(store, id, flag.clone(), &sink)
                && !flag.load(Ordering::SeqCst)
            {
                sink(Notice::Failed(error));
            }
        });
        generations.push(RunningWorker { cancelled, thread });
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        for generations in self
            .workers
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .values()
        {
            for worker in generations {
                worker.cancelled.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn run(
    store: Arc<SqliteStore>,
    id: String,
    cancelled: Arc<AtomicBool>,
    sink: &Sink,
) -> Result<(), String> {
    let layout = Layout::new(store.home(), &id)?;
    let holder = files::running(store.home(), &id)
        .map_err(|e| e.to_string())?
        .ok_or("OpenCode holder is gone")?;
    let live = || -> Result<(), String> {
        if cancelled.load(Ordering::SeqCst) {
            return Err("OpenCode worker cancelled".into());
        }
        if files::running(store.home(), &id).map_err(|e| e.to_string())? != Some(holder) {
            return Err("OpenCode holder changed".into());
        }
        Ok(())
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let (port, version) = loop {
        live()?;
        match layout.endpoint(holder) {
            Ok(endpoint) => break endpoint,
            Err(e) if Instant::now() >= deadline => return Err(format!("OpenCode endpoint: {e}")),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    if version != "1.18.34" {
        return Err(format!("OpenCode {version} is not verified; use 1.18.34"));
    }
    let lookup = id.clone();
    let instance = store
        .call_blocking(move |conn| instances::get(conn, &lookup))
        .map_err(|e| e.to_string())?
        .ok_or("OpenCode instance removed")?;
    // Explicitly reject unsupported launch options instead of silently dropping
    // an operator's requested model or permission configuration.
    let model = super::launch::model(&instance.args)?;
    let password = layout.password().map_err(|e| e.to_string())?;
    let http =
        Http::new(port, &password, &instance.working_directory).map_err(|e| e.to_string())?;
    let health = http.get("/global/health").map_err(|e| e.to_string())?;
    if health["healthy"] != true || health["version"] != version {
        return Err("OpenCode health/version mismatch".into());
    }
    live()?;
    let session = match instance.session_id {
        Some(session) => Session::resume(http, &session)?,
        None => {
            let session = Session::create(http)?;
            live()?;
            let (id, sid, flag) = (id.clone(), session.id().to_owned(), cancelled.clone());
            store
                .call_blocking(move |conn| {
                    if flag.load(Ordering::SeqCst) {
                        return Err(crate::store::StoreError::Invalid(
                            "OpenCode startup cancelled".into(),
                        ));
                    }
                    let changed = conn.execute(
                        "UPDATE instances SET session_id=?2 WHERE id=?1 AND session_id IS NULL",
                        rusqlite::params![id, sid],
                    )?;
                    if changed != 1 {
                        return Err(crate::store::StoreError::Invalid(
                            "OpenCode session changed during startup".into(),
                        ));
                    }
                    Ok(())
                })
                .map_err(|e| e.to_string())?;
            session
        }
    };
    live()?;
    layout
        .handoff(session.id(), port)
        .map_err(|e| e.to_string())?;
    let worker = Worker {
        store: store.clone(),
        instance: id.clone(),
        session,
        cancelled: cancelled.clone(),
        model,
        history_before: std::cell::RefCell::new(None),
        reconcile_after: std::cell::Cell::new(0),
    };
    let mut failure = None;
    let mut reported = None;
    let mut idle_since = None;
    loop {
        live()?;
        match worker.tick() {
            Ok(busy) => {
                failure = None;
                if busy {
                    idle_since = None;
                } else {
                    idle_since.get_or_insert_with(Instant::now);
                }
                let publish = busy
                    || idle_since.is_some_and(|since| since.elapsed() >= Duration::from_secs(5));
                if publish && reported != Some(busy) {
                    let (instance, session) = (id.clone(), worker.session.id().to_owned());
                    store
                        .call_blocking(move |c| {
                            crate::store::opencode::state(
                                c,
                                &instance,
                                &session,
                                busy,
                                crate::log::now_unix_ms(),
                            )
                        })
                        .map_err(|e| e.to_string())?;
                    sink(Notice::State(Some(busy)));
                    reported = Some(busy);
                }
            }
            Err(error) => {
                idle_since = None;
                if failure.is_none() {
                    sink(Notice::State(None));
                    reported = None;
                }
                let started = failure.get_or_insert_with(Instant::now);
                if started.elapsed() >= Duration::from_secs(20) {
                    return Err(error);
                }
            }
        }
        for _ in 0..5 {
            if cancelled.load(Ordering::SeqCst) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
