//! Parse envelope tags before their native core data types. Deriving an
//! internally tagged enum buffers a serde Content tree and converts it after
//! the last Read; that large CPU tail cannot consult the transport deadline.
//! RawValue preserves arbitrary field ordering without constructing that tree.

use agend_core::protocol::client::*;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::value::RawValue;
use std::{io, time::Instant};

mod fleet;

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    data: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
struct CommandEnvelope {
    result: String,
    data: Option<Box<RawValue>>,
    text: Option<Box<RawValue>>,
}

#[derive(Deserialize)]
struct CommandReply {
    request_id: String,
    result: Box<RawValue>,
}

fn parse<T: DeserializeOwned>(bytes: &[u8], deadline: Instant) -> io::Result<T> {
    let value = serde_json::from_reader(super::sliced_json::Decoding {
        bytes,
        deadline,
        until_check: 0,
    })
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    super::remaining(deadline)?;
    Ok(value)
}

fn data<T: DeserializeOwned>(value: Option<Box<RawValue>>, deadline: Instant) -> io::Result<T> {
    let value = value
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing one-shot reply data"))?;
    parse(value.get().as_bytes(), deadline)
}

fn command(bytes: &[u8], deadline: Instant) -> io::Result<ClientCommandResultData> {
    let reply: CommandReply = parse(bytes, deadline)?;
    let envelope: CommandEnvelope = parse(reply.result.get().as_bytes(), deadline)?;
    let result = match envelope.result.as_str() {
        "text" => CommandResult::Text {
            text: data(envelope.text, deadline)?,
        },
        "accepted" => CommandResult::Accepted,
        "backend_switch" => CommandResult::BackendSwitch {
            data: match envelope.data {
                Some(raw) => parse(raw.get().as_bytes(), deadline)?,
                None => None,
            },
        },
        "message_outcome" => CommandResult::MessageOutcome {
            data: data(envelope.data, deadline)?,
        },
        "driver_status" => CommandResult::DriverStatus {
            data: data(envelope.data, deadline)?,
        },
        "message_delivery" => CommandResult::MessageDelivery {
            data: match envelope.data {
                Some(raw) => parse(raw.get().as_bytes(), deadline)?,
                None => None,
            },
        },
        "status" => CommandResult::Status {
            data: data(envelope.data, deadline)?,
        },
        "messages" => CommandResult::Messages {
            data: data(envelope.data, deadline)?,
        },
        "task_created" => CommandResult::TaskCreated {
            data: data(envelope.data, deadline)?,
        },
        "ask_created" => CommandResult::AskCreated {
            data: data(envelope.data, deadline)?,
        },
        "instance_added" => CommandResult::InstanceAdded {
            data: data(envelope.data, deadline)?,
        },
        "restarting" => CommandResult::Restarting {
            data: data(envelope.data, deadline)?,
        },
        _ => CommandResult::Unknown,
    };
    Ok(ClientCommandResultData {
        request_id: reply.request_id,
        result,
    })
}

pub(super) fn response(bytes: &[u8], deadline: Instant) -> io::Result<ClientResponse> {
    let envelope: Envelope = parse(bytes, deadline)?;
    let response = match envelope.kind.as_str() {
        "claude" => ClientResponse::Claude {
            data: data(envelope.data, deadline)?,
        },
        "hello" => ClientResponse::Hello {
            data: data(envelope.data, deadline)?,
        },
        "fleet" => ClientResponse::Fleet {
            data: fleet::decode(envelope.data, deadline)?,
        },
        "error" => ClientResponse::Error {
            data: data(envelope.data, deadline)?,
        },
        "command_result" => {
            let value = envelope.data.ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "missing one-shot reply data")
            })?;
            ClientResponse::CommandResult {
                data: command(value.get().as_bytes(), deadline)?,
            }
        }
        // This API never returns subscriptions or terminal operations. Their
        // valid JSON is consumed within the budget, then skipped by the caller.
        _ => ClientResponse::Unknown,
    };
    super::remaining(deadline)?;
    Ok(response)
}
