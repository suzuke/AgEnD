//! Client 1.5 service. Fresh hooks allow routing; newer historical routing
//! observations only revoke idle. Durable reservations precede the reply,
//! and a lost reply never grants permission to resend.
use crate::{
    handlers::{Context, error},
    log::now_unix_ms,
    store::{SqliteStore, StoreError},
};
use agend_core::{
    model::Backend,
    protocol::{ProtocolVersion, client::*},
    runtime_records::{
        ClaudeAck, ClaudeReservation, ClaudeRoute, InstanceStatus, NewClaudeDelivery,
        NewDriverEvent,
    },
    screen::{SCREEN_RULES, classify},
};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
pub(crate) mod startup;

#[derive(Default)]
struct State {
    session: String,
    idle: Option<Instant>,
    last_route_hook: u64,
    revision: u64,
    busy: bool,
    reported_busy: Option<bool>,
    initial: bool,
    ready_generation: Option<String>,
    idle_connection: Option<crate::runtime::terminal::TerminalConnection>,
    routing_connection: Option<crate::runtime::terminal::TerminalConnection>,
}
#[derive(Default)]
pub(crate) struct ClaudeBridge {
    states: Mutex<BTreeMap<String, State>>,
}

impl ClaudeBridge {
    /// An idle hook candidate plus a current holder observation. Callers must
    /// pause/drain input and separately verify managed launch identity.
    pub async fn session_idle(&self, ctx: &Context, id: &str) -> Result<bool, String> {
        let instance = ctx
            .store
            .instance(id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("Claude instance missing")?;
        if instance.backend != Backend::Claude
            || instance.status != InstanceStatus::Running
            || instance.delivery != "push"
            || !instance.session_started
        {
            return Err("not a running Claude push instance".into());
        }
        let session = instance
            .session_id
            .as_deref()
            .ok_or("Claude session missing")?;
        let (revision, idle, initial, generation, connection) = {
            let states = self.states.lock().await;
            let Some(state) = states.get(id).filter(|s| s.session == session && !s.busy) else {
                return Ok(false);
            };
            let Some(idle) = state
                .idle
                .filter(|at| at.elapsed() >= Duration::from_secs(5))
            else {
                return Ok(false);
            };
            let Some(connection) = state.idle_connection.clone().filter(|c| c.is_current()) else {
                return Ok(false);
            };
            (
                state.revision,
                idle,
                state.initial,
                state.ready_generation.clone(),
                connection,
            )
        };
        if initial {
            let ready = startup::frame(&connection).await.is_some_and(|frame| {
                generation.as_deref() == Some(frame.generation.as_str())
                    && startup::prompt(&frame, &instance.working_directory)
                        == Some(agend_core::screen::claude_startup::Prompt::Ready)
            });
            if !ready {
                return Ok(false);
            }
        }
        let Some(feed) = ctx.runtime.live_terminal(id) else {
            return Ok(false);
        };
        let Ok(Ok((screen, _))) = tokio::time::timeout(Duration::from_secs(2), feed).await else {
            return Ok(false);
        };
        if classify(Backend::Claude, &screen, SCREEN_RULES).is_some()
            || !connection.is_current()
            || ctx
                .store
                .instance(id)
                .await
                .map_err(|e| e.to_string())?
                .as_ref()
                != Some(&instance)
        {
            return Ok(false);
        }
        let states = self.states.lock().await;
        Ok(connection.is_current()
            && states.get(id).is_some_and(|state| {
                state.session == session
                    && state.revision == revision
                    && !state.busy
                    && state.idle == Some(idle)
                    && state.initial == initial
                    && state.ready_generation == generation
            }))
    }

    pub async fn handle(
        &self,
        ctx: &Arc<Context>,
        caller: Option<&str>,
        selected: ProtocolVersion,
        data: ClaudeRequestData,
    ) -> ClientResponse {
        let id = data.request_id.clone();
        if caller != Some(data.instance_id.as_str()) {
            return error(
                Some(id),
                error_code::FORBIDDEN,
                "Claude helper caller must match its instance",
            );
        }
        if selected < V1_5 {
            return error(
                Some(id),
                error_code::NOT_SUPPORTED,
                "Claude helpers require client protocol 1.5",
            );
        }
        match self.execute(ctx, data).await {
            Ok(data) => ClientResponse::Claude { data },
            Err(e) => error(Some(id), error_code::INVALID_REQUEST, e.to_string()),
        }
    }
    async fn execute(
        &self,
        ctx: &Context,
        data: ClaudeRequestData,
    ) -> Result<ClaudeReplyData, StoreError> {
        let invalid = |s: &str| StoreError::Invalid(s.into());
        let mut reply = ClaudeReplyData {
            request_id: data.request_id,
            session_id: None,
            messages: vec![],
            committed: false,
        };
        let now = now_unix_ms();
        // Receipts authenticate against retained attribution, not the current
        // instance: delayed ACKs remain valid after removal or replacement.
        match &data.operation {
            ClaudeOperation::Ack { receipts } | ClaudeOperation::Written { receipts } => {
                if receipts.is_empty() || receipts.len() > CLAUDE_BATCH_COUNT {
                    return Err(invalid("invalid receipt count"));
                }
                for r in receipts {
                    let ack = ClaudeAck {
                        message_id: r.message_id.clone(),
                        delivery_id: r.delivery_id.clone(),
                        instance_id: data.instance_id.clone(),
                        session_id: r.session_id.clone(),
                    };
                    if matches!(data.operation, ClaudeOperation::Ack { .. }) {
                        ctx.store.acknowledge_claude(ack, now).await?;
                    } else {
                        let stored = ctx
                            .store
                            .claude_delivery(&r.message_id)
                            .await?
                            .ok_or_else(|| invalid("missing delivery"))?;
                        if stored.instance_id != ack.instance_id
                            || !stored.attempt.as_ref().is_some_and(|a| {
                                a.delivery_id == ack.delivery_id && a.session_id == ack.session_id
                            })
                        {
                            return Err(invalid("receipt attribution mismatch"));
                        }
                        ctx.store
                            .claude_delivery_written(&r.message_id, &r.delivery_id, now)
                            .await?;
                    }
                }
                reply.committed = true;
                return Ok(reply);
            }
            ClaudeOperation::Hook {
                event_id,
                session_id,
                event,
                payload,
                occurred_at_unix_ms,
                replayed,
            } => {
                if !matches!(
                    event.as_str(),
                    "SessionStart"
                        | "UserPromptSubmit"
                        | "PreToolUse"
                        | "PostToolUse"
                        | "Stop"
                        | "SessionEnd"
                ) {
                    return Err(invalid("unsupported native Claude hook"));
                }
                let native: serde_json::Value =
                    serde_json::from_str(payload).map_err(|_| invalid("invalid hook payload"))?;
                if native.get("session_id").and_then(|v| v.as_str()) != Some(session_id.as_str())
                    || native.get("hook_event_name").and_then(|v| v.as_str())
                        != Some(event.as_str())
                {
                    return Err(invalid("hook payload identity mismatch"));
                }
                let added = ctx
                    .store
                    .append_driver_event(
                        NewDriverEvent {
                            id: event_id.clone(),
                            instance_id: data.instance_id.clone(),
                            session_id: session_id.clone(),
                            kind: event.clone(),
                            payload: payload.clone(),
                            occurred_at_unix_ms: *occurred_at_unix_ms,
                            replayed: *replayed,
                        },
                        now,
                    )
                    .await?;
                reply.committed = true;
                // Historical hooks are saved even when their instance is gone.
                if added.inserted
                    && matches!(event.as_str(), "UserPromptSubmit" | "SessionEnd" | "Stop")
                {
                    ctx.store
                        .halt_claude_startup(&data.instance_id, session_id)
                        .await?;
                }
                // Duplicate/replayed/old hooks can never take a queue or make idle.
                if *replayed || !added.inserted || now.abs_diff(*occurred_at_unix_ms) > 5000 {
                    // A helper may die after publishing but before the live
                    // RPC. A newer historical routing observation can only
                    // invalidate cached idle, never create idle or dispatch.
                    if added.inserted
                        && matches!(
                            event.as_str(),
                            "SessionStart" | "UserPromptSubmit" | "SessionEnd" | "Stop"
                        )
                    {
                        let mut states = self.states.lock().await;
                        if let Some(state) = states.get_mut(&data.instance_id)
                            && state.session == *session_id
                            && *occurred_at_unix_ms >= state.last_route_hook
                        {
                            state.idle = None;
                            state.routing_connection = None;
                            state.initial = false;
                            state.ready_generation = None;
                            state.busy = false;
                            state.reported_busy = None;
                            state.last_route_hook = *occurred_at_unix_ms;
                            state.revision = state.revision.wrapping_add(1);
                        }
                    }
                    return Ok(reply);
                }
            }
            _ => {}
        }
        let instance = ctx
            .store
            .instance(&data.instance_id)
            .await?
            .ok_or_else(|| invalid("no Claude instance"))?;
        if instance.backend != Backend::Claude
            || instance.delivery != "push"
            || instance.status != InstanceStatus::Running
            || !instance.session_started
        {
            return Err(invalid("instance is not running Claude push"));
        }
        let session = instance
            .session_id
            .ok_or_else(|| invalid("missing Claude session"))?;
        if !is_uuid_v4(&session) {
            return Err(invalid("invalid Claude session"));
        }
        reply.session_id = Some(session.clone());
        let mut states = self.states.lock().await;
        let state = states.entry(data.instance_id.clone()).or_default();
        if state.session != session {
            *state = State {
                session: session.clone(),
                idle: None,
                last_route_hook: 0,
                revision: 0,
                busy: false,
                reported_busy: None,
                initial: false,
                ready_generation: None,
                idle_connection: None,
                routing_connection: None,
            };
        }
        let route = match data.operation {
            ClaudeOperation::Attach => {
                reply.committed = true;
                return Ok(reply);
            }
            ClaudeOperation::Poll { session_id } => {
                if session_id != session {
                    return Err(invalid("channel belongs to a different session"));
                }
                if state.busy {
                    let revision = state.revision;
                    drop(states);
                    reply.messages = self
                        .interrupt(ctx, &data.instance_id, &session, revision, now)
                        .await?;
                    return Ok(reply);
                }
                if state
                    .idle
                    .is_none_or(|t| t.elapsed() < Duration::from_secs(5))
                {
                    return Ok(reply);
                }
                ClaudeRoute::Channel
            }
            ClaudeOperation::Hook {
                session_id,
                event,
                payload,
                occurred_at_unix_ms,
                ..
            } => {
                if session_id != session {
                    return Ok(reply);
                }
                if matches!(
                    event.as_str(),
                    "SessionStart" | "UserPromptSubmit" | "SessionEnd" | "Stop"
                ) {
                    // A delayed live hook must not undo a newer busy/idle
                    // observation merely because both arrived within 5 s.
                    if occurred_at_unix_ms < state.last_route_hook {
                        return Ok(reply);
                    }
                    state.last_route_hook = occurred_at_unix_ms;
                    state.revision = state.revision.wrapping_add(1);
                }
                match event.as_str() {
                    "UserPromptSubmit" => {
                        state.routing_connection =
                            ctx.runtime.terminal_connection(&data.instance_id).ok();
                        state.initial = false;
                        state.idle = None;
                        state.busy = true;
                        report_state(ctx, &data.instance_id, state, true, now).await?;
                        return Ok(reply);
                    }
                    "PreToolUse" | "PostToolUse" => {
                        // Tool activity revokes a lifecycle idle observation,
                        // even if an earlier prompt hook was lost.
                        state.idle_connection = None;
                        state.routing_connection =
                            ctx.runtime.terminal_connection(&data.instance_id).ok();
                        state.revision = state.revision.wrapping_add(1);
                        return Ok(reply);
                    }
                    "SessionEnd" => {
                        state.routing_connection = None;
                        state.initial = false;
                        state.idle = None;
                        state.busy = false;
                        state.reported_busy = None;
                        return Ok(reply);
                    }
                    "SessionStart" => {
                        state.routing_connection =
                            ctx.runtime.terminal_connection(&data.instance_id).ok();
                        state.busy = false;
                        state.initial = true;
                        state.idle = None;
                        state.ready_generation = None;
                        return Ok(reply);
                    }
                    "Stop" => {
                        state.initial = false;
                        let native: serde_json::Value = serde_json::from_str(&payload)
                            .map_err(|_| invalid("invalid hook payload"))?;
                        match native.get("stop_hook_active").and_then(|v| v.as_bool()) {
                            Some(true) => {
                                // The queued continuation has finished. Do not block it
                                // again, but allow ordinary debounced idle polling.
                                state.busy = false;
                                state.idle = Some(Instant::now());
                                state.idle_connection =
                                    state.routing_connection.clone().filter(|c| c.is_current());
                                return Ok(reply);
                            }
                            None => {
                                state.idle = None;
                                state.busy = true;
                                report_state(ctx, &data.instance_id, state, true, now).await?;
                                return Ok(reply);
                            }
                            Some(false) => {}
                        }
                        state.busy = false;
                        state.idle = Some(Instant::now());
                        state.idle_connection =
                            state.routing_connection.clone().filter(|c| c.is_current());
                        ClaudeRoute::Stop
                    }
                    _ => return Ok(reply),
                }
            }
            _ => return Err(invalid("unknown Claude operation")),
        };
        let revision = state.revision;
        let initial = state.initial;
        let ready_generation = state.ready_generation.clone();
        // Busy hooks must not wait behind a two-second holder snapshot. Check
        // the same state revision again before committing any delivery intent.
        drop(states);
        if initial {
            let ready = startup::live_frame(ctx, &data.instance_id)
                .await
                .filter(|f| ready_generation.as_deref() == Some(f.generation.as_str()))
                .is_some_and(|f| {
                    startup::prompt(&f, &instance.working_directory)
                        == Some(agend_core::screen::claude_startup::Prompt::Ready)
                });
            if !ready {
                let mut states = self.states.lock().await;
                if let Some(state) = states.get_mut(&data.instance_id)
                    && state.revision == revision
                {
                    state.idle = None;
                    state.ready_generation = None;
                }
                return Ok(reply);
            }
        }
        // Live holder screen is required after restart too. Never reconnect
        // a second screen reader over the daemon's current holder link.
        let Some(feed) = ctx.runtime.live_terminal(&data.instance_id) else {
            return Ok(reply);
        };
        let Ok(Ok((screen, _))) = tokio::time::timeout(Duration::from_secs(2), feed).await else {
            return Ok(reply);
        };
        if classify(Backend::Claude, &screen, SCREEN_RULES).is_some() {
            return Ok(reply);
        }
        let mut states = self.states.lock().await;
        let Some(state) = states.get_mut(&data.instance_id) else {
            return Ok(reply);
        };
        if state.session != session
            || state.revision != revision
            || state.idle.is_none()
            || (route == ClaudeRoute::Channel
                && state
                    .idle
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(5)))
        {
            return Ok(reply);
        }
        reply.messages =
            reserve(&ctx.store, &data.instance_id, &session, route, now, false).await?;
        // Startup is complete after the five-second candidate and this live
        // check. Normal idle polls keep the existing hard-gate screen check.
        state.initial = false;
        if !reply.messages.is_empty() {
            state.idle = None;
            state.busy = true;
            state.revision = state.revision.wrapping_add(1);
            report_state(ctx, &data.instance_id, state, true, now).await?;
        } else if route == ClaudeRoute::Channel {
            report_state(ctx, &data.instance_id, state, false, now).await?;
        }
        Ok(reply)
    }

