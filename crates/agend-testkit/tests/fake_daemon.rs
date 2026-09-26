//! The fake daemon speaks client protocol 1.1 with the real core types.

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::ask::{AnswerSource, AskEntry, AskReply, AskThread, ContextRecap};
use agend_core::protocol::client::error_code::{
    HELLO_REQUIRED, INVALID_REQUEST, UNKNOWN_REQUEST, VERSION_MISMATCH,
};
use agend_core::protocol::client::{
    AgentCommand, AnswerAskData, ClientCommandData, ClientRequest, ClientResponse, CommandResult,
    DaemonEvent, InstanceData, ResultIdentity, STALE_RESULT, SubscribeEventsData, V1_1,
};
use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};

fn connected(daemon: &FakeDaemon) -> ProbeClient {
    let mut client = ProbeClient::connect(daemon.socket_path()).unwrap();
    match client.request(&ClientRequest::hello()).unwrap() {
        ClientResponse::Hello { data } => assert_eq!(data.selected, V1_1),
        other => panic!("expected hello, got {other:?}"),
    }
    client
}

fn command(id: &str, command: AgentCommand) -> ClientRequest {
    ClientRequest::Command {
        data: ClientCommandData {
            request_id: id.into(),
            command,
        },
    }
}

fn error_code(response: ClientResponse) -> String {
    match response {
        ClientResponse::Error { data } => data.code,
        other => panic!("expected an error, got {other:?}"),
    }
}

fn identity(stage: &str, attempt: u32) -> Option<ResultIdentity> {
    Some(ResultIdentity {
        stage_id: stage.into(),
        attempt,
    })
}

#[test]
fn first_message_must_be_hello() {
    let daemon = FakeDaemon::start().unwrap();
    let mut client = ProbeClient::connect(daemon.socket_path()).unwrap();
    let reply = client
        .request(&command("r-1", AgentCommand::Status))
        .unwrap();
    assert_eq!(error_code(reply), HELLO_REQUIRED);
    assert!(client.recv().unwrap().is_none(), "connection must close");
}

#[test]
fn invalid_json_before_hello_gets_hello_required_and_close() {
    let daemon = FakeDaemon::start().unwrap();
    let mut client = ProbeClient::connect(daemon.socket_path()).unwrap();
    client.send_raw("not json").unwrap();
    assert_eq!(error_code(client.recv().unwrap().unwrap()), HELLO_REQUIRED);
    assert!(client.recv().unwrap().is_none(), "connection must close");
}

#[test]
fn invalid_json_after_hello_is_rejected_and_the_connection_stays_open() {
    let daemon = FakeDaemon::start().unwrap();
    let mut client = connected(&daemon);
    client.send_raw("not json").unwrap();
    assert_eq!(error_code(client.recv().unwrap().unwrap()), INVALID_REQUEST);
    let reply = client
        .request(&command("r-1", AgentCommand::Status))
        .unwrap();
    assert!(
        matches!(reply, ClientResponse::CommandResult { .. }),
        "{reply:?}"
    );
}

#[test]
fn incompatible_major_gets_a_clear_error_and_close() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.set_supported_versions(&[ProtocolVersion::new(2, 0)]);
    let mut client = ProbeClient::connect(daemon.socket_path()).unwrap();
    let reply = client.request(&ClientRequest::hello()).unwrap();
    let ClientResponse::Error { data } = reply else {
        panic!("expected an error, got {reply:?}");
    };
    assert_eq!(data.code, VERSION_MISMATCH);
    assert_eq!(
        data.message,
        "client protocol version mismatch: local supports 2.0, remote supports 1.1"
    );
    assert!(client.recv().unwrap().is_none());
}

#[test]
fn status_and_unknown_requests() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.set_status("2 agents, 1 needs you");
    let mut client = connected(&daemon);
    match client
        .request(&command("r-1", AgentCommand::Status))
        .unwrap()
    {
        ClientResponse::CommandResult { data } => {
            assert_eq!(data.request_id, "r-1");
            let CommandResult::Status { data } = data.result else {
                panic!("expected status");
            };
            assert_eq!(data.summary, "2 agents, 1 needs you");
        }
        other => panic!("expected a command result, got {other:?}"),
    }
    client
        .send_raw(r#"{"type":"from_the_future","data":{}}"#)
        .unwrap();
    assert_eq!(error_code(client.recv().unwrap().unwrap()), UNKNOWN_REQUEST);
}

#[test]
fn results_must_echo_the_current_stage_attempt() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.assign("T-1", identity("work", 2).unwrap());
    let mut client = connected(&daemon);
    let done = |id: &str, identity| {
        command(
            id,
            AgentCommand::Done {
                task_id: "T-1".into(),
                identity,
            },
        )
    };
    for (id, stale) in [
        ("r-1", None),
        ("r-2", identity("work", 1)),
        ("r-3", identity("review", 2)),
    ] {
        let reply = client.request(&done(id, stale)).unwrap();
        let ClientResponse::Error { data } = reply else {
            panic!("stale result accepted: {reply:?}");
        };
        assert_eq!(
            (data.code.as_str(), data.request_id.as_deref()),
            (STALE_RESULT, Some(id))
        );
    }
    assert_eq!(
        daemon.assignment("T-1"),
        identity("work", 2),
        "stale results change nothing"
    );
    let reply = client.request(&done("r-4", identity("work", 2))).unwrap();
    assert!(
        matches!(
            reply,
            ClientResponse::CommandResult { ref data } if data.result == CommandResult::Accepted
        ),
        "{reply:?}"
    );
    let replay = client.request(&done("r-5", identity("work", 2))).unwrap();
    assert_eq!(
        error_code(replay),
        STALE_RESULT,
        "a replayed result is stale"
    );
}

