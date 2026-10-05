//! Client-scoped terminal entry service. One bounded actor per instance orders
//! control and PTY writes; views share the holder's authoritative frame sample.
//! Socket EOF marks scopes dead synchronously, even during a pending write.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use agend_core::protocol::client::*;
use agend_core::protocol::terminal::*;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::fleet::Fleet;
use crate::handlers::{CODEX_INPUT, error};
use crate::runtime::{HolderRuntime, terminal::TerminalConnection};

mod actor;
mod control;
use actor::Actor;

const QUEUE: usize = 64;
const SAMPLE_EVERY: Duration = Duration::from_millis(50);

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Clone)]
pub struct TerminalHub(Arc<Inner>);
struct Inner {
    runtime: HolderRuntime,
    fleet: Arc<Fleet>,
    codex_input: agend_core::policy::codex_input::CodexInputPolicy,
    codex: Option<crate::driver::codex::CodexDriver>,
    next: AtomicU64,
    nonce: String,
    actors: Mutex<BTreeMap<String, Handle>>,
}
struct Handle {
    identity: u64,
    jobs: mpsc::Sender<Job>,
    task: JoinHandle<()>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        for handle in lock(&self.actors).values() {
            handle.task.abort();
        }
    }
}

/// Dropping this view synchronously invalidates queued operations for it.
/// The actor releases the actual holder owner after any in-flight write ends.
pub struct ViewStream {
    pub instance_id: String,
    pub view_id: String,
    alive: Arc<AtomicBool>,
    frames: watch::Receiver<Option<Arc<ClientResponse>>>,
}
impl ViewStream {
    pub async fn next(&mut self) -> Result<Option<Arc<ClientResponse>>, watch::error::RecvError> {
        self.frames.changed().await?;
        Ok(self.frames.borrow_and_update().clone())
    }
}
impl Drop for ViewStream {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone)]
pub struct ReplyScope {
    pub client: u64,
    pub alive: Arc<AtomicBool>,
    pub replies: mpsc::Sender<ClientResponse>,
}
impl ReplyScope {
    fn send(&self, response: ClientResponse) {
        if self.alive.load(Ordering::SeqCst) && self.replies.try_send(response).is_err() {
            // A slow client cannot hold control indefinitely or grow a queue.
            self.alive.store(false, Ordering::SeqCst);
        }
    }
}
struct View {
    scope: ReplyScope,
    alive: Arc<AtomicBool>,
    frames: watch::Sender<Option<Arc<ClientResponse>>>,
    selection: String,
    generation: String,
    revision: u64,
    size: TerminalSize,
    viewport: TerminalViewport,
}
impl View {
    fn live(&self) -> bool {
        self.alive.load(Ordering::SeqCst) && self.scope.alive.load(Ordering::SeqCst)
    }
}
struct Owner {
    view_id: String,
    attach_id: String,
}
enum Job {
    Add {
        id: String,
        view: View,
    },
    Viewport {
        scope: ReplyScope,
        data: TerminalViewportData,
    },
    Control {
        scope: ReplyScope,
        data: ClientTerminalControlData,
    },
    Legacy {
        scope: ReplyScope,
        line: Vec<u8>,
    },
}
impl Job {
    fn scope(&self) -> &ReplyScope {
        match self {
            Self::Add { view, .. } => &view.scope,
            Self::Viewport { scope, .. }
            | Self::Control { scope, .. }
            | Self::Legacy { scope, .. } => scope,
        }
    }
    fn request_id(&self) -> Option<String> {
        match self {
            Self::Add { view, .. } => Some(view.selection.clone()),
            Self::Viewport { data, .. } => Some(data.request_id.clone()),
            Self::Control { data, .. } => Some(data.request_id.clone()),
            Self::Legacy { .. } => None,
        }
    }
}

