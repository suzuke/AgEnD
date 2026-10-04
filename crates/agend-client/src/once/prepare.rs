//! Borrowed-size checks before JSON scans a potentially huge string.
//! The encoder checks the exact escaped size and deadline separately.

use agend_core::protocol::ask::AskReply;
use agend_core::protocol::client::{
    AgentCommand, ClientRequest, MAX_LINE_BYTES, MAX_MESSAGE_BYTES, OperatorCommand, ResultIdentity,
};
use std::io;
use std::time::Instant;

struct Budget {
    bytes: usize,
    deadline: Instant,
}

impl Budget {
    fn add(&mut self, text: &str) -> io::Result<()> {
        super::remaining(self.deadline)?;
        self.bytes = self.bytes.saturating_add(text.len());
        if self.bytes > MAX_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "request exceeds protocol line limit",
            ));
        }
        Ok(())
    }
    fn strings<'a>(&mut self, texts: impl IntoIterator<Item = &'a str>) -> io::Result<()> {
        for text in texts {
            self.add(text)?;
        }
        Ok(())
    }
    fn identity(&mut self, identity: &Option<ResultIdentity>) -> io::Result<()> {
        if let Some(identity) = identity {
            self.add(&identity.stage_id)?;
        }
        Ok(())
    }
    fn agent(&mut self, command: &AgentCommand) -> io::Result<()> {
        match command {
            AgentCommand::Status | AgentCommand::Unknown => Ok(()),
            AgentCommand::Done { task_id, identity }
            | AgentCommand::ReviewApprove { task_id, identity } => {
                self.add(task_id)?;
                self.identity(identity)
            }
            AgentCommand::Result {
                task_id,
                summary,
                output,
                identity,
            } => {
                self.strings([task_id.as_str(), summary])?;
                self.strings(output.as_deref())?;
                self.identity(identity)
            }
            AgentCommand::ReviewChanges {
                task_id,
                summary,
                identity,
            } => {
                self.strings([task_id.as_str(), summary])?;
                self.identity(identity)
            }
            AgentCommand::Send {
                to,
                message,
                message_id,
                ..
            } => {
                if message.len() > MAX_MESSAGE_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "send body exceeds message limit",
                    ));
                }
                self.strings([to.as_str(), message])?;
                self.strings(message_id.as_deref())
            }
            AgentCommand::Inbox { after_message_id } => self.strings(after_message_id.as_deref()),
            AgentCommand::Ask { question, options } => {
                self.add(question)?;
                self.strings(options.iter().map(String::as_str))
            }
            AgentCommand::AskFollowUp {
                ask_id,
                question,
                options,
            } => {
                self.strings([ask_id.as_str(), question])?;
                self.strings(options.iter().map(String::as_str))
            }
            AgentCommand::AskResolve { ask_id, summary } => {
                self.strings([ask_id.as_str(), summary])
            }
            AgentCommand::Block { task_id, reason } => self.strings([task_id.as_str(), reason]),
            AgentCommand::Unblock { task_id } | AgentCommand::Remind { task_id, .. } => {
                self.add(task_id)
            }
            AgentCommand::TaskCreate {
                title,
                role,
                team_id,
                workflow_id,
            } => {
                self.strings([title.as_str(), role])?;
                self.strings(team_id.as_deref())?;
                self.strings(workflow_id.as_deref())
            }
        }
    }
    fn operator(&mut self, command: &OperatorCommand) -> io::Result<()> {
        match command {
            OperatorCommand::InstanceAdd {
                instance_id,
                backend,
                working_directory,
                program,
                args,
            } => {
                self.strings([instance_id.as_str(), backend])?;
                self.strings(working_directory.as_deref())?;
                self.strings(program.as_deref())?;
                self.strings(args.iter().map(String::as_str))
            }
            OperatorCommand::InstanceRemove { instance_id } => self.add(instance_id),
            OperatorCommand::DaemonRestart { binary } => self.strings(binary.as_deref()),
            OperatorCommand::TaskCancel { task_id, reason } => {
                self.add(task_id)?;
                self.strings(reason.as_deref())
            }
            OperatorCommand::TaskCreate {
                title,
                role,
                team_id,
                workflow_id,
            } => {
                self.strings([title.as_str(), role, team_id])?;
                self.strings(workflow_id.as_deref())
            }
            OperatorCommand::TeamAdd {
                team_id,
                repo,
                workflow_id,
            } => {
                self.add(team_id)?;
                self.strings(repo.as_deref())?;
                self.strings(workflow_id.as_deref())
            }
            OperatorCommand::TeamJoin {
                team_id,
                instance_id,
                role,
            } => self.strings([team_id.as_str(), instance_id, role]),
            OperatorCommand::TeamSetWorkflow {
                team_id,
                workflow_id,
            } => self.strings([team_id.as_str(), workflow_id]),
            OperatorCommand::WorkflowShow { workflow_id } => self.add(workflow_id),
            OperatorCommand::WorkflowCheck { toml } | OperatorCommand::WorkflowApply { toml } => {
                self.add(toml)
            }
            OperatorCommand::TeamList
            | OperatorCommand::WorkflowList
            | OperatorCommand::Unknown => Ok(()),
        }
    }
}

pub(super) fn check(request: &ClientRequest, deadline: Instant) -> io::Result<()> {
    let mut budget = Budget { bytes: 0, deadline };
    match request {
        ClientRequest::Command { data } => {
            budget.add(&data.request_id)?;
            budget.agent(&data.command)
        }
        ClientRequest::Operator { data } => {
            budget.add(&data.request_id)?;
            budget.operator(&data.command)
        }
        ClientRequest::GetFleet { data } => budget.add(&data.request_id),
        ClientRequest::ResolveAttention { data } => {
            budget.strings([data.request_id.as_str(), &data.attention_id])?;
            budget.strings(data.note.as_deref())
        }
        ClientRequest::AnswerAsk { data } => {
            budget.strings([data.request_id.as_str(), &data.ask_id])?;
            match &data.reply {
                AskReply::Choice { option } => budget.add(option),
                AskReply::Text { text } => budget.add(text),
                AskReply::Unknown => Ok(()),
            }
        }
        ClientRequest::Hello { data } => budget.strings(data.caller.as_deref()),
        // Listing every variant forces future RPCs to add their size check.
        ClientRequest::SubscribeEvents { .. }
        | ClientRequest::SubscribeTerminal { .. }
        | ClientRequest::TerminalInput { .. }
        | ClientRequest::SubscribeTerminalFrames { .. }
        | ClientRequest::SetTerminalViewport { .. }
        | ClientRequest::TerminalControl { .. }
        | ClientRequest::Unknown => Ok(()),
    }
}
