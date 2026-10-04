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
        json!({"type": "hello", "data": {"supported": [{"major": 1, "minor": 5}, {"major": 1, "minor": 4}, {"major": 1, "minor": 3}]}})
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
                supported: vec![
                    v1_0::Version { major: 1, minor: 5 },
                    v1_0::Version { major: 1, minor: 4 },
                    v1_0::Version { major: 1, minor: 3 }
                ]
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
            note: None,
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
                working_directory: None,
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

/// Client protocol 1.1 as it shipped (gate 8), frozen: only the parts 1.2
/// touches (the rest did not change).
mod v1_1 {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Version {
        pub major: u16,
        pub minor: u16,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct SelectedVersionData {
        pub selected: Version,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum ClientRequest {
        Command {
            data: CommandData,
        },
        #[serde(other)]
        Unknown,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CommandData {
        pub request_id: String,
        pub command: AgentCommand,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "command", rename_all = "snake_case")]
    pub enum AgentCommand {
        Status,
        Send {
            to: String,
            message: String,
        },
        #[serde(other)]
        Unknown,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum ClientResponse {
        Hello {
            data: SelectedVersionData,
        },
        CommandResult {
            data: CommandResultData,
        },
        #[serde(other)]
        Unknown,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CommandResultData {
        pub request_id: String,
        pub result: CommandResult,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "result", rename_all = "snake_case")]
    pub enum CommandResult {
        Accepted,
        Status {
            data: StatusData,
        },
        #[serde(other)]
        Unknown,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct StatusData {
        pub task_id: Option<String>,
        pub instance_id: Option<String>,
        pub summary: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    pub struct InstanceView {
        pub instance_id: String,
        pub team_id: String,
        pub backend: String,
        pub state: String,
    }
}

/// Gate 9 P6: a 1.1 peer decodes every 1.2 message: `operator` and the new
/// results are `unknown`, the new fields are ignored.
#[test]
fn a_1_1_peer_decodes_1_2_messages() {
    use agend_core::protocol::client::{
        AgentState, InstanceAddedData, InstanceView, MessageLevel, OperatorCommand, OperatorData,
        RestartingData, SelectedVersionData, StatusData, V1_1,
    };
    let hello = ClientResponse::Hello {
        data: SelectedVersionData {
            selected: V1_1,
            daemon_version: Some("agend 0.0.0".into()),
            daemon_pid: Some(5101),
            boot_id: Some(1_790_000_000_000_000),
        },
    };
    assert_eq!(
        reencode::<_, v1_1::ClientResponse>(&hello),
        v1_1::ClientResponse::Hello {
            data: v1_1::SelectedVersionData {
                selected: v1_1::Version { major: 1, minor: 1 }
            }
        }
    );
    for command in [
        OperatorCommand::InstanceAdd {
            instance_id: "g9-1".into(),
            backend: "claude".into(),
            working_directory: None,
            program: Some("/bin/sh".into()),
            args: vec!["-c".into(), "sleep 60".into()],
        },
        OperatorCommand::InstanceRemove {
            instance_id: "g9-1".into(),
        },
        OperatorCommand::DaemonRestart { binary: None },
        OperatorCommand::TaskCancel {
            task_id: "t-1".into(),
            reason: None,
        },
    ] {
        let request = ClientRequest::Operator {
            data: OperatorData {
                request_id: "r-1".into(),
                command,
            },
        };
        assert_eq!(
            reencode::<_, v1_1::ClientRequest>(&request),
            v1_1::ClientRequest::Unknown
        );
    }
    let send = ClientRequest::Command {
        data: ClientCommandData {
            request_id: "r-2".into(),
            command: AgentCommand::Send {
                to: "g9-2".into(),
                message: "hi".into(),
                level: Some(MessageLevel::Steer),
                message_id: Some("5d0f7c2e-1b7a-4c3e-9f00-0123456789ab".into()),
            },
        },
    };
    assert_eq!(
        reencode::<_, v1_1::ClientRequest>(&send),
        v1_1::ClientRequest::Command {
            data: v1_1::CommandData {
                request_id: "r-2".into(),
                command: v1_1::AgentCommand::Send {
                    to: "g9-2".into(),
                    message: "hi".into()
                }
            }
        }
    );
    let result = |result| ClientResponse::CommandResult {
        data: ClientCommandResultData {
            request_id: "r-3".into(),
            result,
        },
    };
    let v1_1_result = |result| v1_1::ClientResponse::CommandResult {
        data: v1_1::CommandResultData {
            request_id: "r-3".into(),
            result,
        },
    };
    for new in [
        CommandResult::InstanceAdded {
            data: InstanceAddedData {
                instance_id: "g9-1".into(),
                session_id: None,
                working_directory: "/tmp/w".into(),
            },
        },
        CommandResult::Restarting {
            data: RestartingData {
                preflight: vec!["agend 0.0.0".into()],
            },
        },
    ] {
        assert_eq!(
            reencode::<_, v1_1::ClientResponse>(&result(new)),
            v1_1_result(v1_1::CommandResult::Unknown)
        );
    }
    let status = CommandResult::Status {
        data: StatusData {
            task_id: Some("t-42".into()),
            instance_id: Some("g9-1".into()),
            summary: "review".into(),
            identity: Some(ResultIdentity {
                stage_id: "review".into(),
                attempt: 2,
            }),
        },
    };
    assert_eq!(
        reencode::<_, v1_1::ClientResponse>(&result(status)),
        v1_1_result(v1_1::CommandResult::Status {
            data: v1_1::StatusData {
                task_id: Some("t-42".into()),
                instance_id: Some("g9-1".into()),
                summary: "review".into(),
            }
        })
    );
    let view = InstanceView {
        instance_id: "g9-1".into(),
        team_id: "general".into(),
        backend: "codex".into(),
        state: AgentState::Idle,
        working_directory: Some("/tmp/w".into()),
    };
    assert_eq!(
        reencode::<_, v1_1::InstanceView>(&view),
        v1_1::InstanceView {
            instance_id: "g9-1".into(),
            team_id: "general".into(),
            backend: "codex".into(),
            state: "idle".into(),
        }
    );
}

/// Gate 9 P6: 1.2 decodes every 1.1 message; the 1.2 fields are absent.
#[test]
fn a_1_2_peer_decodes_1_1_messages() {
    use agend_core::protocol::client::{InstanceView, SelectedVersionData, V1_1};
    let hello: ClientResponse = reencode(&v1_1::ClientResponse::Hello {
        data: v1_1::SelectedVersionData {
            selected: v1_1::Version { major: 1, minor: 1 },
        },
    });
    assert_eq!(
        hello,
        ClientResponse::Hello {
            data: SelectedVersionData::new(V1_1)
        }
    );
    let send: ClientRequest = reencode(&v1_1::ClientRequest::Command {
        data: v1_1::CommandData {
            request_id: "r-1".into(),
            command: v1_1::AgentCommand::Send {
                to: "g9-2".into(),
                message: "hi".into(),
            },
        },
    });
    let ClientRequest::Command { data } = send else {
        panic!("{send:?}");
    };
    assert_eq!(
        data.command,
        AgentCommand::Send {
            to: "g9-2".into(),
            message: "hi".into(),
            level: None,
            message_id: None
        }
    );
    let view: InstanceView = reencode(&v1_1::InstanceView {
        instance_id: "g9-1".into(),
        team_id: "general".into(),
        backend: "claude".into(),
        state: "unknown".into(),
    });
    assert_eq!(view.working_directory, None);
}

#[test]
fn gate_10_requests_have_stable_additive_wire_shapes() {
    use agend_core::protocol::client::{
        AttentionAction, OperatorCommand, OperatorData, ResolveAttentionData,
    };
    let workflow =
        toml::to_string(&agend_core::pipeline::workflow::Workflow::builtin_research()).unwrap();
    let commands = vec![
        OperatorCommand::TeamAdd {
            team_id: "web".into(),
            repo: Some("/repo".into()),
            workflow_id: Some("research".into()),
        },
        OperatorCommand::TeamList,
        OperatorCommand::TeamJoin {
            team_id: "web".into(),
            instance_id: "dev-1".into(),
            role: "researcher".into(),
        },
        OperatorCommand::TeamSetWorkflow {
            team_id: "web".into(),
            workflow_id: "research".into(),
        },
        OperatorCommand::WorkflowList,
        OperatorCommand::WorkflowShow {
            workflow_id: "research".into(),
        },
        OperatorCommand::WorkflowCheck {
            toml: workflow.clone(),
        },
        OperatorCommand::WorkflowApply { toml: workflow },
        OperatorCommand::TaskCreate {
            title: "Research".into(),
            role: "researcher".into(),
            team_id: "web".into(),
            workflow_id: None,
        },
        OperatorCommand::TaskCancel {
            task_id: "t-1".into(),
            reason: Some("scope changed".into()),
        },
    ];
    let mut requests = commands
        .into_iter()
        .map(|command| ClientRequest::Operator {
            data: OperatorData {
                request_id: "g10".into(),
                command,
            },
        })
        .collect::<Vec<_>>();
    requests.push(ClientRequest::ResolveAttention {
        data: ResolveAttentionData {
            request_id: "changes".into(),
            attention_id: "approval:t-1/approve/1".into(),
            action: AttentionAction::RequestChanges,
            note: Some("Revise the result".into()),
        },
    });
    let text = serde_json::to_string_pretty(&requests).unwrap() + "\n";
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/pipeline-protocol-1.3.json");
    if std::env::var_os("AGEND_BLESS_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    assert_eq!(
        serde_json::from_str::<Vec<ClientRequest>>(&text).unwrap(),
        requests
    );
    for request in &requests[..9] {
        let old: v1_0::ClientRequest =
            serde_json::from_value(serde_json::to_value(request).unwrap()).unwrap();
        assert_eq!(old, v1_0::ClientRequest::Unknown);
    }
}

#[test]
fn full_terminal_requests_are_additive_and_acquire_cannot_choose_an_attach_id() {
    use agend_core::protocol::client::*;
    use agend_core::protocol::terminal::{TerminalSize, TerminalViewport};
    let requests = [
        ClientRequest::SubscribeTerminalFrames {
            data: TerminalSubscribeData {
                request_id: "s-1".into(),
                instance_id: "i-1".into(),
                viewport: TerminalViewport {
                    top: None,
                    rows: 24,
                },
            },
        },
        ClientRequest::SetTerminalViewport {
            data: TerminalViewportData {
                request_id: "v-2".into(),
                instance_id: "i-1".into(),
                view_id: "view-1".into(),
                generation: "holder-1".into(),
                viewport: TerminalViewport {
                    top: Some(100),
                    rows: 24,
                },
            },
        },
        ClientRequest::TerminalControl {
            data: ClientTerminalControlData {
                request_id: "a-3".into(),
                instance_id: "i-1".into(),
                view_id: "view-1".into(),
                generation: "holder-1".into(),
                operation: ClientTerminalOperation::Acquire {
                    size: TerminalSize {
                        rows: 24,
                        columns: 80,
                    },
                },
            },
        },
    ];
    for request in &requests {
        let line = serde_json::to_string(request).unwrap();
        assert_eq!(
            serde_json::from_str::<ClientRequest>(&line).unwrap(),
            *request
        );
        assert_eq!(
            serde_json::from_str::<v1_0::ClientRequest>(&line).unwrap(),
            v1_0::ClientRequest::Unknown
        );
        assert_eq!(
            serde_json::from_str::<v1_1::ClientRequest>(&line).unwrap(),
            v1_1::ClientRequest::Unknown
        );
    }
    assert_eq!(
        serde_json::to_value(&requests[2]).unwrap(),
        json!({
            "type": "terminal_control", "data": {
                "request_id": "a-3", "instance_id": "i-1", "view_id": "view-1", "generation": "holder-1",
                "operation": {"operation": "acquire", "size": {"rows": 24, "columns": 80}}
            }
        })
    );
    for response in [
        ClientResponse::TerminalControlAck {
            data: ClientTerminalControlAck {
                request_id: "a-3".into(),
                instance_id: "i-1".into(),
                view_id: "view-1".into(),
                generation: "holder-1".into(),
                control: TerminalControlState::ReadOnly,
                frame: None,
            },
        },
        ClientResponse::TerminalControlChanged {
            data: TerminalControlChangedData {
                instance_id: "i-1".into(),
                view_id: "view-1".into(),
                generation: "holder-1".into(),
                control: TerminalControlState::ReadOnly,
                reason: "connection closed".into(),
            },
        },
    ] {
        let line = serde_json::to_string(&response).unwrap();
        assert_eq!(
            serde_json::from_str::<ClientResponse>(&line).unwrap(),
            response
        );
        assert_eq!(
            serde_json::from_str::<v1_0::ClientResponse>(&line).unwrap(),
            v1_0::ClientResponse::Unknown
        );
        assert_eq!(
            serde_json::from_str::<v1_1::ClientResponse>(&line).unwrap(),
            v1_1::ClientResponse::Unknown
        );
    }
}

#[test]
fn claude_1_5_envelopes_are_additive_and_receipts_keep_native_attribution() {
    use agend_core::protocol::client::*;
    let receipt = ClaudeReceipt {
        message_id: "11111111-1111-4111-8111-111111111111".into(),
        delivery_id: "22222222-2222-4222-8222-222222222222".into(),
        session_id: "33333333-3333-4333-8333-333333333333".into(),
    };
    let request = ClientRequest::Claude {
        data: ClaudeRequestData {
            request_id: "r-1".into(),
            instance_id: "claude".into(),
            operation: ClaudeOperation::Ack {
                receipts: vec![receipt.clone()],
            },
        },
    };
    let line = serde_json::to_string(&request).unwrap();
    assert_eq!(
        serde_json::from_str::<ClientRequest>(&line).unwrap(),
        request
    );
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"type":"claude","data":{
            "request_id":"r-1", "instance_id":"claude", "operation":{"operation":"ack", "receipts":[{
                "message_id":receipt.message_id, "delivery_id":receipt.delivery_id, "session_id":receipt.session_id
            }]}
        }})
    );
    assert_eq!(
        serde_json::from_str::<v1_0::ClientRequest>(&line).unwrap(),
        v1_0::ClientRequest::Unknown
    );
    assert_eq!(
        serde_json::from_str::<v1_1::ClientRequest>(&line).unwrap(),
        v1_1::ClientRequest::Unknown
    );
    let response = ClientResponse::Claude {
        data: ClaudeReplyData {
            request_id: "r-1".into(),
            session_id: Some(receipt.session_id.clone()),
            messages: vec![ClaudePush {
                receipt,
                content: "From: operator\n\n完整 é".into(),
            }],
            committed: true,
        },
    };
    let line = serde_json::to_string(&response).unwrap();
    assert_eq!(
        serde_json::from_str::<ClientResponse>(&line).unwrap(),
        response
    );
    assert_eq!(
        serde_json::from_str::<v1_0::ClientResponse>(&line).unwrap(),
        v1_0::ClientResponse::Unknown
    );
    assert_eq!(
        serde_json::from_str::<v1_1::ClientResponse>(&line).unwrap(),
        v1_1::ClientResponse::Unknown
    );
}
