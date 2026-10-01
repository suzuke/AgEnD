//! Command handlers, separate from transport (D7). Caller identity -> DB
//! binding gives task, branch, repo, PR, expected head; agents pass intent
//! only. Permissions depend on caller identity (agent vs operator commands,
//! D17); a rejection includes the correct command to run.
//!
//! Gate 8 serves the client protocol requests that have real data (P6):
//! `get_fleet`, `subscribe_events`, `subscribe_terminal` and
//! `resolve_attention` (operator only: any `caller` in `hello` is an agent,
//! P2; identity is checked before the item is looked up). `answer_ask`
//! answers `unknown_ask` (no asks before gate 10) and changes nothing.
//!
//! Gate 11 B (P6): `terminal_input` is the operator's only: an agent gets
//! `forbidden` (identity first), an instance without a live terminal
//! `no_terminal`, a codex instance `not_supported` (until U17 is verified,
//! gate 7 P1); otherwise the bytes go to the holder and nothing is
//! answered. Its errors carry no request id.
//!
//! Gate 9 (P1): permissions are checked here only. `command` (agent
//! commands, [`agent`]) is for agents: the operator gets `forbidden`;
//! `operator` ([`operator`]) is for the operator: an agent gets
//! `forbidden`. Read-only requests are open to both.
//!
//! Must NOT: read the caller's cwd to infer context, or know which transport
//! (socket, future MCP adapter) carried the call.

pub mod agent;
pub mod operator;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::client::{
    AgentCommand, AgentState, ClientCommandResultData, ClientRequest, ClientResponse,
    CommandResult, ErrorData, FleetData, SelectedVersionData, TerminalSnapshotData, error_code,
};
use agend_core::protocol::holder::{MAX_REQUEST_LINE, operator_input_too_long};
use tokio::sync::{broadcast, mpsc::UnboundedSender};

use crate::driver::codex::CodexDriver;
use crate::fleet::{Fleet, Subscription};
use crate::runtime::HolderRuntime;
use crate::store::SqliteStore;
use crate::supervisor::Event;

/// What an agent gets for `resolve_attention`.
pub const OPERATOR_ONLY: &str = "only the operator can resolve needs-you items; ask the operator";
/// What an agent gets for `terminal_input`.
pub const TYPE_OPERATOR_ONLY: &str = "only the operator can type into an agent's terminal";
/// What `terminal_input` into a codex instance gets.
pub const CODEX_INPUT: &str =
    "typing into a codex terminal waits until U17 is verified (gate 7 P1); nothing was written";
/// Longest wait for a holder's answer to `Snapshot`.
const SNAPSHOT_WITHIN: Duration = Duration::from_secs(5);

/// What the handlers work with.
pub struct Context {
    pub fleet: Arc<Fleet>,
    pub pipeline: crate::pipeline::Handle,
    pub runtime: HolderRuntime,
    /// The supervisor's queue (`resolve_attention`, `instance_add` and
    /// `instance_remove` act through it).
    pub supervisor: UnboundedSender<Event>,
    pub store: Arc<SqliteStore>,
    /// `send` delivers through it (gate 7's `deliver`).
    pub codex: CodexDriver,
    /// This daemon's own binary (`daemon_restart` without a binary).
    pub exe: PathBuf,
    /// A restart preflight is running (only one at a time, gate 9 P7).
    pub restarting: AtomicBool,
}

impl Context {
    /// The `hello` reply for `selected`: with the daemon's version, pid
    /// and boot id (gate 9 P6, P7).
    pub fn hello(&self, selected: ProtocolVersion) -> SelectedVersionData {
        SelectedVersionData {
            selected,
            daemon_version: Some(format!("agend {}", env!("CARGO_PKG_VERSION"))),
            daemon_pid: Some(std::process::id()),
            boot_id: Some(self.fleet.base()),
        }
    }
}