impl TerminalHub {
    pub fn new(runtime: HolderRuntime, fleet: Arc<Fleet>) -> Self {
        Self::with_input_policy(runtime, fleet, Default::default())
    }
    pub fn with_input_policy(
        runtime: HolderRuntime,
        fleet: Arc<Fleet>,
        codex_input: agend_core::policy::codex_input::CodexInputPolicy,
    ) -> Self {
        static NEXT_HUB: AtomicU64 = AtomicU64::new(1);
        let nonce = format!(
            "{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT_HUB.fetch_add(1, Ordering::Relaxed)
        );
        Self(Arc::new(Inner {
            runtime,
            fleet,
            codex_input,
            codex: None,
            next: AtomicU64::new(1),
            nonce,
            actors: Mutex::new(BTreeMap::new()),
        }))
    }

    /// Production server admission uses the live driver, not an instance-only policy.
    pub fn with_codex_driver(
        runtime: HolderRuntime,
        fleet: Arc<Fleet>,
        codex: crate::driver::codex::CodexDriver,
    ) -> Self {
        let mut hub = Self::new(runtime, fleet);
        Arc::get_mut(&mut hub.0).unwrap().codex = Some(codex);
        hub
    }

    pub fn subscribe(
        &self,
        scope: ReplyScope,
        data: TerminalSubscribeData,
    ) -> Result<ViewStream, ErrorData> {
        let id = self.token("view");
        let alive = Arc::new(AtomicBool::new(true));
        let (frames, receiver) = watch::channel(None);
        let view = View {
            scope,
            alive: alive.clone(),
            frames,
            selection: data.request_id,
            generation: String::new(),
            revision: 0,
            size: TerminalSize {
                rows: 1,
                columns: 1,
            },
            viewport: data.viewport,
        };
        self.enqueue(
            &data.instance_id,
            Job::Add {
                id: id.clone(),
                view,
            },
        )?;
        Ok(ViewStream {
            instance_id: data.instance_id,
            view_id: id,
            alive,
            frames: receiver,
        })
    }

    pub fn viewport(
        &self,
        scope: ReplyScope,
        view: &ViewStream,
        data: TerminalViewportData,
    ) -> Result<(), ErrorData> {
        self.check_view(view, &data.instance_id, &data.view_id, &data.request_id)?;
        self.enqueue(&data.instance_id.clone(), Job::Viewport { scope, data })
    }
    pub fn control(
        &self,
        scope: ReplyScope,
        view: &ViewStream,
        data: ClientTerminalControlData,
    ) -> Result<(), ErrorData> {
        self.check_view(view, &data.instance_id, &data.view_id, &data.request_id)?;
        self.enqueue(&data.instance_id.clone(), Job::Control { scope, data })
    }
    pub fn legacy(
        &self,
        scope: ReplyScope,
        instance: &str,
        line: Vec<u8>,
    ) -> Result<(), ErrorData> {
        self.enqueue(instance, Job::Legacy { scope, line })
    }
    pub(crate) fn live_instance(&self, instance: &str, request: &str) -> Result<(), ErrorData> {
        if self
            .0
            .fleet
            .instance(instance)
            .is_none_or(|v| v.state == AgentState::Failed)
            || !self.0.runtime.has_link(instance)
        {
            return Err(reject(
                Some(request.into()),
                "no_terminal",
                "instance has no live terminal; nothing was written",
            ));
        }
        Ok(())
    }
    fn check_view(
        &self,
        view: &ViewStream,
        instance: &str,
        id: &str,
        request: &str,
    ) -> Result<(), ErrorData> {
        if view.instance_id != instance || view.view_id != id || !view.alive.load(Ordering::SeqCst)
        {
            return Err(reject(
                Some(request.into()),
                "stale_terminal",
                "this view is not attached to this client connection; subscribe again",
            ));
        }
        Ok(())
    }
    fn token(&self, kind: &str) -> String {
        token(&self.0, kind)
    }
    fn enqueue(&self, instance: &str, job: Job) -> Result<(), ErrorData> {
        // Removal and enqueue use the same lock: a last-view EOF cannot lose
        // a concurrent subscribe in a retiring actor's queue.
        let mut actors = lock(&self.0.actors);
        let handle = actors.entry(instance.into()).or_insert_with(|| {
            let identity = self.0.next.fetch_add(1, Ordering::Relaxed);
            let (sender, jobs) = mpsc::channel(QUEUE);
            let actor = Actor {
                instance: instance.into(),
                identity,
                hub: Arc::downgrade(&self.0),
                runtime: self.0.runtime.clone(),
                fleet: self.0.fleet.clone(),
                codex_input: self.0.codex_input.clone(),
                codex: self.0.codex.clone(),
                jobs,
                views: BTreeMap::new(),
                owner: None,
                connection: None,
                notices: None,
                dirty: true,
                last_sample: None,
                last_notice: None,
            };
            Handle {
                identity,
                jobs: sender,
                task: tokio::spawn(actor.run()),
            }
        });
        handle.jobs.try_send(job).map_err(|e| {
            let job = e.into_inner();
            reject(
                job.request_id(),
                "pty_busy",
                "terminal operation queue is full; nothing was written",
            )
        })
    }
    /// Stop all actors as part of stopping the client server. Dropping pending
    /// runtime controls invalidates their epoch; holder releases the owner.
    pub fn stop(&self) {
        for (_, handle) in std::mem::take(&mut *lock(&self.0.actors)) {
            handle.task.abort();
        }
    }
}
fn token(inner: &Inner, kind: &str) -> String {
    format!(
        "{kind}-{}-{}-{}-{}",
        std::process::id(),
        inner.fleet.base(),
        inner.nonce,
        inner.next.fetch_add(1, Ordering::Relaxed)
    )
}

