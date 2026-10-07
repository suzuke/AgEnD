//! Six directed routes through one real daemon and native backend producers.
//! Claude uses the real channel helper and explicit ACK, not a fake store write.
use super::*;

#[test]
fn all_six_backend_routes_preserve_sender_unicode_and_single_receipts() {
    let mut f = Fixture::new(0);
    {
        let store = f.store();
        for (name, backend, binary) in [
            ("open", Backend::Opencode, "fake-opencode-cli"),
            ("codex", Backend::Codex, "fake-codex"),
        ] {
            let program = Path::new(BIN).parent().unwrap().join(binary);
            assert!(program.is_file(), "build agend-testkit binaries first");
            let cwd = f.home.join(name);
            fs::create_dir(&cwd).unwrap();
            block_on(store.add_instance(&Instance {
                id: name.into(),
                backend,
                program: program.display().to_string(),
                args: vec![],
                working_directory: cwd.display().to_string(),
                session_id: None,
                status: InstanceStatus::New,
                session_started: false,
                agent_pid: None,
                legacy_no_thread: false,
                delivery: "push".into(),
            }))
            .unwrap();
        }
    }
    f.start();
    f.hook("SessionStart", json!({"source":"startup"}));
    std::thread::sleep(Duration::from_millis(5100));
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty()
    );
    let mut channel = Channel::new(&f.home);
    let mut sent = Vec::new();
    for from in ["claude", "codex", "open"] {
        for to in ["claude", "codex", "open"] {
            if from == to {
                continue;
            }
            eprintln!("route {from} -> {to}");
            let message = id();
            let body = format!("{from} to {to}: 繁中 é\nsecond line");
            let home = f.home.clone();
            let request_id = message.clone();
            let content = body.clone();
            let sender = std::thread::spawn(move || {
                operator_request(
                    &home,
                    Some(from),
                    ClientRequest::Command {
                        data: ClientCommandData {
                            request_id: id(),
                            command: AgentCommand::Send {
                                to: to.into(),
                                message: content,
                                level: Some(MessageLevel::Queue),
                                message_id: Some(request_id),
                            },
                        },
                    },
                )
            });
            if to == "claude" {
                let notification = loop {
                    let v = channel.recv();
                    if v["method"] == "notifications/claude/channel" {
                        break v;
                    }
                };
                assert_eq!(
                    notification["params"]["content"],
                    format!("From: {from}\n\n{body}")
                );
                let receipt = Channel::receipt(&notification);
                assert_eq!(receipt.message_id, message);
                channel.written_barrier();
                assert_ne!(channel.ack(&[receipt])["result"]["isError"], true);
                assert_eq!(f.hook("Stop", json!({"stop_hook_active":false})), json!({}));
            }
            let response = sender.join().unwrap();
            assert!(
                matches!(response, ClientResponse::CommandResult { .. }),
                "{response:?}"
            );
            sent.push((message, from, to, body));
        }
    }
    // Worker reconciliation is bounded and asynchronous after POST acceptance.
    std::thread::sleep(Duration::from_secs(2));
    channel.close();
    f.stop();
    let store = f.store();
    for (message, from, to, body) in sent {
        let row = block_on(store.message(&message)).unwrap().unwrap();
        assert_eq!(row.state, DeliveryState::Confirmed, "{from} -> {to}");
        assert_eq!(
            (
                row.from_instance.as_str(),
                row.to_instance.as_str(),
                row.body
            ),
            (from, to, body)
        );
    }
    for to in ["claude", "codex", "open"] {
        assert_eq!(block_on(store.messages_to(to)).unwrap().len(), 2);
    }
}
