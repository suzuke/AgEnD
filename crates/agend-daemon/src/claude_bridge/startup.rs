//! Recorded startup menus only; the holder arbitrates ownership and revision
//! at the actual PTY write. This task never waits while holding routing state.
use super::ClaudeBridge;
use crate::{handlers::Context, runtime::terminal::TerminalConnection};
use agend_core::{
    model::Backend,
    protocol::terminal::{
        TerminalControlOperation, TerminalFrame, TerminalNotice, TerminalViewport,
    },
    runtime_records::{ClaudeStartupKey, InstanceStatus},
    screen::claude_startup::{self, Prompt},
};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) async fn frame(connection: &TerminalConnection) -> Option<TerminalFrame> {
    tokio::time::timeout(Duration::from_secs(2), async {
        // Every recorded startup rule has exactly 24 rows. Request that
        // complete frame once, rather than sampling the shared grid twice.
        // A shorter operator viewport is refused without capturing the grid.
        let frame = connection
            .frame(TerminalViewport {
                top: None,
                rows: 24,
            })
            .await
            .ok()?;
        (frame.size.rows == 24
            && matches!(frame.size.columns, 100 | 140)
            && !frame.alternate_screen
            && frame.viewport_top == frame.live_top
            && !frame.viewport_clamped
            && frame.cells.len() == usize::from(frame.size.rows)
            && frame
                .cells
                .iter()
                .all(|r| r.len() == usize::from(frame.size.columns)))
        .then_some(frame)
    })
    .await
    .ok()
    .flatten()
}
pub(crate) async fn live_frame(ctx: &Context, instance: &str) -> Option<TerminalFrame> {
    frame(&ctx.runtime.terminal_connection(instance).ok()?).await
}
pub(crate) fn prompt(frame: &TerminalFrame, workspace: &str) -> Option<Prompt> {
    let workspace = std::path::Path::new(workspace).canonicalize().ok()?;
    let workspace = workspace.to_str()?;
    let text = frame
        .cells
        .iter()
        .map(|row| {
            row.iter()
                .filter(|c| !c.leading_spacer)
                .map(|c| c.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    claude_startup::classify(&text, workspace, frame.size.columns, frame.size.rows)
}

pub(crate) async fn run(
    ctx: Arc<Context>,
    bridge: Arc<ClaudeBridge>,
    replies: Arc<crate::delivery::Replies>,
) {
    // This observation cache is never key recovery state. Reconnecting starts
    // a fresh stability window; SQLite alone decides whether a key can run.
    let mut stable = BTreeMap::<String, (String, String, u64, Prompt, Instant)>::new();
    // Unknown screens never authorize a key or idle. Avoid serializing the
    // same unrecognized grid repeatedly while an operator is viewing it.
    // Output/link changes trigger the next observation; a one-second refresh
    // also catches changes without PTY output, such as an operator resize.
    let mut unknown = BTreeMap::<String, (String, TerminalNotice, Instant)>::new();
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let Ok(instances) = ctx.store.instances().await else {
            continue;
        };
        stable.retain(|id, _| instances.iter().any(|i| &i.id == id));
        unknown.retain(|id, _| instances.iter().any(|i| &i.id == id));
        for instance in instances {
            if instance.backend != Backend::Claude
                || instance.delivery != "push"
                || instance.status != InstanceStatus::Running
                || !instance.session_started
            {
                continue;
            }
            let Some(session) = instance.session_id.as_deref() else {
                continue;
            };
            let initial = {
                let states = bridge.states.lock().await;
                states
                    .get(&instance.id)
                    .filter(|s| s.session == session && s.initial && !s.busy)
                    .map(|s| s.revision)
            };
            let startup = ctx
                .store
                .claude_startup(&instance.id)
                .await
                .ok()
                .flatten()
                .filter(|s| s.session == session && !s.halted);
            if startup.is_none() && initial.is_none() {
                stable.remove(&instance.id);
                unknown.remove(&instance.id);
                continue;
            }
            let connection = ctx.runtime.terminal_connection(&instance.id).ok();
            let observation = connection
                .as_ref()
                .map(|c| (*c.notices().borrow(), Instant::now()));
            let refresh = observation.is_none_or(|(notice, _)| {
                unknown
                    .get(&instance.id)
                    .is_none_or(|(old_session, old, at)| {
                        old_session != session
                            || old != &notice
                            || at.elapsed() >= Duration::from_secs(1)
                    })
            });
            let frame = match (&connection, refresh) {
                (Some(c), true) => frame(c).await,
                _ => None,
            };
            let known = frame
                .as_ref()
                .and_then(|f| prompt(f, &instance.working_directory));
            if known.is_some() {
                unknown.remove(&instance.id);
            } else if refresh && let Some((notice, at)) = observation {
                unknown.insert(instance.id.clone(), (session.into(), notice, at));
            }
            if let Some(revision) = initial {
                let mut states = bridge.states.lock().await;
                if let Some(state) = states.get_mut(&instance.id)
                    && state.session == session
                    && state.revision == revision
                    && state.initial
                    && !state.busy
                {
                    if known == Some(Prompt::Ready) {
                        let generation = frame.as_ref().unwrap().generation.clone();
                        if state.ready_generation.as_ref() != Some(&generation)
                            || state
                                .idle_connection
                                .as_ref()
                                .is_none_or(|c| !c.is_current())
                        {
                            state.idle = Some(Instant::now());
                            state.ready_generation = Some(generation);
                            state.idle_connection = connection.clone();
                        }
                    } else {
                        state.idle = None;
                        state.ready_generation = None;
                    }
                }
            }
            let (Some(startup), Some(frame), Some(connection), Some(known)) =
                (startup, frame, connection, known)
            else {
                stable.remove(&instance.id);
                continue;
            };
            if known == Prompt::Ready {
                let _ = ctx.store.halt_claude_startup(&instance.id, session).await;
                stable.remove(&instance.id);
                continue;
            }
            if startup
                .generation
                .as_ref()
                .is_some_and(|g| g != &frame.generation)
            {
                stable.remove(&instance.id);
                continue;
            }
            let observation = stable.entry(instance.id.clone()).or_insert_with(|| {
                (
                    startup.launch.clone(),
                    frame.generation.clone(),
                    frame.revision,
                    known,
                    Instant::now(),
                )
            });
            if observation.0 != startup.launch
                || observation.1 != frame.generation
                || observation.2 != frame.revision
                || observation.3 != known
            {
                *observation = (
                    startup.launch.clone(),
                    frame.generation.clone(),
                    frame.revision,
                    known,
                    Instant::now(),
                );
                continue;
            }
            if observation.4.elapsed() < Duration::from_secs(1) {
                continue;
            }
            let Ok(attempt) = crate::store::instances::new_session_id() else {
                continue;
            };
            let intent = ClaudeStartupKey {
                startup,
                generation: frame.generation.clone(),
                prompt: known.name().into(),
                attempt,
            };
            // Register before the atomic pause/reservation check, so prepare
            // either refuses this key or waits for its native write to finish.
            let _write = replies.begin(&instance.id);
            if !ctx
                .store
                .reserve_claude_startup_key(intent.clone())
                .await
                .unwrap_or(false)
            {
                continue;
            }
            // Hooks can revoke this startup while the frame was being read.
            if !ctx
                .store
                .claude_startup(&instance.id)
                .await
                .ok()
                .flatten()
                .is_some_and(|s| {
                    !s.halted && s.launch == intent.startup.launch && s.session == session
                })
            {
                continue;
            }
            let result = tokio::time::timeout(
                Duration::from_secs(2),
                connection.control(
                    frame.generation.clone(),
                    TerminalControlOperation::DaemonKey {
                        key: known.key().unwrap(),
                        expected_revision: frame.revision,
                    },
                ),
            )
            .await;
            let written = matches!(&result,Ok(Ok(data)) if data.generation==frame.generation && data.attach_id.is_none());
            let refused = matches!(&result,Ok(Err(e)) if matches!(e.code.as_str(),"control_lost"|"stale_screen"|"not_supported"|"unknown_control_key"|"invalid_request"|"pty_busy"));
            if written || refused {
                let _ = ctx.store.finish_claude_startup_key(intent, written).await;
            }
            stable.remove(&instance.id);
        }
    }
}
