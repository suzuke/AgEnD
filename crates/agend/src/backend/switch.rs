//! Correlated operator RPCs. Unknown mutation outcomes are queried, never replayed.
use crate::cli::{Failure, Output, Target, to_json};
use agend_client::Redo;
use agend_core::{
    protocol::client::{
        BackendSwitchCommand, ClientRequest, ClientResponse, CommandResult, OperatorCommand,
        OperatorData, V1_8,
    },
    runtime_records::BackendSwitchPhase,
};
use clap::Subcommand;
use std::time::Duration;

#[derive(Subcommand)]
pub enum Command {
    /// Pause new delivery for a verified target; does not activate it
    Prepare {
        instance: String,
        #[arg(long)]
        version: String,
        /// ID of the previous completed/cancelled switch, if one exists
        #[arg(long)]
        previous: Option<String>,
    },
    /// Read the durable switch record after any interrupted request
    Status { instance: String },
    /// Activate exactly this prepared switch; readiness completes asynchronously
    Activate {
        instance: String,
        #[arg(long)]
        switch_id: String,
    },
    /// Restore the original version of this switch
    Rollback {
        instance: String,
        #[arg(long)]
        switch_id: String,
    },
    /// Cancel exactly this prepared switch and resume new delivery
    Cancel {
        instance: String,
        #[arg(long)]
        switch_id: String,
    },
}

pub fn run(command: Command) -> Result<Output, Failure> {
    let (instance, operation) = match command {
        Command::Prepare {
            instance,
            version,
            previous,
        } => (
            instance.clone(),
            BackendSwitchCommand::Prepare {
                instance_id: instance,
                version,
                expected_previous: previous,
            },
        ),
        Command::Status { instance } => (
            instance.clone(),
            BackendSwitchCommand::Status {
                instance_id: instance,
            },
        ),
        Command::Activate {
            instance,
            switch_id,
        } => (
            instance.clone(),
            BackendSwitchCommand::Activate {
                instance_id: instance,
                switch_id,
            },
        ),
        Command::Rollback {
            instance,
            switch_id,
        } => (
            instance.clone(),
            BackendSwitchCommand::Rollback {
                instance_id: instance,
                switch_id,
            },
        ),
        Command::Cancel {
            instance,
            switch_id,
        } => (
            instance.clone(),
            BackendSwitchCommand::Cancel {
                instance_id: instance,
                switch_id,
            },
        ),
    };
    let check = format!("agend backend switch status {instance}");
    let mut client = Target::from_env()?.connect(&check)?;
    let selected = client.daemon().selected;
    if selected.major != V1_8.major || selected.minor < V1_8.minor {
        return Err(Failure::new(
            "not_supported",
            "backend switching requires daemon client protocol 1.8; upgrade the daemon",
        ));
    }
    let request = ClientRequest::Operator {
        data: OperatorData {
            request_id: client.next_request_id(),
            command: OperatorCommand::BackendSwitch { operation },
        },
    };
    let reply = client
        .request_within(&request, Redo::Never, Duration::from_secs(30))
        .map_err(|e| Failure::client(e, &check))?;
    let ClientResponse::CommandResult { data } = reply else {
        return Err(Failure::new(
            "disconnected",
            "unexpected backend switch response",
        ));
    };
    let CommandResult::BackendSwitch { data } = data.result else {
        return Err(Failure::new(
            "disconnected",
            "unexpected backend switch result",
        ));
    };
    let text = match &data {
        None => format!("{instance}: no backend switch"),
        Some(record) => format!(
            "{instance}: switch {}: {}",
            record.id,
            match record.phase {
                BackendSwitchPhase::Prepared =>
                    "prepared; new delivery paused; target not activated",
                BackendSwitchPhase::Cancelled => "cancelled; new delivery resumed",
                BackendSwitchPhase::Committed =>
                    "target selected; activation pending; delivery paused",
                BackendSwitchPhase::Activated => "activated; new delivery resumed",
                BackendSwitchPhase::RollbackPrepared => "rollback prepared; new delivery paused",
                BackendSwitchPhase::Restoring =>
                    "original selected; activation pending; delivery paused",
                BackendSwitchPhase::RolledBack => "rolled back",
            }
        ),
    };
    Ok(Output::new(vec![text], to_json(&data)))
}
