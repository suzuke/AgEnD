//! Team and workflow CLI. Permissions are checked by the daemon.
use super::{Failure, Output, Target, to_json};
use agend_client::Redo;
use agend_core::protocol::client::{
    ClientRequest, ClientResponse, CommandResult, OperatorCommand, OperatorData,
};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum Team {
    Add {
        team: String,
        #[arg(long)]
        repo: Option<std::path::PathBuf>,
        #[arg(long)]
        workflow: Option<String>,
    },
    List,
    Join {
        team: String,
        name: String,
        #[arg(long)]
        role: String,
    },
    SetWorkflow {
        team: String,
        id: String,
    },
}
#[derive(Subcommand)]
pub enum Workflow {
    List,
    Show { id: String },
    Check { file: std::path::PathBuf },
    Apply { file: std::path::PathBuf },
}

pub fn request(target: &Target, command: OperatorCommand) -> Result<Output, Failure> {
    let mut client = target.connect("agend status")?;
    let request = ClientRequest::Operator {
        data: OperatorData {
            request_id: client.next_request_id(),
            command,
        },
    };
    match client
        .request(&request, Redo::Never)
        .map_err(|e| Failure::client(e, "agend status"))?
    {
        ClientResponse::CommandResult { data } => {
            let line = match &data.result {
                CommandResult::Text { text } => text.clone(),
                CommandResult::TaskCreated { data } => format!("created {}", data.task_id),
                _ => "accepted".into(),
            };
            Ok(Output::new(vec![line], to_json(&data.result)))
        }
        _ => Err(Failure::new("disconnected", "unexpected reply")),
    }
}
pub fn team(target: &Target, command: Team) -> Result<Output, Failure> {
    let command = match command {
        Team::Add {
            team,
            repo,
            workflow,
        } => {
            let repo = repo
                .map(|p| p.canonicalize().map(|p| p.display().to_string()))
                .transpose()
                .map_err(|e| Failure::usage(e.to_string()))?;
            OperatorCommand::TeamAdd {
                team_id: team,
                repo,
                workflow_id: workflow,
            }
        }
        Team::List => OperatorCommand::TeamList,
        Team::Join { team, name, role } => OperatorCommand::TeamJoin {
            team_id: team,
            instance_id: name,
            role,
        },
        Team::SetWorkflow { team, id } => OperatorCommand::TeamSetWorkflow {
            team_id: team,
            workflow_id: id,
        },
    };
    request(target, command)
}
pub fn workflow(target: &Target, command: Workflow) -> Result<Output, Failure> {
    let read = |file: &std::path::Path| {
        std::fs::read_to_string(file)
            .map_err(|e| Failure::usage(format!("{}: {e}", file.display())))
    };
    let command = match command {
        Workflow::List => OperatorCommand::WorkflowList,
        Workflow::Show { id } => OperatorCommand::WorkflowShow { workflow_id: id },
        Workflow::Check { file } => OperatorCommand::WorkflowCheck { toml: read(&file)? },
        Workflow::Apply { file } => OperatorCommand::WorkflowApply { toml: read(&file)? },
    };
    request(target, command)
}
