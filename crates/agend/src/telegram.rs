//! Pairing RPCs; token resolution and all Telegram HTTP stay in the daemon.
mod apply;
use crate::cli::{Failure, Output, Target, to_json};
use agend_client::Redo;
use agend_core::{config::SecretRef, protocol::client::*, telegram::pairing::*};
use clap::Subcommand;
use std::{path::PathBuf, time::Duration};

#[derive(Subcommand)]
pub enum Command {
    /// Pair an exact chat and sender; confirmation does not yet edit config.toml
    #[command(subcommand)]
    Setup(Setup),
}
#[derive(Subcommand)]
pub enum Setup {
    /// Begin a ten-minute challenge using a token reference, never an inline token
    Begin {
        #[arg(
            long,
            conflicts_with = "token_file",
            required_unless_present = "token_file"
        )]
        token_env: Option<String>,
        #[arg(
            long,
            conflicts_with = "token_env",
            required_unless_present = "token_env"
        )]
        token_file: Option<PathBuf>,
        /// Exact previous completed, cancelled or expired pairing ID
        #[arg(long)]
        previous: Option<String>,
    },
    /// Read the durable receipt after any interrupted operation
    Status,
    /// Read available Telegram updates once and save any matching destination
    Poll {
        #[arg(long)]
        id: String,
    },
    /// Confirm exactly the observed destination and sender
    Confirm {
        #[arg(long)]
        id: String,
        #[arg(long, allow_hyphen_values = true)]
        chat: i64,
        #[arg(long)]
        user: u64,
        #[arg(long)]
        topic: Option<i64>,
    },
    /// Apply this confirmed receipt to config.toml; retain an original backup
    Apply {
        #[arg(long)]
        id: String,
    },
    /// Cancel exactly this pairing
    Cancel {
        #[arg(long)]
        id: String,
    },
}
pub fn run(command: Command) -> Result<Output, Failure> {
    let target = Target::from_env()?;
    if target.caller.is_some() {
        return Err(Failure::new(
            "forbidden",
            "only the operator can pair Telegram",
        ));
    }
    let Command::Setup(setup) = command;
    let apply_id = match &setup {
        Setup::Apply { id } => Some(id.clone()),
        _ => None,
    };
    let operation = match setup {
        Setup::Status | Setup::Apply { .. } => PairingOperation::Status,
        Setup::Begin {
            token_env,
            token_file,
            previous,
        } => {
            let token = match (token_env, token_file) {
                (Some(name), None) => SecretRef::Env(name),
                (None, Some(path)) if path.is_absolute() => SecretRef::File(
                    path.to_str()
                        .ok_or_else(|| {
                            Failure::new("invalid_request", "token file path must be UTF-8")
                        })?
                        .into(),
                ),
                _ => {
                    return Err(Failure::new(
                        "invalid_request",
                        "provide one token environment name or absolute private token file path",
                    ));
                }
            };
            PairingOperation::Begin {
                id: new_id()?,
                token,
                previous,
            }
        }
        Setup::Poll { id } => PairingOperation::Poll { id },
        Setup::Confirm {
            id,
            chat,
            user,
            topic,
        } => PairingOperation::Confirm {
            id,
            candidate: PairingCandidate {
                chat_id: chat,
                user_id: user,
                topic_id: topic,
            },
        },
        Setup::Cancel { id } => PairingOperation::Cancel { id },
    };
    let check = "agend telegram setup status";
    let mut client = target.connect(check)?;
    let version = client.daemon().selected;
    if version.major != V1_9.major || version.minor < V1_9.minor {
        return Err(Failure::new(
            "not_supported",
            "Telegram pairing requires daemon client protocol 1.9; upgrade the daemon",
        ));
    }
    let request = ClientRequest::Operator {
        data: OperatorData {
            request_id: client.next_request_id(),
            command: OperatorCommand::TelegramPairing { operation },
        },
    };
    // Poll includes two bounded HTTP calls. Mutations are never replayed.
    let response = client
        .request_within(&request, Redo::Never, Duration::from_secs(90))
        .map_err(|e| Failure::client(e, check))?;
    let ClientResponse::CommandResult { data } = response else {
        return Err(Failure::new(
            "disconnected",
            "unexpected pairing response; query status",
        ));
    };
    let CommandResult::TelegramPairing { data } = data.result else {
        return Err(Failure::new(
            "disconnected",
            "unexpected pairing result; query status",
        ));
    };
    if let Some(id) = apply_id {
        let record = data
            .as_deref()
            .filter(|record| record.session.id == id)
            .ok_or_else(|| {
                Failure::new(
                    "invalid_request",
                    "pairing changed or missing; query status",
                )
            })?;
        return apply::run(&crate::home::resolve()?, record);
    }
    let mut lines = Vec::new();
    match &data {
        None => lines.push("No Telegram pairing; run agend telegram setup begin --help".into()),
        Some(record) => {
            lines.push(format!("Pairing {}: {:?}", record.session.id, record.phase));
            match record.phase {
                PairingPhase::Pending => {
                    if let Some(candidate) = &record.session.candidate {
                        lines.push(format!(
                            "Observed chat {} user {} topic {:?}; confirm these exact IDs",
                            candidate.chat_id, candidate.user_id, candidate.topic_id
                        ));
                    } else {
                        lines.push(format!("Send {} to @{} in the intended chat, then run agend telegram setup poll --id {}", record.session.command(), record.session.bot_username, record.session.id));
                    }
                    lines.push(format!(
                        "Expires at Unix milliseconds {}",
                        record.session.expires_at_ms
                    ));
                }
                PairingPhase::Confirmed => lines.push(format!(
                    "Destination confirmed; apply with agend telegram setup apply --id {}",
                    record.session.id
                )),
                PairingPhase::Cancelled => lines.push("Pairing cancelled".into()),
            }
        }
    }
    Ok(Output::new(lines, to_json(&data)))
}

fn new_id() -> Result<String, Failure> {
    use std::io::Read;
    let mut bytes = [0; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| Failure::new("unavailable", "cannot generate a secure pairing challenge"))?;
    Ok(uuid_v4(bytes))
}