fn valid_rows(rows: u16) -> bool {
    (1..=1000).contains(&rows)
}
fn valid_size(size: TerminalSize) -> bool {
    static MINIMUM: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let minimum = MINIMUM.get_or_init(|| {
        serde_json::to_vec(&TerminalCell {
            text: String::new(),
            width: 0,
            foreground: TerminalColor::Foreground,
            background: TerminalColor::Background,
            underline_color: None,
            style: 0,
            leading_spacer: true,
            wrap: true,
        })
        .unwrap()
        .len()
    });
    size.is_valid()
        && usize::from(size.rows) * usize::from(size.columns) * minimum <= MAX_FRAME_LINE
}
fn frame_response(
    instance: &str,
    view: &str,
    selection: &str,
    frame: TerminalFrame,
) -> ClientResponse {
    ClientResponse::TerminalFrame {
        data: ClientTerminalFrameData {
            request_id: selection.into(),
            instance_id: instance.into(),
            view_id: view.into(),
            frame,
        },
    }
}

pub(crate) fn reject(
    request_id: Option<String>,
    code: &str,
    message: impl Into<String>,
) -> ErrorData {
    ErrorData {
        request_id,
        code: code.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejected_subscription_actors_retire_without_leaving_a_task_per_instance() {
        let runtime = HolderRuntime::new(
            std::path::Path::new("/tmp/unused-g11-hub"),
            std::path::Path::new("/nonexistent/agend"),
            Vec::new(),
            Arc::new(|_| {}),
        );
        let hub = TerminalHub::new(runtime, Arc::new(Fleet::new(1)));
        for n in 0..20 {
            let (replies, mut receiver) = mpsc::channel(8);
            let scope = ReplyScope {
                client: n,
                alive: Arc::new(AtomicBool::new(true)),
                replies,
            };
            let view = hub
                .subscribe(
                    scope,
                    TerminalSubscribeData {
                        request_id: format!("missing-{n}"),
                        instance_id: format!("missing-{n}"),
                        viewport: TerminalViewport {
                            top: None,
                            rows: 10,
                        },
                    },
                )
                .unwrap();
            let response = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(response, ClientResponse::Error { data } if data.code == "no_terminal")
            );
            drop(view);
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while !lock(&hub.0.actors).is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("retired instances leaked actor handles");
    }
}
