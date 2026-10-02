//! Fake full-terminal server policy with an injected real frame producer.
//! Each instance has one bounded operation worker; socket readers never wait
//! for a resize or write. Frames coalesce independently of ordered replies.

use super::*;
use agend_core::protocol::client::*;
use agend_core::protocol::terminal::*;
use agend_core::traits::TerminalProducer;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{RecvTimeoutError, TryRecvError};

mod control;

#[derive(Clone)]
pub(super) struct Scope(Arc<ScopeState>);
struct ScopeState {
    id: u64,
    writer: Writer,
    alive: AtomicBool,
    selection: AtomicU64,
    replies: SyncSender<ClientResponse>,
    frame: Mutex<Option<ClientResponse>>,
}
impl Scope {
    pub(super) fn start(id: u64, writer: Writer) -> io::Result<Self> {
        let (replies, receiver) = sync_channel(8);
        let scope = Self(Arc::new(ScopeState {
            id,
            writer,
            alive: AtomicBool::new(true),
            selection: AtomicU64::new(0),
            replies,
            frame: Mutex::new(None),
        }));
        let forward = scope.clone();
        std::thread::Builder::new()
            .name("fake-full-replies".into())
            .spawn(move || {
                while forward.live() {
                    match receiver.try_recv() {
                        Ok(reply) => {
                            if send(&forward.0.writer, &reply).is_err() {
                                forward.close();
                            }
                            continue;
                        }
                        Err(TryRecvError::Disconnected) => break,
                        Err(TryRecvError::Empty) => (),
                    }
                    let frame = lock(&forward.0.frame).take();
                    if let Some(frame) = frame
                        && send(&forward.0.writer, &frame).is_err()
                    {
                        forward.close();
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            })?;
        Ok(scope)
    }
    pub(super) fn close(&self) {
        self.0.alive.store(false, Ordering::SeqCst);
        self.replace_view();
    }
    fn live(&self) -> bool {
        self.0.alive.load(Ordering::SeqCst)
    }
    pub(super) fn replace_view(&self) -> u64 {
        let next = self.0.selection.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&self.0.frame).take();
        next
    }
    fn publish(&self, response: ClientResponse) {
        if !self.live() {
            return;
        }
        if self.0.replies.try_send(response).is_err() {
            self.close();
            let _ = lock(&self.0.writer).shutdown(Shutdown::Both);
        }
    }
    fn publish_frame(&self, selection: u64, response: ClientResponse) {
        let mut latest = lock(&self.0.frame);
        if self.live() && selection == self.0.selection.load(Ordering::SeqCst) {
            *latest = Some(response);
        }
    }
    fn fail(&self, id: Option<String>, code: &str, message: &str) {
        self.publish(error(id, code, message.into()));
    }
}

pub(super) struct Endpoint {
    queue: SyncSender<(Scope, Job)>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    live: Arc<AtomicBool>,
}
enum Job {
    Subscribe(TerminalSubscribeData, u64),
    Viewport(TerminalViewportData),
    Control(ClientTerminalControlData),
    Legacy(String),
}
impl Endpoint {
    pub(super) fn start(instance: &str, producer: Box<dyn TerminalProducer>) -> io::Result<Self> {
        let (queue, receiver) = sync_channel::<(Scope, Job)>(64);
        let stop = Arc::new(AtomicBool::new(false));
        let live = Arc::new(AtomicBool::new(true));
        let mut actor = Actor {
            instance: instance.into(),
            producer,
            views: BTreeMap::new(),
            owner: None,
            size: None,
            next: 0,
            nonce: {
                static NEXT: AtomicU64 = AtomicU64::new(0);
                format!("{}-{}", now_unix_ms(), NEXT.fetch_add(1, Ordering::SeqCst))
            },
            live: Arc::clone(&live),
        };
        let stopped = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("fake-full-terminal".into())
            .spawn(move || {
                let mut last_sample = std::time::Instant::now();
                while !stopped.load(Ordering::SeqCst) {
                    actor.cleanup();
                    match receiver.recv_timeout(Duration::from_millis(5)) {
                        Ok((scope, job)) if scope.live() => actor.job(scope, job),
                        Ok(_) | Err(RecvTimeoutError::Timeout) => (),
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    if last_sample.elapsed() >= Duration::from_millis(50) {
                        actor.capture();
                        last_sample = std::time::Instant::now();
                    }
                }
            })?;
        Ok(Self {
            queue,
            stop,
            thread: Some(thread),
            live,
        })
    }
    pub(super) fn set_live(&self, live: bool) {
        self.live.store(live, Ordering::SeqCst);
    }
    fn enqueue(&self, scope: Scope, job: Job, id: Option<String>) {
        if self.queue.try_send((scope.clone(), job)).is_err() {
            scope.fail(
                id,
                error_code::PTY_BUSY,
                "terminal queue is full; nothing was written",
            );
        }
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
struct View {
    scope: Scope,
    selection: u64,
    id: String,
    request_id: String,
    generation: String,
    viewport: TerminalViewport,
    revision: u64,
}
impl View {
    fn live(&self) -> bool {
        self.scope.live() && self.selection == self.scope.0.selection.load(Ordering::SeqCst)
    }
}
struct Owner {
    view: String,
    attach: String,
    generation: String,
}
struct Actor {
    instance: String,
    producer: Box<dyn TerminalProducer>,
    views: BTreeMap<String, View>,
    owner: Option<Owner>,
    size: Option<TerminalSize>,
    next: u64,
    nonce: String,
    live: Arc<AtomicBool>,
}
impl Drop for Actor {
    fn drop(&mut self) {
        self.release();
    }
}
impl Actor {
    fn token(&mut self, kind: &str) -> String {
        self.next += 1;
        format!("fake-{kind}-{}-{}-{}", self.instance, self.nonce, self.next)
    }
    fn release(&mut self) {
        if let Some(owner) = self.owner.take() {
            let _ = self.producer.control(TerminalControlRequest {
                request_id: "eof-release".into(),
                generation: owner.generation,
                operation: TerminalControlOperation::Release {
                    attach_id: owner.attach,
                },
            });
        }
    }
    fn cleanup(&mut self) {
        if !self.live.load(Ordering::SeqCst) {
            self.invalidate(error_code::NO_TERMINAL, "instance has no live terminal");
        }
        if self
            .owner
            .as_ref()
            .is_some_and(|owner| self.views.get(&owner.view).is_none_or(|v| !v.live()))
        {
            self.release();
        }
        self.views.retain(|_, view| view.live());
    }
    fn job(&mut self, scope: Scope, job: Job) {
        if !self.live.load(Ordering::SeqCst) {
            let id = match &job {
                Job::Subscribe(data, _) => Some(data.request_id.clone()),
                Job::Viewport(data) => Some(data.request_id.clone()),
                Job::Control(data) => Some(data.request_id.clone()),
                Job::Legacy(_) => None,
            };
            scope.fail(id, error_code::NO_TERMINAL, "instance has no live terminal");
            return;
        }
        match job {
            Job::Subscribe(data, selection) => {
                if selection != scope.0.selection.load(Ordering::SeqCst) {
                    return;
                }
                self.cleanup();
                if data.viewport.rows == 0 || data.viewport.rows > MAX_SCREEN_SIDE {
                    scope.fail(
                        Some(data.request_id),
                        error_code::INVALID_SIZE,
                        "invalid viewport rows",
                    );
                    return;
                }
                // Ask for the producer's real size first; clamp requested view rows.
                let first = self.producer.frame(TerminalFrameRequest {
                    request_id: data.request_id.clone(),
                    viewport: TerminalViewport { top: None, rows: 1 },
                });
                let frame = match first {
                    Ok(data) => data.frame,
                    Err(error) => {
                        scope.fail(Some(data.request_id), &error.code, &error.message);
                        return;
                    }
                };
                self.size = Some(frame.size);
                let id = self.token("view");
                self.views.insert(
                    id.clone(),
                    View {
                        scope,
                        selection,
                        id: id.clone(),
                        request_id: data.request_id,
                        generation: frame.generation,
                        viewport: data.viewport,
                        revision: 0,
                    },
                );
                self.capture_view(&id, true);
            }
            Job::Viewport(data) => {
                if !self.valid_view(&scope, &data.view_id, &data.generation) {
                    scope.fail(
                        Some(data.request_id),
                        error_code::STALE_TERMINAL,
                        "view or generation changed",
                    );
                    return;
                }
                if data.viewport.rows == 0 || data.viewport.rows > MAX_SCREEN_SIDE {
                    scope.fail(
                        Some(data.request_id),
                        error_code::INVALID_SIZE,
                        "invalid viewport rows",
                    );
                    return;
                }
                let view = self.views.get_mut(&data.view_id).unwrap();
                view.request_id = data.request_id;
                view.viewport = data.viewport;
                self.capture_view(&data.view_id, true);
            }
            Job::Control(data) => self.control(scope, data),
            Job::Legacy(bytes) => {
                self.cleanup();
                if self.owner.is_some() {
                    scope.fail(
                        None,
                        error_code::CONTROL_REQUIRED,
                        "a full-terminal view owns this PTY",
                    );
                } else if let Err(error) = self.producer.legacy_input(bytes) {
                    scope.fail(None, &error.code, &error.message);
                }
            }
        }
    }
    fn valid_view(&self, scope: &Scope, id: &str, generation: &str) -> bool {
        self.views.get(id).is_some_and(|view| {
            view.live() && view.scope.0.id == scope.0.id && view.generation == generation
        })
    }
    fn invalidate(&mut self, code: &str, reason: &str) {
        for view in self.views.values() {
            view.scope.publish(ClientResponse::TerminalControlChanged {
                data: TerminalControlChangedData {
                    instance_id: self.instance.clone(),
                    view_id: view.id.clone(),
                    generation: view.generation.clone(),
                    control: TerminalControlState::ReadOnly,
                    reason: reason.into(),
                },
            });
            view.scope.fail(Some(view.request_id.clone()), code, reason);
            view.scope.replace_view();
        }
        self.release();
        self.views.clear();
    }
    fn capture(&mut self) {
        for id in self.views.keys().cloned().collect::<Vec<_>>() {
            self.capture_view(&id, false);
        }
    }
    fn capture_view(&mut self, id: &str, force: bool) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        if !view.live() {
            return;
        }
        let rows = view
            .viewport
            .rows
            .min(self.size.map_or(view.viewport.rows, |s| s.rows));
        match self.producer.frame(TerminalFrameRequest {
            request_id: view.request_id.clone(),
            viewport: TerminalViewport {
                rows,
                ..view.viewport
            },
        }) {
            Ok(data) if data.frame.generation == view.generation => {
                if !view.live() {
                    return;
                }
                if data.frame.revision < view.revision {
                    return;
                }
                if !force && data.frame.revision == view.revision {
                    return;
                }
                view.revision = data.frame.revision;
                self.size = Some(data.frame.size);
                view.scope.publish_frame(
                    view.selection,
                    ClientResponse::TerminalFrame {
                        data: ClientTerminalFrameData {
                            request_id: view.request_id.clone(),
                            instance_id: self.instance.clone(),
                            view_id: view.id.clone(),
                            frame: data.frame,
                        },
                    },
                );
            }
            Ok(_) => self.invalidate(error_code::STALE_TERMINAL, "producer generation changed"),
            Err(error) => self.invalidate(&error.code, &error.message),
        }
    }
}

/// Returns true if the request was handled outside the legacy handler.
pub(super) fn dispatch(
    shared: &Shared,
    conn: &Connection,
    request: &ClientRequest,
    length: usize,
) -> io::Result<bool> {
    let (instance, id, control) = match request {
        ClientRequest::SubscribeTerminalFrames { data } => {
            (&data.instance_id, Some(data.request_id.clone()), false)
        }
        ClientRequest::SetTerminalViewport { data } => {
            (&data.instance_id, Some(data.request_id.clone()), false)
        }
        ClientRequest::TerminalControl { data } => {
            (&data.instance_id, Some(data.request_id.clone()), true)
        }
        ClientRequest::TerminalInput { data } => (&data.instance_id, None, true),
        _ => return Ok(false),
    };
    let legacy = matches!(request, ClientRequest::TerminalInput { .. });
    let mut state = lock(&shared.state);
    if legacy && !state.full_terminals.contains_key(instance) {
        return Ok(false);
    }
    if matches!(request, ClientRequest::SubscribeTerminalFrames { .. }) {
        conn.full_scope.replace_view();
        state.terminals.retain(|t| t.connection != conn.id);
    }
    let fail = |code: &str, message: &str| {
        conn.full_scope.fail(id.clone(), code, message);
        Ok(true)
    };
    if control && conn.caller.is_some() {
        return fail(error_code::FORBIDDEN, TYPE_OPERATOR_ONLY);
    }
    if !legacy && conn.selected < V1_4 {
        return fail(
            error_code::NOT_SUPPORTED,
            "full terminals require client protocol 1.4",
        );
    }
    if !legacy && length > MAX_REQUEST_LINE {
        return fail(
            error_code::INVALID_REQUEST,
            "terminal request exceeds 1 MiB; nothing changed",
        );
    }
    let Some(live) = state
        .instances
        .iter()
        .find(|i| i.instance_id == *instance && i.state != AgentState::Failed)
    else {
        return fail(error_code::NO_TERMINAL, "instance has no live terminal");
    };
    if live.backend == "codex"
        && (legacy
            || matches!(request, ClientRequest::TerminalControl { data } if matches!(data.operation, ClientTerminalOperation::Acquire { .. } | ClientTerminalOperation::Input { .. })))
    {
        return fail(error_code::NOT_SUPPORTED, CODEX_INPUT);
    }
    let Some(endpoint) = state.full_terminals.get(instance) else {
        return fail(
            error_code::NOT_SUPPORTED,
            "no terminal producer was installed",
        );
    };
    let job = match request.clone() {
        ClientRequest::SubscribeTerminalFrames { data } => {
            Job::Subscribe(data, conn.full_scope.0.selection.load(Ordering::SeqCst))
        }
        ClientRequest::SetTerminalViewport { data } => Job::Viewport(data),
        ClientRequest::TerminalControl { data } => Job::Control(data),
        ClientRequest::TerminalInput { data } => {
            let holder = HolderRequest::OperatorTerminalInput {
                data: OperatorTerminalInputData {
                    bytes_base64: data.bytes_base64.clone(),
                },
            };
            let size = serde_json::to_vec(&holder).map_or(usize::MAX, |line| line.len() + 1);
            if size > MAX_REQUEST_LINE {
                return fail(error_code::INVALID_REQUEST, &operator_input_too_long(size));
            }
            Job::Legacy(data.bytes_base64)
        }
        _ => unreachable!(),
    };
    endpoint.enqueue(conn.full_scope.clone(), job, id);
    Ok(true)
}
