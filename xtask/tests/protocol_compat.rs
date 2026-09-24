use agend_core::protocol::ask::{AnswerSource, AskEntry, AskReply, AskThread, ContextRecap};
use agend_core::protocol::client::{
    AgentCommand, ClientCommandData, ClientCommandResultData, ClientRequest, ClientResponse,
    CommandResult, DaemonEvent, EventData, TaskChangedData, TerminalInputData,
};
use agend_core::protocol::client::{AnswerAskData, AskCreatedData, AttentionRequiredData};
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

/// D35: asks take optional options and free-text answers, and continue as a
/// thread (question, answer, follow-up, resolution). D37: attention items
/// carry a context recap. All additive in v1.
#[test]
fn ask_threads_answers_and_recap_have_stable_wire_shapes() {
    let ask = AgentCommand::Ask {
        question: "Which storage?".into(),
        options: vec!["sqlite".into(), "files".into()],
    };
    assert_eq!(
        serde_json::to_value(ask).unwrap(),
        json!({"command": "ask", "question": "Which storage?", "options": ["sqlite", "files"]})
    );
    assert_eq!(
        serde_json::to_value(AgentCommand::AskFollowUp {
            ask_id: "A-1".into(),
            question: "About 2 GB. Still sqlite?".into(),
            options: Vec::new(),
        })
        .unwrap(),
        json!({"command": "ask_follow_up", "ask_id": "A-1", "question": "About 2 GB. Still sqlite?", "options": []})
    );
    assert_eq!(
        serde_json::to_value(AgentCommand::AskResolve {
            ask_id: "A-1".into(),
            summary: "Use sqlite.".into(),
        })
        .unwrap(),
        json!({"command": "ask_resolve", "ask_id": "A-1", "summary": "Use sqlite."})
    );

    let answer = ClientRequest::AnswerAsk {
        data: AnswerAskData {
            request_id: "r-3".into(),
            ask_id: "A-1".into(),
            source: AnswerSource::Telegram,
            reply: AskReply::Text {
                text: "depends on size".into(),
            },
        },
    };
    assert_eq!(
        serde_json::to_value(answer).unwrap(),
        json!({
            "type": "answer_ask",
            "data": {
                "request_id": "r-3",
                "ask_id": "A-1",
                "source": "telegram",
                "reply": {"reply": "text", "text": "depends on size"}
            }
        })
    );
    assert_eq!(
        serde_json::to_value(AskReply::Choice {
            option: "sqlite".into()
        })
        .unwrap(),
        json!({"reply": "choice", "option": "sqlite"})
    );
    assert_eq!(
        serde_json::to_value(CommandResult::AskCreated {
            data: AskCreatedData {
                ask_id: "A-1".into()
            }
        })
        .unwrap(),
        json!({"result": "ask_created", "data": {"ask_id": "A-1"}})
    );

    let thread = AskThread {
        ask_id: "A-1".into(),
        task_id: Some("T-1".into()),
        entries: vec![
            AskEntry::Question {
                from: "dev-1".into(),
                text: "Which storage?".into(),
                options: vec!["sqlite".into()],
            },
            AskEntry::Answer {
                from: "operator".into(),
                source: AnswerSource::Tui,
                reply: AskReply::Choice {
                    option: "sqlite".into(),
                },
            },
            AskEntry::Resolution {
                from: "dev-1".into(),
                summary: "Use sqlite.".into(),
            },
        ],
    };
    let attention = DaemonEvent::AttentionRequired {
        data: AttentionRequiredData {
            reason: "ask".into(),
            task_id: Some("T-1".into()),
            ask: Some(thread.clone()),
            recap: Some(ContextRecap {
                goal: "Persist the task queue".into(),
                decisions: vec!["Queue lives in the daemon".into()],
                asking: "Which storage?".into(),
                next: "dev-1 implements the store".into(),
            }),
        },
    };
    assert_eq!(
        serde_json::to_value(attention).unwrap(),
        json!({
            "event": "attention_required",
            "data": {
                "reason": "ask",
                "task_id": "T-1",
                "ask": {
                    "ask_id": "A-1",
                    "task_id": "T-1",
                    "entries": [
                        {"entry": "question", "from": "dev-1", "text": "Which storage?", "options": ["sqlite"]},
                        {"entry": "answer", "from": "operator", "source": "tui", "reply": {"reply": "choice", "option": "sqlite"}},
                        {"entry": "resolution", "from": "dev-1", "summary": "Use sqlite."}
                    ]
                },
                "recap": {
                    "goal": "Persist the task queue",
                    "decisions": ["Queue lives in the daemon"],
                    "asking": "Which storage?",
                    "next": "dev-1 implements the store"
                }
            }
        })
    );
    assert_eq!(
        serde_json::to_value(DaemonEvent::AskUpdated { data: thread }).unwrap()["event"],
        json!("ask_updated")
    );
}

/// Messages from a peer that predates D35/D37 still decode: the new fields
/// default, so the change stays additive within v1.
#[test]
fn pre_ask_thread_messages_still_decode() {
    assert_eq!(
        serde_json::from_value::<AgentCommand>(json!({"command": "ask", "question": "Proceed?"}))
            .unwrap(),
        AgentCommand::Ask {
            question: "Proceed?".into(),
            options: Vec::new(),
        }
    );
    assert_eq!(
        serde_json::from_value::<DaemonEvent>(json!({
            "event": "attention_required",
            "data": {"reason": "usage limit", "task_id": null}
        }))
        .unwrap(),
        DaemonEvent::AttentionRequired {
            data: AttentionRequiredData {
                reason: "usage limit".into(),
                task_id: None,
                ask: None,
                recap: None,
            }
        }
    );
    assert_eq!(
        serde_json::from_value::<AskEntry>(json!({"entry": "future_entry", "x": 1})).unwrap(),
        AskEntry::Unknown
    );
    assert_eq!(
        serde_json::from_value::<AnswerSource>(json!("slack")).unwrap(),
        AnswerSource::Unknown
    );
}