#[test]
fn events_replay_the_backlog_then_stream_live() {
    let daemon = FakeDaemon::start().unwrap();
    let mut agent = connected(&daemon);
    agent
        .request(&command(
            "r-1",
            AgentCommand::TaskCreate {
                title: "first".into(),
                role: "dev".into(),
                team_id: None,
                workflow_id: None,
            },
        ))
        .unwrap();
    let mut viewer = connected(&daemon);
    viewer
        .send(&ClientRequest::SubscribeEvents {
            data: SubscribeEventsData {
                after_event_id: None,
            },
        })
        .unwrap();
    let ClientResponse::Event { data: backlog } = viewer.recv().unwrap().unwrap() else {
        panic!("expected the backlog event");
    };
    // Event ids continue from the daemon's base: the first one is base + 1.
    assert_eq!(backlog.event_id, daemon.event_id_start() + 1);
    let reply = agent
        .request(&command(
            "r-2",
            AgentCommand::Ask {
                question: "sqlite or files?".into(),
                options: vec!["sqlite".into(), "files".into()],
            },
        ))
        .unwrap();
    let ClientResponse::CommandResult { data } = reply else {
        panic!("expected ask_created, got {reply:?}");
    };
    let CommandResult::AskCreated { data: created } = data.result else {
        panic!("expected ask_created");
    };
    let ClientResponse::Event { data: live } = viewer.recv().unwrap().unwrap() else {
        panic!("expected a live event");
    };
    assert_eq!(live.event_id, daemon.event_id_start() + 2);
    assert!(matches!(live.event, DaemonEvent::AttentionRequired { .. }));
    let answer = viewer
        .request(&ClientRequest::AnswerAsk {
            data: AnswerAskData {
                request_id: "r-3".into(),
                ask_id: created.ask_id,
                source: AnswerSource::Tui,
                reply: AskReply::Choice {
                    option: "sqlite".into(),
                },
            },
        })
        .unwrap();
    // The reply comes first, then the event it caused (as from the real
    // server, which writes a request's reply before reading further events).
    assert!(
        matches!(answer, ClientResponse::CommandResult { .. }),
        "{answer:?}"
    );
    let updated = viewer.recv().unwrap().unwrap();
    assert!(
        matches!(updated, ClientResponse::Event { data } if matches!(data.event, DaemonEvent::AskUpdated { .. }))
    );
    viewer
        .send(&ClientRequest::SubscribeTerminal {
            data: InstanceData {
                instance_id: "dev-1".into(),
            },
        })
        .unwrap();
    assert!(matches!(
        viewer.recv().unwrap().unwrap(),
        ClientResponse::TerminalSnapshot { .. }
    ));
    assert_eq!(daemon.requests().len(), 7);
}

#[test]
fn dropping_the_daemon_closes_open_connections() {
    let daemon = FakeDaemon::start().unwrap();
    let mut client = connected(&daemon);
    drop(daemon);
    let started = std::time::Instant::now();
    let outcome = client
        .send(&command("r-1", AgentCommand::Status))
        .and_then(|()| client.recv());
    let elapsed = started.elapsed();
    match outcome {
        Ok(None) => {}
        Err(e)
            if e.kind() != std::io::ErrorKind::WouldBlock
                && e.kind() != std::io::ErrorKind::TimedOut => {}
        other => panic!("expected EOF or a connection error after drop, got {other:?}"),
    }
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "closing took {elapsed:?}"
    );
}

#[test]
fn open_ask_carries_task_and_recap_and_is_answerable() {
    let daemon = FakeDaemon::start().unwrap();
    let recap = ContextRecap {
        goal: "g".into(),
        decisions: vec![],
        asking: "a".into(),
        next: "n".into(),
    };
    let thread = AskThread {
        ask_id: "A-9".into(),
        task_id: Some("T-9".into()),
        entries: vec![AskEntry::Question {
            from: "dev-9".into(),
            text: "which?".into(),
            options: vec!["x".into()],
        }],
    };
    assert_eq!(
        daemon.open_ask(thread, Some(recap.clone())),
        daemon.event_id_start() + 1
    );
    let mut viewer = connected(&daemon);
    viewer
        .send(&ClientRequest::SubscribeEvents {
            data: SubscribeEventsData {
                after_event_id: None,
            },
        })
        .unwrap();
    let Some(ClientResponse::Event { data }) = viewer.recv().unwrap() else {
        panic!("expected the backlog event");
    };
    let DaemonEvent::AttentionRequired { data } = data.event else {
        panic!("expected attention_required");
    };
    assert_eq!(data.task_id.as_deref(), Some("T-9"));
    assert_eq!(data.recap, Some(recap));
    viewer
        .send(&ClientRequest::AnswerAsk {
            data: AnswerAskData {
                request_id: "r-1".into(),
                ask_id: "A-9".into(),
                source: AnswerSource::Tui,
                reply: AskReply::Choice { option: "x".into() },
            },
        })
        .unwrap();
    let mut accepted = false;
    for _ in 0..2 {
        if let Some(ClientResponse::CommandResult { data }) = viewer.recv().unwrap() {
            accepted = data.request_id == "r-1" && data.result == CommandResult::Accepted;
        }
    }
    assert!(accepted, "open_ask threads accept answer_ask");
}
