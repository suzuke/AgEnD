//! The fake daemon speaks client protocol 1.2 with the real core types.

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::ask::{AnswerSource, AskEntry, AskReply, AskThread, ContextRecap};
use agend_core::protocol::client::error_code::{
    HELLO_REQUIRED, INVALID_REQUEST, UNKNOWN_REQUEST, VERSION_MISMATCH,
};
use agend_core::protocol::client::{
    AgentCommand, AgentState, AnswerAskData, AttentionAction, AttentionRequiredData,
    ClientCommandData, ClientRequest, ClientResponse, CommandResult, DaemonEvent, InstanceData,
    InstanceView, OperatorCommand, OperatorData, ResolveAttentionData, ResultIdentity,
    STALE_RESULT, SubscribeEventsData, V1_3,
};
use agend_testkit::contract::client::terminal_input;
use agend_testkit::fake_daemon::{CODEX_INPUT, FakeDaemon, ProbeClient, TYPE_OPERATOR_ONLY};
use std::time::Duration;

/// An agent's connection (agent commands are for agents, gate 9).
fn connected(daemon: &FakeDaemon) -> ProbeClient {
    let mut client = ProbeClient::connect(daemon.socket_path()).unwrap();
    match client
        .request(&ClientRequest::hello_as(Some("fd-agent".into())))
        .unwrap()
    {
        ClientResponse::Hello { data } => assert_eq!(data.selected, V1_3),
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
    daemon.set_status("idle");
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
        "client protocol version mismatch: local supports 2.0, remote supports 1.3, 1.4"
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
    // dev-1 is not in the fleet view: no terminal (gate 11 B P1).
    viewer
        .send(&ClientRequest::SubscribeTerminal {
            data: InstanceData {
                instance_id: "dev-1".into(),
            },
        })
        .unwrap();
    assert_eq!(error_code(viewer.recv().unwrap().unwrap()), "no_terminal");
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

/// Gate 9: `hello` names the daemon; agent commands are for agents and
/// operator requests for the operator (the real daemon's words).
#[test]
fn hello_names_the_daemon_and_permissions_go_both_ways() {
    let daemon = FakeDaemon::start().unwrap();
    let mut operator = ProbeClient::connect(daemon.socket_path()).unwrap();
    let ClientResponse::Hello { data } = operator.request(&ClientRequest::hello()).unwrap() else {
        panic!("no hello");
    };
    assert_eq!(data.daemon_pid, Some(std::process::id()));
    assert_eq!(data.boot_id, Some(daemon.event_id_start()));
    assert!(data.daemon_version.unwrap().starts_with("agend "));
    let ClientResponse::Error { data } = operator
        .request(&command("r-1", AgentCommand::Status))
        .unwrap()
    else {
        panic!("the operator's status was not refused");
    };
    assert_eq!(
        (data.code.as_str(), data.message.as_str()),
        (
            "forbidden",
            "agend status is an agent command; it runs inside an agent, where AGEND_INSTANCE is set"
        )
    );
    let mut agent = connected(&daemon);
    let add = ClientRequest::Operator {
        data: OperatorData {
            request_id: "r-2".into(),
            command: OperatorCommand::InstanceRemove {
                instance_id: "x".into(),
            },
        },
    };
    assert_eq!(error_code(agent.request(&add).unwrap()), "forbidden");
}

/// Gate 9: `daemon_restart` answers `restarting`, closes every connection
/// and starts a new boot id; a binary that is not agend is refused.
#[test]
fn a_restart_closes_connections_and_changes_the_boot_id() {
    let daemon = FakeDaemon::start().unwrap();
    let before = daemon.event_id_start();
    let mut operator = ProbeClient::hello(daemon.socket_path(), None).unwrap().0;
    let restart = |binary: Option<&str>| ClientRequest::Operator {
        data: OperatorData {
            request_id: "r-1".into(),
            command: OperatorCommand::DaemonRestart {
                binary: binary.map(Into::into),
            },
        },
    };
    let refused = operator.request(&restart(Some("/usr/bin/false"))).unwrap();
    let ClientResponse::Error { data } = refused else {
        panic!("{refused:?}");
    };
    assert_eq!(data.code, "preflight_failed");
    assert!(
        data.message.starts_with(
            "/usr/bin/false daemon preflight exited with status 1; the daemon keeps running agend "
        ),
        "{}",
        data.message
    );
    let reply = operator.request(&restart(None)).unwrap();
    assert!(
        matches!(&reply, ClientResponse::CommandResult { data }
            if matches!(data.result, CommandResult::Restarting { .. })),
        "{reply:?}"
    );
    assert!(
        operator.recv().unwrap().is_none(),
        "the connection must close"
    );
    assert!(daemon.event_id_start() > before, "a new boot id");
}

fn instance(id: &str, backend: &str) -> InstanceView {
    InstanceView {
        instance_id: id.into(),
        team_id: "general".into(),
        backend: backend.into(),
        state: AgentState::Unknown,
        working_directory: None,
    }
}

fn operator(daemon: &FakeDaemon) -> ProbeClient {
    ProbeClient::hello(daemon.socket_path(), None).unwrap().0
}

fn subscribe_terminal(id: &str) -> ClientRequest {
    ClientRequest::SubscribeTerminal {
        data: InstanceData {
            instance_id: id.into(),
        },
    }
}

#[test]
fn each_instance_has_its_screen_and_bytes_update_it() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.set_instance(instance("g-1", "claude"));
    daemon.set_instance(instance("g-2", "claude"));
    daemon.set_screen("g-1", "$ ls");
    let mut c = operator(&daemon);
    let reply = c.request(&subscribe_terminal("g-2")).unwrap();
    assert!(
        matches!(&reply, ClientResponse::TerminalSnapshot { data } if data.screen == "fake screen of g-2"),
        "{reply:?}"
    );
    // Replaced by g-1: g-2's bytes no longer come.
    let reply = c.request(&subscribe_terminal("g-1")).unwrap();
    assert!(
        matches!(&reply, ClientResponse::TerminalSnapshot { data } if data.screen == "$ ls"),
        "{reply:?}"
    );
    daemon.push_terminal_bytes("g-2", b"two\r\n");
    daemon.push_terminal_bytes("g-1", b"\r\nREADME\r\n");
    let bytes = c.recv().unwrap().unwrap();
    assert!(
        matches!(&bytes, ClientResponse::TerminalBytes { data }
            if data.instance_id == "g-1" && data.bytes_base64 == "DQpSRUFETUUNCg=="),
        "{bytes:?}"
    );
    let reply = c.request(&subscribe_terminal("g-1")).unwrap();
    assert!(
        matches!(&reply, ClientResponse::TerminalSnapshot { data } if data.screen == "$ ls\nREADME\n"),
        "{reply:?}"
    );
    assert_eq!(daemon.terminal_subscribers(), 1);
    drop(c);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while daemon.terminal_subscribers() > 0 {
        assert!(std::time::Instant::now() < deadline, "subscription kept");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn terminal_input_checks_identity_then_instance_then_backend() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.set_instance(instance("g-1", "claude"));
    daemon.set_instance(instance("g-x", "codex"));
    let mut agent = connected(&daemon);
    for id in ["g-1", "nobody"] {
        agent.send(&terminal_input(id, b"a")).unwrap();
        let ClientResponse::Error { data } = agent.recv().unwrap().unwrap() else {
            panic!("expected an error");
        };
        assert_eq!(
            (data.code.as_str(), data.message.as_str(), data.request_id),
            ("forbidden", TYPE_OPERATOR_ONLY, None)
        );
    }
    let mut op = operator(&daemon);
    op.send(&terminal_input("nobody", b"a")).unwrap();
    assert_eq!(error_code(op.recv().unwrap().unwrap()), "no_terminal");
    op.send(&terminal_input("g-x", b"a")).unwrap();
    let ClientResponse::Error { data } = op.recv().unwrap().unwrap() else {
        panic!("expected an error");
    };
    assert_eq!(
        (data.code.as_str(), data.message.as_str()),
        ("not_supported", CODEX_INPUT)
    );
    op.send(&terminal_input("g-1", "é\r\x1b[A".as_bytes()))
        .unwrap();
    let quiet = op.recv_within(Duration::from_millis(300));
    assert!(quiet.is_err(), "no reply to accepted input: {quiet:?}");
    assert_eq!(
        daemon.terminal_inputs(),
        vec![("g-1".to_owned(), "é\r\x1b[A".as_bytes().to_vec())]
    );
}

#[test]
fn held_resolved_events_wait_for_release() {
    let daemon = FakeDaemon::start().unwrap();
    daemon.add_attention(AttentionRequiredData {
        reason: "g-1 failed".into(),
        task_id: None,
        ask: None,
        recap: None,
        attention_id: Some("instance-failed:g-1".into()),
        unblocks: Some(0),
        waiting_since_unix_ms: Some(1),
        if_ignored: None,
        actions: vec![AttentionAction::Retry],
        instance_id: Some("g-1".into()),
    });
    daemon.hold_resolved_events(true);
    let mut op = operator(&daemon);
    let as_of = daemon.fleet().as_of_event_id;
    op.send(&ClientRequest::SubscribeEvents {
        data: SubscribeEventsData {
            after_event_id: Some(as_of),
        },
    })
    .unwrap();
    let reply = op
        .request(&ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: "r-1".into(),
                attention_id: "instance-failed:g-1".into(),
                action: AttentionAction::Retry,
                note: None,
            },
        })
        .unwrap();
    assert!(
        matches!(reply, ClientResponse::CommandResult { .. }),
        "{reply:?}"
    );
    assert!(daemon.fleet().attention.is_empty());
    let held = op.recv_within(Duration::from_millis(300));
    assert!(held.is_err(), "the event is held: {held:?}");
    daemon.release_resolved_events();
    let released = op.recv().unwrap().unwrap();
    assert!(
        matches!(&released, ClientResponse::Event { data }
            if matches!(data.event, DaemonEvent::AttentionResolved { .. })),
        "{released:?}"
    );
}
