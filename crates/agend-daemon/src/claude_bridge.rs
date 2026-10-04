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

#[derive(Default)]
struct State {
    session: String,
    idle: Option<Instant>,
    last_route_hook: u64,
    revision: u64,
}
#[derive(Default)]
pub(crate) struct ClaudeBridge {
    states: Mutex<BTreeMap<String, State>>,
}

impl ClaudeBridge {
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
                    "UserPromptSubmit" | "SessionEnd" => {
                        state.idle = None;
                        return Ok(reply);
                    }
                    "SessionStart" => {
                        state.idle = Some(Instant::now());
                        return Ok(reply);
                    }
                    "Stop" => {
                        let native: serde_json::Value = serde_json::from_str(&payload)
                            .map_err(|_| invalid("invalid hook payload"))?;
                        if native.get("stop_hook_active").and_then(|v| v.as_bool()) != Some(false) {
                            state.idle = None;
                            return Ok(reply);
                        }
                        state.idle = Some(Instant::now());
                        ClaudeRoute::Stop
                    }
                    _ => return Ok(reply),
                }
            }
            _ => return Err(invalid("unknown Claude operation")),
        };
        let revision = state.revision;
        // Busy hooks must not wait behind a two-second holder snapshot. Check
        // the same state revision again before committing any delivery intent.
        drop(states);
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
        reply.messages = reserve(&ctx.store, &data.instance_id, &session, route, now).await?;
        if !reply.messages.is_empty() {
            state.idle = None;
            state.revision = state.revision.wrapping_add(1);
        }
        Ok(reply)
    }
}

async fn reserve(
    store: &SqliteStore,
    instance: &str,
    session: &str,
    route: ClaudeRoute,
    now: u64,
) -> Result<Vec<ClaudePush>, StoreError> {
    let mut pushes = vec![];
    let mut bytes = 0;
    for m in store
        .pending_claude_messages(instance, route == ClaudeRoute::Stop)
        .await?
    {
        // Busy Steer/Interrupt need the future holder-control driver; never
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
        }
    }
    Ok(pushes)
}
