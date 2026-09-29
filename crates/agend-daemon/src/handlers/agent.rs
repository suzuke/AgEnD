//! Agent commands (`command`, gate 9 P1): only callers that named
//! themselves in `hello` (`AGEND_INSTANCE`); the operator gets `forbidden`
//! with what to do instead.
//!
//! - `status`: the caller's instance; no task before gate 10.
//! - `send`: `deliver` of the codex driver (gate 7), which claims the id in
//!   the `messages` table first: a message id the client chose must be a
//!   UUID v4; the same id with the same content is accepted again without a
//!   second delivery, with other content it is `invalid_request`. The reply
//!   comes after the row is written.
//! - `inbox`: the caller's messages by `seq`: the last 20, or every one after
//!   `after_message_id` (`unknown_message` when the caller has no such
//!   message).
//! - Everything else (`done`, `result`, `review …`, `ask …`, `block`,
//!   `unblock`, `remind`, `task create`): `not_supported` until gate 10;
//!   nothing changes.
//!
//! Must NOT: change anything for a refused command.

use agend_core::policy::busy::BusyLevel;
use agend_core::protocol::client::{
    AgentCommand, ClientCommandData, ClientCommandResultData, ClientResponse, CommandResult,
    InboxMessage, MAX_MESSAGE_BYTES, MessageLevel, MessagesData, StatusData, error_code,
    is_uuid_v4, message_too_long as too_long,
};
use agend_core::traits::{AgentMessage, Driver};

use super::{Context, command_name, error};
use crate::driver::codex::DriverError;
use crate::store::Message;
use crate::store::instances::new_session_id;

/// Messages `agend inbox` shows without `--after`.
pub const INBOX_LAST: usize = 20;

type Refusal = (&'static str, String);

fn not_an_instance(caller: &str) -> Refusal {
    (
        error_code::UNKNOWN_INSTANCE,
        format!(
            "no instance {caller}: AGEND_INSTANCE names no instance of this daemon (the operator sees them with agend instance list)"
        ),
    )
}

/// Handles `command` from agent `caller`.
pub async fn handle(ctx: &Context, caller: &str, data: ClientCommandData) -> ClientResponse {
    let request_id = data.request_id;
    let result = match data.command {
        AgentCommand::Status => status(ctx, caller).await,
        AgentCommand::Send {
            to,
            message,
            level,
            message_id,
        } => send(ctx, caller, to, message, level, message_id).await,
        AgentCommand::Inbox { after_message_id } => inbox(ctx, caller, after_message_id).await,
        AgentCommand::Unknown => Err((error_code::UNKNOWN_REQUEST, "unknown agent command".into())),
        other => Err((
            error_code::NOT_SUPPORTED,
            format!(
                "{} arrives in gate 10; nothing changed",
                command_name(&other)
            ),
        )),
    };
    match result {
        Ok(result) => ClientResponse::CommandResult {
            data: ClientCommandResultData { request_id, result },
        },
        Err((code, message)) => error(Some(request_id), code, message),
    }
}

async fn status(ctx: &Context, caller: &str) -> Result<CommandResult, Refusal> {
    let instance = ctx
        .store
        .instance(caller)
        .await
        .map_err(|e| {
            (
                error_code::INVALID_REQUEST,
                format!("cannot read {caller}: {e}"),
            )
        })?
        .ok_or_else(|| not_an_instance(caller))?;
    Ok(CommandResult::Status {
        data: StatusData {
            task_id: None,
            instance_id: Some(instance.id.clone()),
            summary: format!(
                "{} ({}): no task\nnext: agend inbox | agend send <name> \"<message>\"",
                instance.id,
                instance.backend.as_str()
            ),
            identity: None,
        },
    })
}

async fn send(
    ctx: &Context,
    caller: &str,
    to: String,
    body: String,
    level: Option<MessageLevel>,
    message_id: Option<String>,
) -> Result<CommandResult, Refusal> {
    if body.len() > MAX_MESSAGE_BYTES {
        return Err((error_code::INVALID_REQUEST, too_long(body.len())));
    }
    let level = match level.unwrap_or(MessageLevel::Queue) {
        MessageLevel::Queue => BusyLevel::Queue,
        MessageLevel::Steer => BusyLevel::Steer,
        MessageLevel::Interrupt => BusyLevel::Interrupt,
        MessageLevel::Unknown => {
            return Err((
                error_code::INVALID_REQUEST,
                "unknown level; use queue, steer or interrupt".into(),
            ));
        }
    };
    let id = match message_id {
        Some(id) if is_uuid_v4(&id) => id,
        Some(id) => {
            return Err((
                error_code::INVALID_REQUEST,
                format!("message id {id} is not a UUID v4"),
            ));
        }
        None => new_session_id().map_err(|e| {
            (
                error_code::INVALID_REQUEST,
                format!("cannot make a message id: {e}"),
            )
        })?,
    };
    let read = |e| {
        (
            error_code::INVALID_REQUEST,
            format!("cannot read instances: {e}"),
        )
    };
    if ctx.store.instance(caller).await.map_err(read)?.is_none() {
        return Err(not_an_instance(caller));
    }
    let message = AgentMessage {
        id: id.clone(),
        from: caller.to_owned(),
        task_id: None,
        body,
    };
    match ctx.codex.deliver(&to, &message, level).await {
        Ok(_) => Ok(CommandResult::Accepted),
        Err(DriverError::UnknownInstance(_)) => Err((
            error_code::UNKNOWN_INSTANCE,
            format!("no instance {to}; the names are in agend status"),
        )),
        Err(DriverError::InvalidRequest(_)) => Err((
            error_code::INVALID_REQUEST,
            format!("message id {id} is already used by another message"),
        )),
        Err(e) => Err((
            error_code::INVALID_REQUEST,
            format!("cannot deliver to {to}: {e}"),
        )),
    }
}

fn shown(message: Message) -> InboxMessage {
    InboxMessage {
        message_id: message.id,
        from: message.from_instance,
        body: message.body,
        created_at_unix_ms: message.created_at_unix_ms,
    }
}

async fn inbox(
    ctx: &Context,
    caller: &str,
    after: Option<String>,
) -> Result<CommandResult, Refusal> {
    let read = |e| {
        (
            error_code::INVALID_REQUEST,
            format!("cannot read messages: {e}"),
        )
    };
    let messages = match after {
        None => {
            let mut all = ctx.store.messages_to(caller).await.map_err(read)?;
            let skip = all.len().saturating_sub(INBOX_LAST);
            all.drain(..skip);
            all
        }
        Some(after) => ctx
            .store
            .messages_after(caller, &after)
            .await
            .map_err(read)?
            .ok_or_else(|| {
                (
                    error_code::UNKNOWN_MESSAGE,
                    format!(
                        "you have no message {after} (unknown, older than 30 days, or not yours); run agend inbox without --after"
                    ),
                )
            })?,
    };
    Ok(CommandResult::Messages {
        data: MessagesData {
            messages: messages.into_iter().map(shown).collect(),
        },
    })
}