    async fn interrupt(
        &self,
        ctx: &Context,
        instance: &str,
        session: &str,
        revision: u64,
        now: u64,
    ) -> Result<Vec<ClaudePush>, StoreError> {
        use agend_core::protocol::{
            holder::ControlKey,
            terminal::{TerminalControlOperation, TerminalViewport},
        };
        if ctx
            .store
            .pending_claude_interrupts(instance)
            .await?
            .is_empty()
        {
            return Ok(vec![]);
        }
        let Ok(connection) = ctx.runtime.terminal_connection(instance) else {
            return Ok(vec![]);
        };
        let Ok(Ok(frame)) = tokio::time::timeout(Duration::from_secs(2), async {
            // Viewport rows may not exceed the actual PTY height. Read
            // its dimensions before requesting the complete live grid.
            let header = connection
                .frame(TerminalViewport { top: None, rows: 1 })
                .await?;
            connection
                .frame(TerminalViewport {
                    top: None,
                    rows: header.size.rows,
                })
                .await
        })
        .await
        else {
            return Ok(vec![]);
        };
        let screen = frame
            .cells
            .iter()
            .map(|row| row.iter().map(|c| c.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        if classify(Backend::Claude, &screen, SCREEN_RULES).is_some() {
            return Ok(vec![]);
        }
        let mut states = self.states.lock().await;
        let Some(state) = states.get_mut(instance) else {
            return Ok(vec![]);
        };
        if state.session != session || state.revision != revision || !state.busy {
            return Ok(vec![]);
        }
        // One intent protects BOTH the single Esc and the following channel
        // reply. A lost key completion or channel reply can never replay it.
        let pushes = reserve(
            &ctx.store,
            instance,
            session,
            ClaudeRoute::Channel,
            now,
            true,
        )
        .await?;
        if pushes.is_empty() {
            return Ok(pushes);
        }
        state.revision = state.revision.wrapping_add(1);
        let reserved_revision = state.revision;
        drop(states);
        let key = tokio::time::timeout(
            Duration::from_secs(2),
            connection.control(
                frame.generation,
                TerminalControlOperation::DaemonKey {
                    key: ControlKey::Esc,
                    expected_revision: frame.revision,
                },
            ),
        )
        .await;
        if let Ok(Err(error)) = &key
            && matches!(
                error.code.as_str(),
                "control_lost"
                    | "stale_screen"
                    | "unknown_control_key"
                    | "not_supported"
                    | "invalid_request"
                    | "pty_busy"
            )
        {
            ctx.store
                .release_refused_claude_key(
                    &pushes[0].receipt.message_id,
                    &pushes[0].receipt.delivery_id,
                )
                .await?;
            crate::log::line(&format!(
                "{instance}: interrupt refused ({}); message remains queued: {}",
                error.code, pushes[0].receipt.message_id
            ));
            return Ok(vec![]);
        }
        if !matches!(key, Ok(Ok(_))) {
            crate::log::line(&format!(
                "{instance}: interrupt completion unknown; no key or content replay: {}",
                pushes[0].receipt.message_id
            ));
            return Ok(vec![]);
        }
        let states = self.states.lock().await;
        if states
            .get(instance)
            .is_none_or(|s| s.session != session || s.revision != reserved_revision)
        {
            // A newer hook superseded the work state while the key ran. Keep
            // the intent unresolved rather than sending into another turn.
            return Ok(vec![]);
        }
        Ok(pushes)
    }
}

async fn report_state(
    ctx: &Context,
    instance: &str,
    state: &mut State,
    busy: bool,
    now: u64,
) -> Result<(), StoreError> {
    if state.reported_busy == Some(busy) {
        return Ok(());
    }
    ctx.store
        .append_driver_event(
            NewDriverEvent {
                id: crate::store::instances::new_session_id()
                    .map_err(|e| StoreError::Invalid(e.to_string()))?,
                instance_id: instance.into(),
                session_id: state.session.clone(),
                kind: "AgendState".into(),
                payload: serde_json::json!({"busy":busy}).to_string(),
                occurred_at_unix_ms: now,
                replayed: false,
            },
            now,
        )
        .await?;
    state.reported_busy = Some(busy);
    if let Some(mut view) = ctx.fleet.instance(instance)
        && !matches!(view.state, AgentState::Failed | AgentState::Stuck)
    {
        view.state = if busy {
            AgentState::Working
        } else {
            AgentState::Idle
        };
        ctx.fleet.set_instance(
            view,
            format!("{instance}: {}", if busy { "working" } else { "idle" }),
        );
    }
    Ok(())
}

async fn reserve(
    store: &SqliteStore,
    instance: &str,
    session: &str,
    route: ClaudeRoute,
    now: u64,
    interrupt_only: bool,
) -> Result<Vec<ClaudePush>, StoreError> {
    let mut pushes = vec![];
    let mut bytes = 0;
    let messages = if interrupt_only {
        store.pending_claude_interrupts(instance).await?
    } else {
        store
            .pending_claude_messages(instance, route == ClaudeRoute::Stop)
            .await?
    };
    for m in messages {
        // Busy Steer/Interrupt use the correlated holder key path; never
        // silently turn them into a Stop Queue operation.
        if route == ClaudeRoute::Stop && m.level != agend_core::policy::busy::BusyLevel::Queue {
            continue;
        }
        let Some(d) = store.claude_delivery(&m.id).await? else {
            continue;
        };
        if !d.may_start(&m) {
            continue;
        }
        if m.body.len() > MAX_MESSAGE_BYTES
            || m.from_instance.len() > 64
            || m.task_id.as_ref().is_some_and(|t| t.len() > 128)
        {
            return Err(StoreError::Invalid(
                "stored Claude message exceeds transport bounds; nothing truncated".into(),
            ));
        }
        let content = crate::delivery::render(&m.from_instance, m.task_id.as_deref(), &m.body);
        // Headers count too; permit a single legal maximum-sized message.
        let size = content.len() + 512;
        if !pushes.is_empty()
            && (bytes + size > CLAUDE_BATCH_BYTES || pushes.len() == CLAUDE_BATCH_COUNT)
        {
            break;
        }
        let delivery_id = crate::store::instances::new_session_id()
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        let new = NewClaudeDelivery {
            message_id: m.id.clone(),
            delivery_id: delivery_id.clone(),
            instance_id: instance.into(),
            session_id: session.into(),
            route,
        };
        if let ClaudeReservation::Started(_) = store.reserve_claude_delivery(new, now).await? {
            bytes += size;
            pushes.push(ClaudePush {
                receipt: ClaudeReceipt {
                    message_id: m.id,
                    delivery_id,
                    session_id: session.into(),
                },
                content,
            });
            if interrupt_only {
                break;
            }
        }
    }
    Ok(pushes)
}
