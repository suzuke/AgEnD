//! Per-instance lifecycle, dirty sampling and bounded job execution.
use super::*;

pub(super) struct Actor {
    pub(super) instance: String,
    pub(super) identity: u64,
    pub(super) hub: Weak<Inner>,
    pub(super) runtime: HolderRuntime,
    pub(super) fleet: Arc<Fleet>,
    pub(super) codex_input: agend_core::policy::codex_input::CodexInputPolicy,
    pub(super) codex: Option<crate::driver::codex::CodexDriver>,
    pub(super) jobs: mpsc::Receiver<Job>,
    pub(super) views: BTreeMap<String, View>,
    pub(super) owner: Option<Owner>,
    pub(super) connection: Option<TerminalConnection>,
    pub(super) notices: Option<watch::Receiver<TerminalNotice>>,
    pub(super) dirty: bool,
    pub(super) last_sample: Option<Instant>,
    pub(super) last_notice: Option<Instant>,
    pub(super) dirty_since: Option<Instant>,
}
async fn notice(
    receiver: &mut Option<watch::Receiver<TerminalNotice>>,
) -> Result<(), watch::error::RecvError> {
    match receiver {
        Some(r) => r.changed().await,
        None => std::future::pending().await,
    }
}
impl Actor {
    pub(super) fn codex_input_allowed(&self) -> bool {
        self.codex.as_ref().map_or_else(
            || self.codex_input.allows_instance(&self.instance),
            |driver| driver.can_input(&self.instance),
        )
    }
    pub(super) async fn run(mut self) {
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                job = self.jobs.recv() => match job {
                    Some(job) => {
                        self.cleanup().await;
                        if job.scope().alive.load(Ordering::SeqCst) { self.job(job).await; }
                    },
                    None => break,
                },
                _ = notice(&mut self.notices) => {
                    self.dirty = true;
                    self.last_notice = Some(Instant::now());
                    self.dirty_since.get_or_insert_with(Instant::now);
                },
                _ = tick.tick() => {},
            }
            self.cleanup().await;
            if self.dirty
                && !self.views.is_empty()
                && self
                    .dirty_since
                    .is_none_or(|at| at.elapsed() >= SAMPLE_EVERY)
                && self
                    .last_sample
                    .is_none_or(|at| at.elapsed() >= SAMPLE_EVERY)
            {
                self.capture().await;
            }
            if self.views.is_empty() && self.retire() {
                break;
            }
        }
    }
    fn retire(&mut self) -> bool {
        let Some(hub) = self.hub.upgrade() else {
            return true;
        };
        let mut actors = lock(&hub.actors);
        if self.jobs.is_empty()
            && actors
                .get(&self.instance)
                .is_some_and(|h| h.identity == self.identity)
        {
            self.jobs.close();
            actors.remove(&self.instance);
            true
        } else {
            false
        }
    }
    pub(super) fn live(&self) -> bool {
        self.fleet
            .instance(&self.instance)
            .is_some_and(|v| v.state != AgentState::Failed)
            && self.runtime.has_link(&self.instance)
    }
    pub(super) async fn cleanup(&mut self) {
        if self.connection.as_ref().is_some_and(|c| !c.is_current())
            || (!self.views.is_empty() && !self.live())
        {
            self.invalidate(
                "terminal disconnected; subscribe again and acquire control explicitly",
            );
        }
        if self
            .owner
            .as_ref()
            .is_some_and(|o| self.views.get(&o.view_id).is_none_or(|v| !v.live()))
        {
            self.release().await;
        }
        self.views.retain(|_, v| v.live());
    }
    pub(super) fn changed(&self, owner: &Owner, reason: &str) {
        if let Some(view) = self.views.get(&owner.view_id) {
            view.scope.send(ClientResponse::TerminalControlChanged {
                data: TerminalControlChangedData {
                    instance_id: self.instance.clone(),
                    view_id: owner.view_id.clone(),
                    generation: view.generation.clone(),
                    control: TerminalControlState::ReadOnly,
                    reason: reason.into(),
                },
            });
        }
    }
    pub(super) fn invalidate(&mut self, reason: &str) {
        if let Some(connection) = &self.connection {
            connection.invalidate(reason);
        }
        if let Some(owner) = self.owner.take() {
            self.changed(&owner, reason);
        }
        for view in self.views.values() {
            view.scope.send(error(
                Some(view.selection.clone()),
                "stale_terminal",
                reason,
            ));
            view.alive.store(false, Ordering::SeqCst);
        }
        self.views.clear();
        self.connection = None;
        self.notices = None;
        self.last_notice = None;
        self.dirty_since = None;
        self.dirty = true;
    }
    pub(super) async fn release(&mut self) {
        if let Some(owner) = self.owner.take()
            && let (Some(connection), Some(view)) =
                (&self.connection, self.views.get(&owner.view_id))
        {
            let result = connection
                .control(
                    view.generation.clone(),
                    TerminalControlOperation::Release {
                        attach_id: owner.attach_id.clone(),
                    },
                )
                .await;
            if let Err(e) = result
                && (e.code != "control_lost" || !connection.is_current())
            {
                self.invalidate(&e.message);
            }
        }
    }
    pub(super) fn valid_view(
        &self,
        scope: &ReplyScope,
        id: &str,
        generation: &str,
    ) -> Result<(), &'static str> {
        let view = self
            .views
            .get(id)
            .filter(|v| v.live() && v.scope.client == scope.client)
            .ok_or("view is no longer attached to this client connection")?;
        if view.generation != generation {
            return Err("holder generation changed; subscribe again");
        }
        if self.connection.as_ref().is_none_or(|c| !c.is_current()) {
            return Err("holder connection changed; subscribe again");
        }
        Ok(())
    }
    async fn job(&mut self, job: Job) {
        match job {
            Job::Add { id, mut view } => {
                if !view.live() {
                    return;
                }
                let refuse = |code, message: &str| {
                    view.scope
                        .send(error(Some(view.selection.clone()), code, message))
                };
                if !self.live() {
                    refuse("no_terminal", "instance has no live terminal");
                    return;
                }
                if !valid_rows(view.viewport.rows) {
                    refuse("invalid_size", "viewport rows must be between 1 and 1000");
                    return;
                }
                if self.connection.is_none() {
                    match self.runtime.terminal_connection(&self.instance) {
                        Ok(c) => {
                            self.notices = Some(c.notices());
                            self.connection = Some(c);
                        }
                        Err(e) => {
                            refuse(&e.code, &e.message);
                            return;
                        }
                    }
                }
                // Learn the actual PTY dimensions without assuming its previous
                // controller's size; read only the requested visible rows next.
                let connection = self.connection.as_ref().unwrap();
                let metadata = match connection
                    .frame(TerminalViewport {
                        top: view.viewport.top,
                        rows: 1,
                    })
                    .await
                {
                    Ok(frame) => frame,
                    Err(e) => {
                        refuse(&e.code, &e.message);
                        return;
                    }
                };
                view.size = metadata.size;
                view.generation = metadata.generation;

                let frame = match connection
                    .frame(TerminalViewport {
                        top: view.viewport.top,
                        rows: view.viewport.rows.min(view.size.rows),
                    })
                    .await
                {
                    Ok(frame) => frame,
                    Err(e) => {
                        refuse(&e.code, &e.message);
                        return;
                    }
                };
                if !view.live() || !connection.is_current() {
                    return;
                }
                view.revision = frame.revision;
                view.scope
                    .send(frame_response(&self.instance, &id, &view.selection, frame));
                self.views.insert(id, view);
                self.last_sample = Some(Instant::now());
                self.dirty = true;
            }
            Job::Viewport { scope, data } => {
                if let Err(message) = self.valid_view(&scope, &data.view_id, &data.generation) {
                    scope.send(error(Some(data.request_id), "stale_terminal", message));
                    return;
                }
                if !valid_rows(data.viewport.rows) {
                    scope.send(error(
                        Some(data.request_id),
                        "invalid_size",
                        "viewport rows must be between 1 and 1000",
                    ));
                    return;
                }
                let view = self.views.get_mut(&data.view_id).unwrap();
                view.selection = data.request_id;
                view.viewport = TerminalViewport {
                    top: data.viewport.top,
                    rows: data.viewport.rows,
                };
                view.frames.send_replace(None);
                self.dirty = true;
            }
            Job::Control { scope, data } => self.control(scope, data).await,
            Job::Legacy { scope, line } => {
                if self
                    .fleet
                    .instance(&self.instance)
                    .is_some_and(|v| v.backend == "codex")
                    && !self.codex_input_allowed()
                {
                    scope.send(error(None, "not_supported", CODEX_INPUT));
                    return;
                }
                if self.owner.is_some() {
                    scope.send(error(None, "control_required", "another full-terminal view controls this PTY; acquire control before typing"));
                    return;
                }
                let runtime = self.runtime.clone();
                let instance = self.instance.clone();
                // The old holder write is blocking, but not on the socket's
                // reader task. Keep it ordered with grants in this actor.
                if !tokio::task::spawn_blocking(move || runtime.terminal_input(&instance, line))
                    .await
                    .unwrap_or(false)
                {
                    scope.send(error(
                        None,
                        "no_terminal",
                        "instance has no live terminal; nothing was written",
                    ));
                }
            }
        }
    }
    async fn capture(&mut self) {
        // Wait one shared-cache interval from the first dirty notice, rather
        // than serializing a predictably stale frame and fetching it again.
        // Later notices do not extend that wait, so continuous output streams.
        // Other readers (including startup) share the holder's 50 ms
        // sample. A sample requested too soon after output can still be old.
        // Preserve dirty until a follow-up starts after that cache expires;
        // compare at request start, not after slow frame IO completes.
        let settled = self
            .last_notice
            .is_none_or(|at| at.elapsed() >= SAMPLE_EVERY);
        let sequence = self.notices.as_ref().map(|n| n.borrow().output_sequence);
        let ids: Vec<_> = self.views.keys().cloned().collect();
        for id in ids {
            let Some(view) = self.views.get(&id).filter(|v| v.live()) else {
                continue;
            };
            let viewport = TerminalViewport {
                top: view.viewport.top,
                rows: view.viewport.rows.min(view.size.rows),
            };
            let Some(connection) = &self.connection else {
                break;
            };
            match connection.frame(viewport).await {
                Ok(frame) => {
                    let view = self.views.get_mut(&id).unwrap();
                    if frame.generation != view.generation {
                        self.invalidate("holder generation changed; subscribe again");
                        break;
                    }
                    if frame.revision < view.revision {
                        continue;
                    }
                    view.size = frame.size;
                    view.revision = frame.revision;
                    view.frames.send_replace(Some(Arc::new(frame_response(
                        &self.instance,
                        &id,
                        &view.selection,
                        frame,
                    ))));
                }
                Err(e) => {
                    if self.connection.as_ref().is_some_and(|c| !c.is_current()) {
                        self.invalidate(&e.message);
                        break;
                    }
                    self.views.get(&id).unwrap().scope.send(error(
                        Some(self.views[&id].selection.clone()),
                        &e.code,
                        e.message,
                    ));
                }
            }
        }
        self.last_sample = Some(Instant::now());
        self.dirty_since = None;
        self.dirty =
            !settled || self.notices.as_ref().map(|n| n.borrow().output_sequence) != sequence;
    }
}

impl Drop for Actor {
    fn drop(&mut self) {
        if self.owner.is_some()
            && let Some(connection) = &self.connection
        {
            connection.invalidate(
                "terminal controller stopped; subscribe again and acquire control explicitly",
            );
        }
    }
}
