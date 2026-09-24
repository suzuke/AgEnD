use agend_core::protocol::client::{
    AgentCommand, ClientCommandData, ClientCommandResultData, ClientRequest, ClientResponse,
    CommandResult, DaemonEvent, EventData, TaskChangedData, TerminalInputData,
};
use agend_core::protocol::holder::{
    ControlKey, HolderRequest, HolderResponse, OperatorTerminalInputData,
};
use agend_core::protocol::{Hello, ProtocolVersion};
use serde_json::json;

#[test]
fn client_request_wire_shapes_are_stable_and_approval_does_not_supply_a_head() {
    assert_eq!(
        serde_json::to_value(ClientRequest::hello()).unwrap(),
        json!({"type": "hello", "data": {"supported": [{"major": 1, "minor": 0}]}})
    );

    let review = ClientRequest::Command {
        data: ClientCommandData {
            request_id: "r-1".into(),
            command: AgentCommand::ReviewApprove {
                task_id: "T-1".into(),
            },
        },
    };
    assert_eq!(
        serde_json::to_value(review).unwrap(),
        json!({
            "type": "command",
            "data": {
                "request_id": "r-1",
                "command": {"command": "review_approve", "task_id": "T-1"}
            }
        })
    );

    let input = ClientRequest::TerminalInput {
        data: TerminalInputData {
            instance_id: "i-1".into(),
            bytes_base64: "AQI=".into(),
        },
    };
    assert_eq!(
        serde_json::to_value(input).unwrap(),
        json!({
            "type": "terminal_input",
            "data": {"instance_id": "i-1", "bytes_base64": "AQI="}
        })
    );
}

#[test]
fn client_response_and_event_wire_shapes_are_stable() {
    let response = ClientResponse::CommandResult {
        data: ClientCommandResultData {
            request_id: "r-2".into(),
            result: CommandResult::Accepted,
        },
    };
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({
            "type": "command_result",
            "data": {"request_id": "r-2", "result": {"result": "accepted"}}
        })
    );

    let event = DaemonEvent::TaskChanged {
        data: TaskChangedData {
            task_id: "T-1".into(),
            summary: "running".into(),
        },
    };
    assert_eq!(
        serde_json::to_value(ClientResponse::Event {
            data: EventData { event_id: 9, event },
        })
        .unwrap(),
        json!({
            "type": "event",
            "data": {
                "event_id": 9,
                "event": {"event": "task_changed", "data": {"task_id": "T-1", "summary": "running"}}
            }
        })
    );
}

#[test]
fn holder_operator_input_and_control_keys_have_stable_wire_shapes() {
    assert_eq!(
        serde_json::to_value(HolderRequest::OperatorTerminalInput {
            data: OperatorTerminalInputData {
                bytes_base64: "AQI=".into(),
            },
        })
        .unwrap(),
        json!({"type": "operator_terminal_input", "data": {"bytes_base64": "AQI="}})
    );
    assert_eq!(
        serde_json::to_value(ControlKey::Enter).unwrap(),
        json!("enter")
    );
    assert_eq!(
        serde_json::to_value(ControlKey::Digit1).unwrap(),
        json!("digit1")
    );
}

#[test]
fn unknown_tagged_variants_are_tolerated_at_protocol_boundaries() {
    assert_eq!(
        serde_json::from_value::<ClientRequest>(json!({"type": "future_request", "data": {}}))
            .unwrap(),
        ClientRequest::Unknown
    );
    assert_eq!(
        serde_json::from_value::<AgentCommand>(json!({"command": "future_command"})).unwrap(),
        AgentCommand::Unknown
    );
    assert_eq!(
        serde_json::from_value::<ControlKey>(json!("future_key")).unwrap(),
        ControlKey::Unknown
    );
    assert_eq!(
        serde_json::from_value::<ClientResponse>(json!({"type": "future_response", "data": {}}))
            .unwrap(),
        ClientResponse::Unknown
    );
    assert_eq!(
        serde_json::from_value::<CommandResult>(json!({"result": "future_result"})).unwrap(),
        CommandResult::Unknown
    );
    assert_eq!(
        serde_json::from_value::<DaemonEvent>(json!({"event": "future_event"})).unwrap(),
        DaemonEvent::Unknown
    );
    assert_eq!(
        serde_json::from_value::<HolderRequest>(json!({"type": "future_request", "data": {}}))
            .unwrap(),
        HolderRequest::Unknown
    );
    assert_eq!(
        serde_json::from_value::<HolderResponse>(json!({"type": "future_response", "data": {}}))
            .unwrap(),
        HolderResponse::Unknown
    );
}

#[test]
fn additive_fields_and_hello_version_round_trip() {
    let hello: Hello = serde_json::from_value(json!({
        "supported": [{"major": 1, "minor": 0}],
        "future_field": true
    }))
    .unwrap();
    assert_eq!(hello.supported, [ProtocolVersion::new(1, 0)]);

    let encoded = serde_json::to_value(hello).unwrap();
    let decoded: Hello = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.supported, [ProtocolVersion::new(1, 0)]);
}
