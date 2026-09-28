//! Agent commands (D17, gate 9 P1, P2): they run inside an agent, where
//! `AGEND_INSTANCE` names the caller; the daemon refuses them from the
//! operator. Result commands carry a ticket `<task>/<stage>/<attempt>`
//! (from `agend status` or the assignment), split here into `task_id` and
//! `identity`; `block`, `unblock` and `remind` ask `status` for the task
//! first (one agent, one task: D33).
//!
//! Resent after a restart (P5): `status`, `inbox`, and `send` with the same
//! message id (a UUID v4 made here). Never resent: the rest.
//!
//! Must NOT: accept context the daemon can derive (branch, repo, PR, head).

use std::io::Read;
use std::time::Duration;

use agend_client::Redo;
use agend_core::protocol::client::{
    AgentCommand, ClientCommandData, ClientRequest, ClientResponse, CommandResult, MessageLevel,
    Ticket, uuid_v4,
};
use clap::{Subcommand, ValueEnum};

use super::{Failure, Output, Target, retried, to_json};

/// What to check after a request that changes something was cut off.
const CHECK: &str = "agend status";
/// How long `send` waits for its reply: the daemon answers once the message
/// is in the DB and, for codex, handed to its link (gate 7: at most 60 s).
const SEND_REPLY_WITHIN: Duration = Duration::from_secs(70);

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Level {
    Queue,
    Steer,
    Interrupt,
}

#[derive(Subcommand)]
pub enum Review {
    /// Approve the change under review
    #[command(before_help = "Example: agend review approve t-42/review/2")]
    Approve { ticket: String },
    /// Ask for changes
    #[command(
        before_help = "Example: agend review changes t-42/review/2 \"rename the flag to --force\""
    )]
    Changes { ticket: String, what: String },
}

/// Sends `command` and returns the result.
fn command(target: &Target, command: AgentCommand, redo: Redo) -> Result<CommandResult, Failure> {
    let mut client = target.connect(CHECK)?;
    command_on(&mut client, command, redo)
}

fn command_on(
    client: &mut agend_client::Client,
    command: AgentCommand,
    redo: Redo,
) -> Result<CommandResult, Failure> {
    command_within(
        client,
        command,
        redo,
        agend_client::connection::REPLY_WITHIN,
    )
}

fn command_within(
    client: &mut agend_client::Client,
    command: AgentCommand,
    redo: Redo,
    within: Duration,
) -> Result<CommandResult, Failure> {
    let request = ClientRequest::Command {
        data: ClientCommandData {
            request_id: client.next_request_id(),
            command,
        },
    };
    match client.request_within(&request, redo, within) {
        Ok(ClientResponse::CommandResult { data }) => Ok(data.result),
        Ok(other) => Err(Failure::new(
            "disconnected",
            format!("unexpected reply: {other:?}"),
        )),
        Err(e) => Err(Failure::client(e, CHECK)),
    }
}

/// `accepted` (or what else the daemon answered) for people and `--json`.
fn shown(result: CommandResult) -> Output {
    let line = match &result {
        CommandResult::Accepted => "accepted".to_owned(),
        CommandResult::TaskCreated { data } => format!("created {}", data.task_id),
        CommandResult::AskCreated { data } => format!("asked {}", data.ask_id),
        other => format!("{other:?}"),
    };
    Output::new(vec![line], to_json(&result))
}

fn ticket(text: &str) -> Result<Ticket, Failure> {
    Ticket::parse(text).ok_or_else(|| {
        Failure::usage(format!(
            "invalid ticket {text:?}: use <task>/<stage>/<attempt> from agend status, e.g. t-42/review/2"
        ))
    })
}

pub fn status(target: &Target) -> Result<Output, Failure> {
    let result = command(target, AgentCommand::Status, Redo::Safe)?;
    let CommandResult::Status { data } = &result else {
        return Ok(shown(result));
    };
    let mut lines: Vec<String> = data.summary.lines().map(str::to_owned).collect();
    if let (Some(task), Some(identity)) = (&data.task_id, &data.identity) {
        let ticket = Ticket {
            task_id: task.clone(),
            identity: identity.clone(),
        };
        lines.push(format!(
            "{task} · {} (attempt {}) · ticket {ticket}",
            identity.stage_id, identity.attempt
        ));
    }
    Ok(Output::new(lines, to_json(&result)))
}

/// A new message id: a UUID v4 from `/dev/urandom`.
fn message_id() -> Result<String, Failure> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| Failure::new("usage", format!("cannot read /dev/urandom: {e}")))?;
    Ok(uuid_v4(bytes))
}

pub fn send(target: &Target, to: String, message: String, level: Level) -> Result<Output, Failure> {
    let id = message_id()?;
    let level = match level {
        Level::Queue => MessageLevel::Queue,
        Level::Steer => MessageLevel::Steer,
        Level::Interrupt => MessageLevel::Interrupt,
    };
    let mut client = target.connect(CHECK)?;
    let command = AgentCommand::Send {
        to: to.clone(),
        message,
        level: Some(level),
        message_id: Some(id.clone()),
    };
    // Safe to send again: the daemon keeps one message per id (P5).
    let result = command_within(&mut client, command, Redo::Safe, SEND_REPLY_WITHIN)?;
    let line = format!(
        "accepted: message {id} to {to} ({}){}",
        level.as_str(),
        retried(&client)
    );
    Ok(Output::new(vec![line], to_json(&result)))
}

