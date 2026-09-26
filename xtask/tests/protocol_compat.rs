use agend_core::protocol::ask::{AnswerSource, AskEntry, AskReply, AskThread, ContextRecap};
use agend_core::protocol::client::{
    AgentCommand, ClientCommandData, ClientCommandResultData, ClientRequest, ClientResponse,
    CommandResult, DaemonEvent, EventData, TaskChangedData, TerminalInputData,
};
use agend_core::protocol::client::{
    AnswerAskData, AskCreatedData, AttentionRequiredData, ResultIdentity, STALE_RESULT,
};
use agend_core::protocol::holder::{
    ControlKey, ExitedData, HolderRequest, HolderResponse, OperatorTerminalInputData,
};
use agend_core::protocol::{Hello, ProtocolVersion};
use serde_json::json;

#[test]
fn client_request_wire_shapes_are_stable_and_approval_does_not_supply_a_head() {
    assert_eq!(
        serde_json::to_value(ClientRequest::hello()).unwrap(),
        json!({"type": "hello", "data": {"supported": [{"major": 1, "minor": 1}]}})
    );

    let review = ClientRequest::Command {
        data: ClientCommandData {
            request_id: "r-1".into(),
            command: AgentCommand::ReviewApprove {
                task_id: "T-1".into(),
                identity: None,
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
            task: None,
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
fn holder_exited_signal_is_an_additive_optional_field() {
    // A holder that predates `signal` sends only `code`; it must still parse.
    assert_eq!(
        serde_json::from_value::<HolderResponse>(json!({"type": "exited", "data": {"code": 7}}))
            .unwrap(),
        HolderResponse::Exited {
            data: ExitedData {
                code: Some(7),
                signal: None
            }
        }
    );
    // A normal exit keeps the old wire shape exactly (no `signal` key).
    assert_eq!(
        serde_json::to_value(HolderResponse::Exited {
            data: ExitedData {
                code: Some(7),
                signal: None
            }
        })
        .unwrap(),
        json!({"type": "exited", "data": {"code": 7}})
    );
    assert_eq!(
        serde_json::to_value(HolderResponse::Exited {
            data: ExitedData {
                code: None,
                signal: Some("SIGKILL".into())
            }
        })
        .unwrap(),
        json!({"type": "exited", "data": {"code": null, "signal": "SIGKILL"}})
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
            attention_id: None,
            unblocks: None,
            waiting_since_unix_ms: None,
            if_ignored: None,
            actions: Vec::new(),
            instance_id: None,
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
                attention_id: None,
                unblocks: None,
                waiting_since_unix_ms: None,
                if_ignored: None,
                actions: Vec::new(),
                instance_id: None,
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

/// Event identity on agent results: the stage attempt from the assignment is
/// echoed back. It is optional on the wire (older v1 peers decode), and a
/// result without it is stale by default (`stale_result`).
#[test]
fn agent_results_carry_the_stage_attempt_identity() {
    let identity = || {
        Some(ResultIdentity {
            stage_id: "checks".into(),
            attempt: 2,
        })
    };
    assert_eq!(
        serde_json::to_value(AgentCommand::ReviewApprove {
            task_id: "T-1".into(),
            identity: identity(),
        })
        .unwrap(),
        json!({
            "command": "review_approve",
            "task_id": "T-1",
            "identity": {"stage_id": "checks", "attempt": 2}
        })
    );
    assert_eq!(
        serde_json::to_value(AgentCommand::Done {
            task_id: "T-1".into(),
            identity: identity(),
        })
        .unwrap(),
        json!({"command": "done", "task_id": "T-1", "identity": {"stage_id": "checks", "attempt": 2}})
    );
    assert_eq!(
        serde_json::to_value(AgentCommand::ReviewChanges {
            task_id: "T-1".into(),
            summary: "rename".into(),
            identity: identity(),
        })
        .unwrap()["identity"]["attempt"],
        json!(2)
    );
    // A pre-identity message still decodes, with no identity (stale by default).
    assert_eq!(
        serde_json::from_value::<AgentCommand>(json!({"command": "done", "task_id": "T-1"}))
            .unwrap(),
        AgentCommand::Done {
            task_id: "T-1".into(),
            identity: None,
        }
    );
    assert_eq!(
        serde_json::from_value::<AgentCommand>(json!({
            "command": "result",
            "task_id": "T-1",
            "summary": "s",
            "output": null
        }))
        .unwrap(),
        AgentCommand::Result {
            task_id: "T-1".into(),
            summary: "s".into(),
            output: None,
            identity: None,
        }
    );
    assert_eq!(STALE_RESULT, "stale_result");
}

/// Client protocol 1.0 as it shipped, frozen: the peer that predates gate 8.
/// Only the parts 1.1 touches (the rest did not change).
mod v1_0 {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Version {
        pub major: u16,
        pub minor: u16,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Hello {
        pub supported: Vec<Version>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum ClientRequest {
        Hello {
            data: Hello,
        },
        SubscribeEvents {
            data: SubscribeEventsData,
        },
        #[serde(other)]
        Unknown,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct SubscribeEventsData {
        pub after_event_id: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum ClientResponse {
        Event {
            data: EventData,
        },
        #[serde(other)]
        Unknown,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct EventData {
        pub event_id: u64,
        pub event: DaemonEvent,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "event", rename_all = "snake_case")]
    pub enum DaemonEvent {
        AttentionRequired {
            data: AttentionRequiredData,
        },
        TaskChanged {
            data: TaskChangedData,
        },
        InstanceChanged {
            data: InstanceChangedData,
        },
        #[serde(other)]
        Unknown,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct AttentionRequiredData {
        pub reason: String,
        pub task_id: Option<String>,
        #[serde(default)]
        pub ask: Option<serde_json::Value>,
        #[serde(default)]
        pub recap: Option<serde_json::Value>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct TaskChangedData {
        pub task_id: String,
        pub summary: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct InstanceChangedData {
        pub instance_id: String,
        pub summary: String,
    }
}

fn reencode<T: serde::Serialize, U: serde::de::DeserializeOwned>(value: &T) -> U {
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

/// Gate 8 P3: a 1.0 peer decodes every 1.1 message: new requests, replies
/// and events are `unknown`, new fields are ignored, the rest is unchanged.
#[test]
fn a_1_0_peer_decodes_1_1_messages() {
    use agend_core::protocol::client::{
        AgentState, AttentionAction, AttentionResolvedData, FleetData, FleetView,
        InstanceChangedData, InstanceView, RequestIdData, ResolveAttentionData,
    };
    let hello: v1_0::ClientRequest = reencode(&ClientRequest::hello_as(Some("g8-1".into())));
    assert_eq!(
        hello,
        v1_0::ClientRequest::Hello {
            data: v1_0::Hello {
                supported: vec![v1_0::Version { major: 1, minor: 1 }]
            }
        }
    );
    let get_fleet = ClientRequest::GetFleet {
        data: RequestIdData {
            request_id: "r-1".into(),
        },
    };
    let resolve = ClientRequest::ResolveAttention {
        data: ResolveAttentionData {
            request_id: "r-2".into(),
            attention_id: "instance-failed:g8-2".into(),
            action: AttentionAction::Retry,
        },
    };
    for request in [get_fleet, resolve] {
        assert_eq!(
            reencode::<_, v1_0::ClientRequest>(&request),
            v1_0::ClientRequest::Unknown
        );
    }
    let fleet = ClientResponse::Fleet {
        data: FleetData {
            request_id: "r-1".into(),
            fleet: FleetView {
                as_of_event_id: 1_790_000_000_000_000,
                teams: vec![],
                tasks: vec![],
                instances: vec![],
                attention: vec![],
            },
        },
    };
    assert_eq!(
        reencode::<_, v1_0::ClientResponse>(&fleet),
        v1_0::ClientResponse::Unknown
    );
    let event = |event: DaemonEvent| ClientResponse::Event {
        data: EventData { event_id: 7, event },
    };
    let v1_0_event = |event: v1_0::DaemonEvent| v1_0::ClientResponse::Event {
        data: v1_0::EventData { event_id: 7, event },
    };
    let resolved = event(DaemonEvent::AttentionResolved {
        data: AttentionResolvedData {
            attention_id: "instance-failed:g8-2".into(),
            action: AttentionAction::Retry,
        },
    });
    assert_eq!(
        reencode::<_, v1_0::ClientResponse>(&resolved),
        v1_0_event(v1_0::DaemonEvent::Unknown)
    );
    let required = event(DaemonEvent::AttentionRequired {
        data: AttentionRequiredData {
            reason: "g8-2 failed: restarted 3 times".into(),
            task_id: None,
            ask: None,
            recap: None,
            attention_id: Some("instance-failed:g8-2".into()),
            unblocks: Some(0),
            waiting_since_unix_ms: Some(1_790_000_000_000),
            if_ignored: Some("g8-2 stays stopped".into()),
            actions: vec![AttentionAction::Retry],
            instance_id: Some("g8-2".into()),
        },
    });
    assert_eq!(
        reencode::<_, v1_0::ClientResponse>(&required),
        v1_0_event(v1_0::DaemonEvent::AttentionRequired {
            data: v1_0::AttentionRequiredData {
                reason: "g8-2 failed: restarted 3 times".into(),
                task_id: None,
                ask: None,
                recap: None,
            }
        })
    );
    let changed = event(DaemonEvent::InstanceChanged {
        data: InstanceChangedData {
            instance_id: "g8-2".into(),
            summary: "failed".into(),
            instance: Some(InstanceView {
                instance_id: "g8-2".into(),
                team_id: "general".into(),
                backend: "claude".into(),
                state: AgentState::Failed,
            }),
        },
    });
    assert_eq!(
        reencode::<_, v1_0::ClientResponse>(&changed),
        v1_0_event(v1_0::DaemonEvent::InstanceChanged {
            data: v1_0::InstanceChangedData {
                instance_id: "g8-2".into(),
                summary: "failed".into(),
            }
        })
    );
}

/// Gate 8 P3: 1.1 decodes every 1.0 message; the 1.1 fields are absent.
#[test]
fn a_1_1_peer_decodes_1_0_messages() {
    use agend_core::protocol::client::{InstanceChangedData, SubscribeEventsData};
    let hello: ClientRequest = reencode(&v1_0::ClientRequest::Hello {
        data: v1_0::Hello {
            supported: vec![v1_0::Version { major: 1, minor: 0 }],
        },
    });
    let ClientRequest::Hello { data } = hello else {
        panic!("{hello:?}");
    };
    assert_eq!(data.supported, [ProtocolVersion::new(1, 0)]);
    assert_eq!(data.caller, None, "a 1.0 hello is the operator's");
    let subscribe: ClientRequest = reencode(&v1_0::ClientRequest::SubscribeEvents {
        data: v1_0::SubscribeEventsData {
            after_event_id: None,
        },
    });
    assert_eq!(
        subscribe,
        ClientRequest::SubscribeEvents {
            data: SubscribeEventsData {
                after_event_id: None
            }
        }
    );
    let required: DaemonEvent = reencode(&v1_0::DaemonEvent::AttentionRequired {
        data: v1_0::AttentionRequiredData {
            reason: "usage limit".into(),
            task_id: Some("T-1".into()),
            ask: None,
            recap: None,
        },
    });
    let DaemonEvent::AttentionRequired { data } = required else {
        panic!("{required:?}");
    };
    assert_eq!(
        (
            data.attention_id,
            data.unblocks,
            data.waiting_since_unix_ms,
            data.if_ignored,
            data.actions,
            data.instance_id
        ),
        (None, None, None, None, vec![], None)
    );
    let changed: DaemonEvent = reencode(&v1_0::DaemonEvent::InstanceChanged {
        data: v1_0::InstanceChangedData {
            instance_id: "dev-1".into(),
            summary: "started".into(),
        },
    });
    assert_eq!(
        changed,
        DaemonEvent::InstanceChanged {
            data: InstanceChangedData {
                instance_id: "dev-1".into(),
                summary: "started".into(),
                instance: None,
            }
        }
    );
    let task: DaemonEvent = reencode(&v1_0::DaemonEvent::TaskChanged {
        data: v1_0::TaskChangedData {
            task_id: "T-1".into(),
            summary: "merged".into(),
        },
    });
    assert_eq!(
        task,
        DaemonEvent::TaskChanged {
            data: TaskChangedData {
                task_id: "T-1".into(),
                summary: "merged".into(),
                task: None,
            }
        }
    );
}