/// What the server does with a request.
pub enum Outcome {
    Reply(ClientResponse),
    /// Nothing to answer (accepted `terminal_input`).
    Nothing,
    /// Send the backlog, then forward live events.
    Events(Subscription),
    /// Send the screen, then forward the PTY chunks (`None`: the screen
    /// only, of a `failed` instance).
    Terminal {
        snapshot: ClientResponse,
        live: Option<broadcast::Receiver<String>>,
    },
    /// Send `reply` (`restarting`), then hand `binary` to the supervisor,
    /// which stops the daemon so it can `exec` it.
    Restart {
        reply: ClientResponse,
        binary: PathBuf,
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

/// What the operator gets for an agent command.
pub(crate) fn pipeline_reply(request_id: String, result: crate::pipeline::Reply) -> ClientResponse {
    match result {
        Ok(result) => ClientResponse::CommandResult {
            data: ClientCommandResultData { request_id, result },
        },
        Err((code, message)) => error(Some(request_id), &code, message),
    }
}

fn agent_only(command: &AgentCommand) -> String {
    format!(
        "{} is an agent command; it runs inside an agent, where AGEND_INSTANCE is set",
        command_name(command)
    )
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
        ClientRequest::TerminalInput { data } => {
            return terminal_input(ctx, caller, data.instance_id, data.bytes_base64);
        }
        ClientRequest::AnswerAsk { data } => {
            if caller.is_some() {
                error(
                    Some(data.request_id),
                    error_code::FORBIDDEN,
                    "only the operator can answer asks",
                )
            } else {
                let id = data.request_id.clone();
                pipeline_reply(id, ctx.pipeline.answer(data).await)
            }
        }
        ClientRequest::Command { data } => match caller {
            Some(caller) => agent::handle(ctx, caller, data).await,
            None => error(
                Some(data.request_id),
                error_code::FORBIDDEN,
                agent_only(&data.command),
            ),
        },
        ClientRequest::Operator { data } => {
            if caller.is_some() {
                let message = operator::forbidden(&data.command);
                return Outcome::Reply(error(
                    Some(data.request_id),
                    error_code::FORBIDDEN,
                    message,
                ));
            }
            return operator::handle(ctx, data).await;
        }
        ClientRequest::ResolveAttention { data } => {
            if caller.is_some() {
                return Outcome::Reply(error(
                    Some(data.request_id),
                    error_code::FORBIDDEN,
                    OPERATOR_ONLY,
                ));
            }
            if !data.attention_id.starts_with("instance-failed:") {
                let id = data.request_id.clone();
                return Outcome::Reply(pipeline_reply(id, ctx.pipeline.resolve(data).await));
            }
            // Checked and taken off the list here, so the reply never waits
            // for a busy supervisor (a retry can take 15 s, C6); the
            // supervisor does the retry afterwards.
            let Some(item) = ctx.fleet.resolve(&data.attention_id, data.action) else {
                return Outcome::Reply(error(
                    Some(data.request_id),
                    error_code::UNKNOWN_ATTENTION,
                    format!(
                        "no needs-you item {} with action {}",
                        data.attention_id,
                        data.action.as_str()
                    ),
                ));
            };
            if let Err(unsent) = ctx.supervisor.send(Event::Retry {
                item: Box::new(item),
            }) {
                let Event::Retry { item } = unsent.0 else {
                    unreachable!("sent a retry")
                };
                ctx.fleet.raise(*item);
                return Outcome::Reply(error(
                    Some(data.request_id),
                    error_code::NOT_SUPPORTED,
                    "the daemon is stopping; nothing changed",
                ));
            }
            ClientResponse::CommandResult {
                data: ClientCommandResultData {
                    request_id: data.request_id,
                    result: CommandResult::Accepted,
                },
            }
        }
        ClientRequest::Unknown => error(None, error_code::UNKNOWN_REQUEST, "unknown request type"),
    };
    Outcome::Reply(reply)
}

/// `terminal_input` (gate 11 B P6): identity, then a live terminal, then the
/// backend; accepted input is not answered.
fn terminal_input(
    ctx: &Context,
    caller: Option<&str>,
    instance_id: String,
    bytes_base64: String,
) -> Outcome {
    let refuse = |code: &str, message: String| Outcome::Reply(error(None, code, message));
    if caller.is_some() {
        return refuse(error_code::FORBIDDEN, TYPE_OPERATOR_ONLY.into());
    }
    let live = ctx
        .fleet
        .instance(&instance_id)
        .filter(|view| view.state != AgentState::Failed && ctx.runtime.has_link(&instance_id));
    let Some(view) = live else {
        return refuse(
            error_code::NO_TERMINAL,
            format!("{instance_id} has no live terminal; nothing was written"),
        );
    };
    if view.backend == agend_core::model::Backend::Codex.as_str() {
        return refuse(error_code::NOT_SUPPORTED, CODEX_INPUT.into());
    }
    let line = crate::runtime::link::input_line(bytes_base64);
    if line.len() > MAX_REQUEST_LINE {
        return refuse(
            error_code::INVALID_REQUEST,
            operator_input_too_long(line.len()),
        );
    }
    if !ctx.runtime.terminal_input(&instance_id, line) {
        return refuse(
            error_code::NO_TERMINAL,
            format!("{instance_id} has no live terminal; nothing was written"),
        );
    }
    Outcome::Nothing
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