pub fn inbox(target: &Target, after: Option<String>) -> Result<Output, Failure> {
    let result = command(
        target,
        AgentCommand::Inbox {
            after_message_id: after,
        },
        Redo::Safe,
    )?;
    let CommandResult::Messages { data } = &result else {
        return Ok(shown(result));
    };
    let mut lines = Vec::new();
    for message in &data.messages {
        let mut body = message.body.lines();
        lines.push(format!(
            "{} from {}: {}",
            message.message_id,
            message.from,
            body.next().unwrap_or("")
        ));
        lines.extend(body.map(|l| format!("  {l}")));
    }
    if lines.is_empty() {
        lines.push("no messages".into());
    }
    Ok(Output::new(lines, to_json(&result)))
}

pub fn done(target: &Target, text: &str) -> Result<Output, Failure> {
    let t = ticket(text)?;
    let command_ = AgentCommand::Done {
        task_id: t.task_id,
        identity: Some(t.identity),
    };
    command(target, command_, Redo::Never).map(shown)
}

pub fn result(target: &Target, text: &str, summary: String) -> Result<Output, Failure> {
    let t = ticket(text)?;
    let command_ = AgentCommand::Result {
        task_id: t.task_id,
        summary,
        output: None,
        identity: Some(t.identity),
    };
    command(target, command_, Redo::Never).map(shown)
}

pub fn review(target: &Target, review: Review) -> Result<Output, Failure> {
    let command_ = match review {
        Review::Approve { ticket: text } => {
            let t = ticket(&text)?;
            AgentCommand::ReviewApprove {
                task_id: t.task_id,
                identity: Some(t.identity),
            }
        }
        Review::Changes { ticket: text, what } => {
            let t = ticket(&text)?;
            AgentCommand::ReviewChanges {
                task_id: t.task_id,
                summary: what,
                identity: Some(t.identity),
            }
        }
    };
    command(target, command_, Redo::Never).map(shown)
}

pub fn ask(
    target: &Target,
    text: String,
    options: Vec<String>,
    follow_up: Option<String>,
    resolve: Option<String>,
) -> Result<Output, Failure> {
    let command_ = match (follow_up, resolve) {
        (Some(ask_id), _) => AgentCommand::AskFollowUp {
            ask_id,
            question: text,
            options,
        },
        (None, Some(ask_id)) => AgentCommand::AskResolve {
            ask_id,
            summary: text,
        },
        (None, None) => AgentCommand::Ask {
            question: text,
            options,
        },
    };
    command(target, command_, Redo::Never).map(shown)
}

/// The caller's task from `status` (inside an agent), else empty: the
/// daemon answers an operator `forbidden` before it looks at the task.
fn current_task(target: &Target, client: &mut agend_client::Client) -> Result<String, Failure> {
    if target.caller.is_none() {
        return Ok(String::new());
    }
    match command_on(client, AgentCommand::Status, Redo::Safe)? {
        CommandResult::Status { data } => Ok(data.task_id.unwrap_or_default()),
        _ => Ok(String::new()),
    }
}

pub fn block(target: &Target, reason: Option<String>) -> Result<Output, Failure> {
    let mut client = target.connect(CHECK)?;
    let task_id = current_task(target, &mut client)?;
    let command_ = match reason {
        Some(reason) => AgentCommand::Block { task_id, reason },
        None => AgentCommand::Unblock { task_id },
    };
    command_on(&mut client, command_, Redo::Never).map(shown)
}

/// `90s`, `30m`, `2h` in seconds.
pub fn delay_seconds(text: &str) -> Option<u64> {
    let unit = match text.chars().last()? {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        _ => return None,
    };
    let number = &text[..text.len() - 1];
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number.parse::<u64>().ok()?.checked_mul(unit)
}

pub fn remind(target: &Target, delay: &str) -> Result<Output, Failure> {
    let delay_seconds = delay_seconds(delay)
        .ok_or_else(|| Failure::usage(format!("invalid delay {delay:?}: use 90s, 30m or 2h")))?;
    let mut client = target.connect(CHECK)?;
    let task_id = current_task(target, &mut client)?;
    let command_ = AgentCommand::Remind {
        task_id,
        delay_seconds,
    };
    command_on(&mut client, command_, Redo::Never).map(shown)
}

pub fn task_create(
    target: &Target,
    title: String,
    role: String,
    team: Option<String>,
    workflow: Option<String>,
) -> Result<Output, Failure> {
    if target.caller.is_none() {
        // The operator's form goes through `operator` `task_create`, which
        // arrives with its handler in gate 10 (P1).
        if team.is_none() {
            return Err(Failure::usage(
                "the operator's agend task create needs --team <team>\nexample: agend task create --role dev \"<title>\" --team web",
            ));
        }
        return Err(Failure::coded(
            "not_supported",
            "the operator's agend task create arrives in gate 10; nothing was sent",
        ));
    }
    let command_ = AgentCommand::TaskCreate {
        title,
        role,
        team_id: team,
        workflow_id: workflow,
    };
    command(target, command_, Redo::Never).map(shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_are_seconds_minutes_or_hours() {
        assert_eq!(delay_seconds("90s"), Some(90));
        assert_eq!(delay_seconds("30m"), Some(1800));
        assert_eq!(delay_seconds("2h"), Some(7200));
        for bad in ["", "s", "10", "1d", "-1s", "1.5h", "m30"] {
            assert_eq!(delay_seconds(bad), None, "{bad}");
        }
    }
}
