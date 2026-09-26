//! Command handlers, separate from transport (D7). Caller identity -> DB
//! binding gives task, branch, repo, PR, expected head; agents pass intent
//! only. Permissions depend on caller identity (agent vs operator commands,
//! D17); a rejection includes the correct command to run.
//!
//! Gate 8 serves the client protocol requests that have real data (P6):
//! `get_fleet`, `subscribe_events`, `subscribe_terminal` and
//! `resolve_attention` (operator only: any `caller` in `hello` is an agent,
//! P2; identity is checked before the item is looked up). `terminal_input`
//! and agent commands answer `not_supported`, `answer_ask` answers
//! `unknown_ask` (no asks before gates 9 and 10); none of them changes
//! anything.
//!
//! Must NOT: read the caller's cwd to infer context, or know which transport
//! (socket, future MCP adapter) carried the call.

use std::sync::Arc;
use std::time::Duration;

use agend_core::protocol::client::{
    AgentCommand, AgentState, ClientCommandResultData, ClientRequest, ClientResponse,
    CommandResult, ErrorData, FleetData, TerminalSnapshotData, error_code,
};
use tokio::sync::{broadcast, mpsc::UnboundedSender, oneshot};

use crate::fleet::{Fleet, Subscription};
use crate::runtime::HolderRuntime;
use crate::supervisor::Event;

/// What an agent gets for `resolve_attention`.
pub const OPERATOR_ONLY: &str =
    "only the operator can resolve needs-you items; ask the operator with agend ask";
/// Longest wait for a holder's answer to `Snapshot`.
const SNAPSHOT_WITHIN: Duration = Duration::from_secs(5);

/// What the handlers work with.
pub struct Context {
    pub fleet: Arc<Fleet>,
    pub runtime: HolderRuntime,
    /// The supervisor's queue (`resolve_attention` acts through it).
    pub supervisor: UnboundedSender<Event>,
}

/// What the server does with a request.
pub enum Outcome {
    Reply(ClientResponse),
    /// Send the backlog, then forward live events.
    Events(Subscription),
    /// Send the screen, then forward the PTY chunks (`None`: the screen
    /// only, of a `failed` instance).
    Terminal {
        snapshot: ClientResponse,
        live: Option<broadcast::Receiver<String>>,
    },
}

pub fn error(request_id: Option<String>, code: &str, message: impl Into<String>) -> ClientResponse {
    ClientResponse::Error {
        data: ErrorData {
            request_id,
            code: code.to_owned(),
            message: message.into(),
        },
    }
}

/// `agend review approve` for `review_approve`, from the command's wire tag.
fn command_name(command: &AgentCommand) -> String {
    let tag = serde_json::to_value(command)
        .ok()
        .and_then(|v| v.get("command")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into());
    format!("agend {}", tag.replace('_', " "))
}

/// Handles one request after `hello`; `caller` is the hello's.
pub async fn handle(ctx: &Context, caller: Option<&str>, request: ClientRequest) -> Outcome {
    let reply = match request {
        ClientRequest::Hello { .. } => error(
            None,
            error_code::INVALID_REQUEST,
            "hello was already negotiated",
        ),
        ClientRequest::GetFleet { data } => ClientResponse::Fleet {
            data: FleetData {
                request_id: data.request_id,
                fleet: ctx.fleet.view(),
            },
        },
        ClientRequest::SubscribeEvents { data } => match ctx.fleet.subscribe(data.after_event_id) {
            Ok(subscription) => return Outcome::Events(subscription),
            Err(message) => error(None, error_code::EVENT_GAP, message),
        },
        ClientRequest::SubscribeTerminal { data } => {
            return terminal(ctx, data.instance_id).await;
        }
        ClientRequest::TerminalInput { .. } => error(
            None,
            error_code::NOT_SUPPORTED,
            "terminal input arrives with the attach view (gate 11); nothing was written",
        ),
        ClientRequest::AnswerAsk { data } => error(
            Some(data.request_id),
            error_code::UNKNOWN_ASK,
            format!(
                "no open ask {} (asks arrive in gates 9 and 10)",
                data.ask_id
            ),
        ),
        ClientRequest::Command { data } => error(
            Some(data.request_id),
            error_code::NOT_SUPPORTED,
            format!(
                "agent commands arrive in gate 9 ({})",
                command_name(&data.command)
            ),
        ),
        ClientRequest::ResolveAttention { data } => {
            if caller.is_some() {
                return Outcome::Reply(error(
                    Some(data.request_id),
                    error_code::FORBIDDEN,
                    OPERATOR_ONLY,
                ));
            }
            let (reply, answer) = oneshot::channel();
            let sent = ctx.supervisor.send(Event::Resolve {
                attention_id: data.attention_id,
                action: data.action,
                reply,
            });
            match (sent, answer.await) {
                (Ok(()), Ok(Ok(()))) => ClientResponse::CommandResult {
                    data: ClientCommandResultData {
                        request_id: data.request_id,
                        result: CommandResult::Accepted,
                    },
                },
                (Ok(()), Ok(Err(refusal))) => {
                    error(Some(data.request_id), refusal.code, refusal.message)
                }
                _ => error(
                    Some(data.request_id),
                    error_code::NOT_SUPPORTED,
                    "the daemon is stopping; nothing changed",
                ),
            }
        }
        ClientRequest::Unknown => error(None, error_code::UNKNOWN_REQUEST, "unknown request type"),
    };
    Outcome::Reply(reply)
}

/// `subscribe_terminal`: a running instance's screen and bytes through the
/// daemon's link; a `failed` one's last screen only (its holder, if still
/// there); otherwise `no_terminal`.
async fn terminal(ctx: &Context, instance_id: String) -> Outcome {
    let no_terminal =
        |message: String| Outcome::Reply(error(None, error_code::NO_TERMINAL, message));
    let snapshot = |instance_id: &str, screen: String| ClientResponse::TerminalSnapshot {
        data: TerminalSnapshotData {
            instance_id: instance_id.to_owned(),
            screen,
        },
    };
    let Some(view) = ctx.fleet.instance(&instance_id) else {
        return no_terminal(format!("no instance {instance_id}"));
    };
    if view.state == AgentState::Failed {
        return match ctx.runtime.last_screen(&instance_id).await {
            Ok(Some(screen)) => Outcome::Terminal {
                snapshot: snapshot(&instance_id, screen),
                live: None,
            },
            Ok(None) => no_terminal(format!("{instance_id} failed and its holder is gone")),
            Err(e) => no_terminal(format!("{instance_id} failed; its last screen: {e}")),
        };
    }
    let Some(feed) = ctx.runtime.live_terminal(&instance_id) else {
        return no_terminal(format!(
            "{instance_id} has no terminal now ({})",
            view.state.as_str()
        ));
    };
    match tokio::time::timeout(SNAPSHOT_WITHIN, feed).await {
        Ok(Ok((screen, live))) => Outcome::Terminal {
            snapshot: snapshot(&instance_id, screen),
            live: Some(live),
        },
        _ => no_terminal(format!("{instance_id}: its holder did not send its screen")),
    }
}
